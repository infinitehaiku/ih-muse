//! Periodically measure one host or pod and publish canonical graph observations.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use ih_muse::graph_muse::{
    GraphMuse, GraphMuseConfig, KubernetesContext, RedisInfoAdapter, RustvelloRunnerContext,
};
use ih_muse::secret_file::read_secret_file;
use ih_muse_client::GraphPoetClient;
use ih_muse_core::MuseError;
use uuid::Uuid;

const MAX_DELIVERY_BACKOFF: Duration = Duration::from_secs(5);

fn delivery_backoff(consecutive_failures: u32) -> Duration {
    Duration::from_millis(100_u64.saturating_mul(1_u64 << consecutive_failures.min(5)))
        .min(MAX_DELIVERY_BACKOFF)
}

fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is required"))
}

fn bounded_usize(name: &str, default: usize, minimum: usize, maximum: usize) -> usize {
    let value = std::env::var(name)
        .ok()
        .map(|raw| {
            raw.parse::<usize>()
                .unwrap_or_else(|_| panic!("{name} must be an integer"))
        })
        .unwrap_or(default);
    assert!(
        (minimum..=maximum).contains(&value),
        "{name} is outside its supported range"
    );
    value
}

fn process_ids(maximum: usize) -> Vec<u32> {
    let value = std::env::var("IH_GRAPH_PROCESS_IDS").unwrap_or_default();
    let result = value
        .split(',')
        .filter(|part| !part.trim().is_empty())
        .map(|part| {
            part.trim()
                .parse::<u32>()
                .expect("IH_GRAPH_PROCESS_IDS contains an invalid PID")
        })
        .collect::<Vec<_>>();
    assert!(
        result.len() <= maximum,
        "IH_GRAPH_PROCESS_IDS exceeds the process budget"
    );
    result
}

fn rustvello_runner() -> Option<RustvelloRunnerContext> {
    let application_id = std::env::var("IH_GRAPH_RUSTVELLO_APPLICATION_ID").ok();
    let runner_id = std::env::var("IH_GRAPH_RUSTVELLO_RUNNER_ID").ok();
    let pid = std::env::var("IH_GRAPH_RUSTVELLO_RUNNER_PID").ok();
    match (application_id, runner_id, pid) {
        (None, None, None) => None,
        (Some(application_id), Some(runner_id), Some(pid)) => Some(RustvelloRunnerContext {
            application_id,
            runner_id,
            pid: pid
                .parse()
                .expect("IH_GRAPH_RUSTVELLO_RUNNER_PID is invalid"),
        }),
        _ => panic!("all IH_GRAPH_RUSTVELLO_RUNNER_* values must be set together"),
    }
}

/// Read a credential file, panicking with a secret-free reason when it is refused.
fn private_token(path: &str, label: &str) -> String {
    read_secret_file(path, label).unwrap_or_else(|error| panic!("{error}"))
}

fn bounded_file(path: &str, label: &str) -> Vec<u8> {
    let metadata = fs::metadata(path).unwrap_or_else(|_| panic!("{label} file must exist"));
    assert!(
        metadata.is_file() && metadata.len() > 0 && metadata.len() <= 1024 * 1024,
        "{label} file exceeds its bounds"
    );
    fs::read(path).unwrap_or_else(|_| panic!("{label} file could not be read"))
}

struct RedisProbe {
    address: SocketAddr,
    password: String,
    instance_id: String,
    process_pid: u32,
}

impl RedisProbe {
    fn from_environment() -> Option<Self> {
        let address = std::env::var("IH_GRAPH_REDIS_ADDRESS").ok();
        let password_file = std::env::var("IH_GRAPH_REDIS_PASSWORD_FILE").ok();
        let instance_id = std::env::var("IH_GRAPH_REDIS_INSTANCE_ID").ok();
        let process_pid = std::env::var("IH_GRAPH_REDIS_PROCESS_PID").ok();
        match (address, password_file, instance_id, process_pid) {
            (None, None, None, None) => None,
            (Some(address), Some(password_file), Some(instance_id), Some(process_pid)) => {
                let address: SocketAddr =
                    address.parse().expect("IH_GRAPH_REDIS_ADDRESS is invalid");
                assert!(
                    address.ip().is_loopback(),
                    "Redis INFO collection is restricted to the colocated loopback service"
                );
                Some(Self {
                    address,
                    password: private_token(&password_file, "Redis password"),
                    instance_id,
                    process_pid: process_pid
                        .parse()
                        .expect("IH_GRAPH_REDIS_PROCESS_PID is invalid"),
                })
            }
            _ => panic!("all IH_GRAPH_REDIS_* values must be set together"),
        }
    }

    fn info(&self) -> Vec<u8> {
        let mut stream = TcpStream::connect_timeout(&self.address, Duration::from_secs(2))
            .expect("Redis INFO connection failed");
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("Redis read timeout could not be set");
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .expect("Redis write timeout could not be set");
        write!(
            stream,
            "*2\r\n$4\r\nAUTH\r\n${}\r\n{}\r\n*2\r\n$4\r\nINFO\r\n$3\r\nALL\r\n",
            self.password.len(),
            self.password
        )
        .expect("Redis INFO request failed");
        stream.flush().expect("Redis INFO request flush failed");
        let mut reader = BufReader::new(stream);
        let mut auth = String::new();
        reader
            .read_line(&mut auth)
            .expect("Redis AUTH response failed");
        assert_eq!(auth, "+OK\r\n", "Redis AUTH was rejected");
        let mut header = String::new();
        reader
            .read_line(&mut header)
            .expect("Redis INFO header failed");
        let length = header
            .strip_prefix('$')
            .and_then(|value| value.trim_end().parse::<usize>().ok())
            .expect("Redis INFO response is not a bulk string");
        assert!(length <= 1024 * 1024, "Redis INFO response exceeds 1 MiB");
        let mut response = vec![0; length];
        reader
            .read_exact(&mut response)
            .expect("Redis INFO response was truncated");
        response
    }
}

#[tokio::main]
async fn main() {
    let max_processes = bounded_usize("IH_GRAPH_MAX_PROCESSES", 32, 1, 256);
    let interval_ms = bounded_usize("IH_GRAPH_INTERVAL_MS", 1_000, 100, 300_000) as u64;
    let max_samples = bounded_usize("IH_GRAPH_MAX_SAMPLES", 0, 0, 31_536_000);
    let print_batches = std::env::var("IH_GRAPH_PRINT_BATCH").ok().as_deref() == Some("true");
    let pids = process_ids(max_processes);
    let redis = RedisProbe::from_environment();
    let config = GraphMuseConfig {
        organization: required("IH_GRAPH_ORGANIZATION"),
        owner_id: required("IH_GRAPH_OWNER"),
        source_id: required("IH_GRAPH_SOURCE"),
        environment_id: required("IH_GRAPH_ENVIRONMENT"),
        host_id: required("IH_GRAPH_HOST"),
        kubernetes: KubernetesContext::from_environment(),
        rustvello_runner: rustvello_runner(),
        max_processes,
    };
    let mut muse = GraphMuse::new(config).expect("invalid graph Muse configuration");
    let client = std::env::var("IH_GRAPH_POET_ENDPOINT")
        .ok()
        .map(|endpoint| {
            let token = private_token(&required("IH_GRAPH_POET_TOKEN_FILE"), "graph Poet token");
            match std::env::var("IH_GRAPH_POET_CA_FILE") {
                Ok(path) => GraphPoetClient::private_tls(
                    endpoint,
                    token,
                    &bounded_file(&path, "graph owner CA"),
                ),
                Err(_) => GraphPoetClient::new(endpoint, token),
            }
            .expect("invalid Poet endpoint")
        });

    let mut samples = 0usize;
    let mut delivery_failures = 0u64;
    'collect: loop {
        let mut batch = muse.collect(&pids).expect("native graph collection failed");
        if let Some(redis) = &redis {
            let snapshot =
                RedisInfoAdapter::parse(&redis.info()).expect("Redis INFO response is invalid");
            muse.append_database_snapshot_at_process(
                &mut batch,
                "redis",
                &redis.instance_id,
                &snapshot,
                Some(redis.process_pid),
            )
            .expect("Redis observations are invalid");
        }
        let request = muse
            .intake_request(Uuid::new_v4().to_string(), batch)
            .expect("native graph batch is invalid");
        if let Some(client) = &client {
            let mut consecutive_failures = 0u32;
            loop {
                match client.publish(&request).await {
                    Ok(_) => break,
                    Err(
                        error @ (MuseError::Unavailable(_)
                        | MuseError::Backpressure(_)
                        | MuseError::Client(_)),
                    ) => {
                        consecutive_failures = consecutive_failures.saturating_add(1);
                        delivery_failures = delivery_failures.saturating_add(1);
                        let delay = delivery_backoff(consecutive_failures);
                        eprintln!(
                            "graph delivery remains pending after transient failure #{consecutive_failures}; retrying in {} ms: {error}",
                            delay.as_millis()
                        );
                        tokio::select! {
                            result = tokio::signal::ctrl_c() => {
                                result.expect("failed to install graph Muse shutdown signal");
                                break 'collect;
                            }
                            () = tokio::time::sleep(delay) => {}
                        }
                    }
                    Err(error) => panic!("Poet permanently rejected graph batch: {error}"),
                }
            }
        }
        samples += 1;
        if print_batches {
            println!(
                "{}",
                serde_json::to_string(&request).expect("graph request serializes")
            );
        }
        if max_samples != 0 && samples >= max_samples {
            break;
        }
        tokio::select! {
            result = tokio::signal::ctrl_c() => {
                result.expect("failed to install graph Muse shutdown signal");
                break;
            }
            () = tokio::time::sleep(Duration::from_millis(interval_ms)) => {}
        }
    }
    eprintln!(
        "graph Muse stopped after {samples} accepted samples and {delivery_failures} transient delivery failures"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivery_backoff_is_bounded() {
        assert_eq!(delivery_backoff(0), Duration::from_millis(100));
        assert_eq!(delivery_backoff(1), Duration::from_millis(200));
        assert_eq!(delivery_backoff(32), Duration::from_millis(3_200));
        assert!(delivery_backoff(u32::MAX) <= MAX_DELIVERY_BACKOFF);
    }
}
