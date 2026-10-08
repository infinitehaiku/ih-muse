//! Authenticated, bounded delivery for the canonical graph intake contract.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use ih_muse_core::{MuseError, MuseResult};
use ih_muse_proto::trace::{DeliveryTrace, Sampler, TRACEPARENT, TRACESTATE};
use ih_muse_proto::{GraphIntakeAnswer, GraphIntakeRequest};
use reqwest::{Client, StatusCode};
use tokio::time::sleep;

/// Sends validated `ih.graph.intake.v1` batches to a Poet cluster.
///
/// Any Poet of the cluster accepts a batch; the Poets replicate among
/// themselves. The client sticks to the Poet that last accepted a batch and
/// moves to the next one when it fails transiently, so one Muse feeds the
/// cluster through an outage. The bearer token is held only in this client
/// and is never included in an error. Delivery IDs are caller-owned and stay
/// stable across retries and Poets, so a retry after a lost acknowledgement
/// is stored once.
///
/// Every delivery carries a W3C trace context (`traceparent` and
/// `tracestate` headers, [`DeliveryTrace`]), sampled by [`Sampler`] from
/// `IH_TRACE_SAMPLE_RATIO`: the Poet that accepts it records the Muse's
/// delivery and its own intake as one trace. The body is unchanged.
#[derive(Clone)]
pub struct GraphPoetClient {
    endpoints: Vec<String>,
    /// Index of the Poet that last accepted a batch; shared by clones.
    preferred: Arc<AtomicUsize>,
    token: String,
    client: Client,
    /// Head sampling of the delivery traces this client starts.
    sampler: Sampler,
}

impl GraphPoetClient {
    /// Attempts per Poet before moving to the next one.
    const MAX_ATTEMPTS: usize = 2;
    const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

    /// Build a bounded graph client for an HTTPS or loopback HTTP Poet endpoint.
    pub fn new(endpoint: impl Into<String>, token: impl Into<String>) -> MuseResult<Self> {
        Self::build(vec![endpoint.into()], token.into(), None)
    }

    /// Build a client for every Poet of a cluster, in preference order.
    pub fn cluster(endpoints: Vec<String>, token: impl Into<String>) -> MuseResult<Self> {
        Self::build(endpoints, token.into(), None)
    }

    /// Build a bounded HTTPS graph client that trusts one owner-provided CA.
    pub fn private_tls(
        endpoint: impl Into<String>,
        token: impl Into<String>,
        ca_pem: &[u8],
    ) -> MuseResult<Self> {
        Self::build(vec![endpoint.into()], token.into(), Some(ca_pem))
    }

    /// Build a client for every Poet of a cluster that serves HTTPS with one
    /// owner-provided CA (an in-cluster Muse reaching Poet's private TLS port).
    pub fn cluster_private_tls(
        endpoints: Vec<String>,
        token: impl Into<String>,
        ca_pem: &[u8],
    ) -> MuseResult<Self> {
        Self::build(endpoints, token.into(), Some(ca_pem))
    }

    /// The Poet the next batch goes to first.
    pub fn preferred_endpoint(&self) -> &str {
        &self.endpoints[self.preferred.load(Ordering::Relaxed) % self.endpoints.len()]
    }

    fn build(endpoints: Vec<String>, token: String, ca_pem: Option<&[u8]>) -> MuseResult<Self> {
        let endpoints = endpoints
            .into_iter()
            .map(|endpoint| endpoint.trim().trim_end_matches('/').to_owned())
            .filter(|endpoint| !endpoint.is_empty())
            .collect::<Vec<_>>();
        if endpoints.is_empty() || endpoints.len() > 16 {
            return Err(MuseError::Configuration(
                "graph client needs 1 to 16 Poet endpoints".into(),
            ));
        }
        for endpoint in &endpoints {
            if !(endpoint.starts_with("https://") || endpoint.starts_with("http://127.0.0.1")) {
                return Err(MuseError::Configuration(
                    "graph Poet endpoint must use HTTPS or explicit loopback HTTP".into(),
                ));
            }
            if ca_pem.is_some() && !endpoint.starts_with("https://") {
                return Err(MuseError::Configuration(
                    "an owner CA requires an HTTPS graph Poet endpoint".into(),
                ));
            }
        }
        if token.is_empty() || token.len() > 4096 || token.contains(['\r', '\n', '\0']) {
            return Err(MuseError::Configuration(
                "graph Poet bearer token is invalid".into(),
            ));
        }
        let mut builder = Client::builder().timeout(Self::REQUEST_TIMEOUT);
        if let Some(ca_pem) = ca_pem {
            if ca_pem.len() > 1024 * 1024
                || !ca_pem.starts_with(b"-----BEGIN CERTIFICATE-----")
                || !ca_pem.ends_with(b"-----END CERTIFICATE-----\n")
            {
                return Err(MuseError::Configuration(
                    "graph Poet owner CA is not bounded PEM".into(),
                ));
            }
            let certificate = reqwest::Certificate::from_pem(ca_pem).map_err(|_| {
                MuseError::Configuration("graph Poet owner CA is not valid PEM".into())
            })?;
            builder = builder.add_root_certificate(certificate);
        }
        let client = builder
            .build()
            .map_err(|error| MuseError::Unavailable(error.to_string()))?;
        Ok(Self {
            endpoints,
            preferred: Arc::new(AtomicUsize::new(0)),
            token,
            client,
            sampler: Sampler::from_env(),
        })
    }

    /// Replaces the sampling of the delivery traces this client starts.
    pub fn with_sampler(mut self, sampler: Sampler) -> Self {
        self.sampler = sampler;
        self
    }

    /// A delivery trace for `request`: a new trace that starts when the
    /// batch's newest observation was made (else now).
    pub fn delivery_trace(&self, request: &GraphIntakeRequest) -> DeliveryTrace {
        let observed = request
            .batch
            .observations
            .iter()
            .map(|observation| observation.provenance.observed_at_unix_nano)
            .max()
            .filter(|time| *time > 0);
        let created = observed.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos().min(u64::MAX as u128) as u64)
        });
        DeliveryTrace::start(&self.sampler, created)
    }

    /// Deliver a prevalidated graph batch, retrying only transient failures,
    /// and return the accepting Poet's answer (its definitions epoch).
    ///
    /// Starts at the preferred Poet and moves on after `MAX_ATTEMPTS`
    /// transient failures. A 4xx answer is final: another Poet of the same
    /// cluster would reject the same batch for the same reason.
    pub async fn publish(&self, request: &GraphIntakeRequest) -> MuseResult<GraphIntakeAnswer> {
        let trace = self.delivery_trace(request);
        self.publish_traced(request, &trace).await
    }

    /// [`Self::publish`] with the caller's delivery trace (the same on
    /// every retry of one delivery).
    pub async fn publish_traced(
        &self,
        request: &GraphIntakeRequest,
        trace: &DeliveryTrace,
    ) -> MuseResult<GraphIntakeAnswer> {
        let traceparent = trace.context.traceparent();
        let tracestate = trace.tracestate();
        request
            .validate()
            .map_err(|error| MuseError::Validation(error.to_string()))?;
        let start = self.preferred.load(Ordering::Relaxed);
        let mut failures = Vec::new();
        for offset in 0..self.endpoints.len() {
            let index = (start + offset) % self.endpoints.len();
            let url = format!("{}/api/v1/graph/batches", self.endpoints[index]);
            for attempt in 0..Self::MAX_ATTEMPTS {
                match self
                    .client
                    .post(&url)
                    .bearer_auth(&self.token)
                    .header(TRACEPARENT, &traceparent)
                    .header(TRACESTATE, &tracestate)
                    .json(request)
                    .send()
                    .await
                {
                    Ok(response) if response.status() == StatusCode::CREATED => {
                        self.preferred.store(index, Ordering::Relaxed);
                        // Stored already: an answer that does not parse (an
                        // empty body) only means the Poet named no epoch.
                        let body = response.bytes().await.unwrap_or_default();
                        return Ok(serde_json::from_slice(&body).unwrap_or_default());
                    }
                    Ok(response) if response.status().is_client_error() => {
                        return Err(MuseError::Validation(format!(
                            "graph intake rejected delivery: HTTP {}",
                            response.status()
                        )));
                    }
                    Ok(response) => {
                        failures.push(format!("Poet {index}: HTTP {}", response.status()))
                    }
                    Err(error) => failures.push(format!("Poet {index}: {error}")),
                }
                if attempt + 1 < Self::MAX_ATTEMPTS {
                    sleep(Duration::from_millis(25_u64 << attempt)).await;
                }
            }
        }
        Err(MuseError::Unavailable(format!(
            "graph intake unavailable on {} Poet(s): {}",
            self.endpoints.len(),
            failures.join("; ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_transport_security_or_explicit_loopback() {
        assert!(GraphPoetClient::new("http://poet.example", "token").is_err());
        assert!(GraphPoetClient::new("https://poet.example", "token").is_ok());
        assert!(GraphPoetClient::new("http://127.0.0.1:18080", "token").is_ok());
        assert!(
            GraphPoetClient::private_tls("http://127.0.0.1:18080", "token", b"not-used").is_err()
        );
        assert!(GraphPoetClient::private_tls(
            "https://poet.example",
            "token",
            b"not-a-certificate"
        )
        .is_err());
        assert!(GraphPoetClient::cluster(Vec::new(), "token").is_err());
        assert!(GraphPoetClient::cluster_private_tls(
            vec![
                "https://poet-0.example".into(),
                "http://127.0.0.1:18080".into()
            ],
            "token",
            b"not-used"
        )
        .is_err());
    }

    /// A tiny HTTP server that answers every request with `status`.
    async fn server(status: &'static str) -> (String, Arc<AtomicUsize>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                counter.fetch_add(1, Ordering::SeqCst);
                let mut buffer = vec![0_u8; 64 * 1024];
                let _ = socket.read(&mut buffer).await;
                let reply =
                    format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                let _ = socket.write_all(reply.as_bytes()).await;
            }
        });
        (format!("http://{address}"), hits)
    }

    fn request() -> GraphIntakeRequest {
        serde_json::from_value(serde_json::json!({
            "schema_version": ih_muse_proto::GRAPH_INTAKE_SCHEMA_VERSION,
            "contract_revision": ih_muse_proto::GRAPH_INTAKE_CONTRACT_REVISION,
            "delivery_id": "muse-test-1",
            "organization": "org",
            "owner_id": "org",
            "batch": {
                "schema_version": ih_muse_proto::GRAPH_SCHEMA_VERSION,
                "contract_revision": ih_muse_proto::GRAPH_CONTRACT_REVISION,
                "entities": [], "relations": [], "observations": [],
                "events": [], "derivations": [], "availability": []
            }
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn a_down_poet_fails_over_and_the_healthy_one_stays_preferred() {
        let (down, down_hits) = server("503 Service Unavailable").await;
        let (up, up_hits) = server("201 Created").await;
        let client = GraphPoetClient::cluster(vec![down.clone(), up.clone()], "token").unwrap();
        client.publish(&request()).await.unwrap();
        assert_eq!(client.preferred_endpoint(), up);
        assert_eq!(
            down_hits.load(Ordering::SeqCst),
            2,
            "retried the first Poet, then moved on"
        );
        client.publish(&request()).await.unwrap();
        assert_eq!(
            down_hits.load(Ordering::SeqCst),
            2,
            "the healthy Poet stays preferred"
        );
        assert_eq!(up_hits.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_rejected_batch_is_not_retried_elsewhere() {
        let (reject, _) = server("400 Bad Request").await;
        let (up, up_hits) = server("201 Created").await;
        let client = GraphPoetClient::cluster(vec![reject, up], "token").unwrap();
        assert!(matches!(
            client.publish(&request()).await,
            Err(MuseError::Validation(_))
        ));
        assert_eq!(up_hits.load(Ordering::SeqCst), 0);
    }

    /// Like [`server`], keeping each request's head (request line and headers).
    async fn recording_server(
        status: &'static str,
    ) -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let heads = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = heads.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let mut buffer = vec![0_u8; 64 * 1024];
                let read = socket.read(&mut buffer).await.unwrap_or(0);
                let text = String::from_utf8_lossy(&buffer[..read]).to_string();
                let head = text
                    .split("\r\n\r\n")
                    .next()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                seen.lock().unwrap().push(head);
                let reply =
                    format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                let _ = socket.write_all(reply.as_bytes()).await;
            }
        });
        (format!("http://{address}"), heads)
    }

    fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
        head.lines()
            .find_map(|line| line.strip_prefix(&format!("{name}: ")))
    }

    #[tokio::test]
    async fn every_attempt_of_a_delivery_carries_the_same_trace() {
        let (down, down_heads) = recording_server("503 Service Unavailable").await;
        let (up, up_heads) = recording_server("201 Created").await;
        let client = GraphPoetClient::cluster(vec![down, up], "token")
            .unwrap()
            .with_sampler(Sampler {
                ratio: 1.0,
                ..Sampler::default()
            });
        let mut request = request();
        request.delivery_id = "muse-test-traced".into();
        let trace = client.delivery_trace(&request);
        client.publish_traced(&request, &trace).await.unwrap();
        let heads: Vec<String> = down_heads
            .lock()
            .unwrap()
            .iter()
            .chain(up_heads.lock().unwrap().iter())
            .cloned()
            .collect();
        assert_eq!(
            heads.len(),
            3,
            "two attempts on the down Poet, one on the healthy one"
        );
        for head in &heads {
            let parent = header(head, TRACEPARENT).expect("traceparent on every attempt");
            let state = header(head, TRACESTATE).expect("tracestate on every attempt");
            assert_eq!(
                DeliveryTrace::from_headers(Some(parent), Some(state)),
                Some(trace.clone())
            );
            assert!(parent.ends_with("-01"), "sampled at ratio 1: {parent}");
        }
    }

    #[tokio::test]
    async fn an_unsampled_delivery_still_names_its_trace() {
        let (up, heads) = recording_server("201 Created").await;
        let client = GraphPoetClient::cluster(vec![up], "token")
            .unwrap()
            .with_sampler(Sampler {
                ratio: 0.0,
                ..Sampler::default()
            });
        client.publish(&request()).await.unwrap();
        let head = heads.lock().unwrap()[0].clone();
        let parent = header(&head, TRACEPARENT).unwrap();
        assert!(parent.ends_with("-00"), "{parent}");
        let created = DeliveryTrace::from_headers(Some(parent), header(&head, TRACESTATE))
            .unwrap()
            .created_unix_nano;
        assert!(created > 0, "an empty batch starts its trace now");
    }
}
