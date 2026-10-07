//! `ih-muse-k8s`: collects every interval and sends to Poet with failover.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use clap::Parser;
use ih_muse_client::GraphPoetClient;
use ih_muse_k8s::api::{ApiConfig, KubeApi};
use ih_muse_k8s::backlog::{
    BacklogConfig, DEFAULT_BACKLOG_MAX_BYTES, DEFAULT_MAX_THINNING,
    DEFAULT_REPLAY_BATCHES_PER_INTERVAL,
};
use ih_muse_k8s::identity::MuseIdentity;
use ih_muse_k8s::K8sMuse;

/// Poet, Kubernetes API and cadence settings.
#[derive(Debug, Parser)]
#[command(
    about = "Send a Kubernetes cluster's nodes, pods, containers and usage to Infinite Haiku Poet"
)]
struct Args {
    /// Every Poet of the cluster, comma-separated. The Muse sends to one and
    /// fails over to the next; the Poets replicate among themselves.
    #[arg(long, env = "IH_MUSE_POET_URL", value_delimiter = ',', required = true)]
    poet_url: Vec<String>,
    /// CA that signs the Poets' HTTPS certificate (Poet's private TLS port).
    #[arg(long, env = "IH_MUSE_POET_CA_FILE")]
    poet_ca_file: Option<PathBuf>,
    /// Poet intake bearer token file.
    #[arg(long, env = "IH_MUSE_POET_TOKEN_PATH")]
    poet_token_path: PathBuf,
    /// Tenant (organization) the Poets' intake token is scoped to.
    #[arg(long, env = "IH_MUSE_ORGANIZATION", default_value = "local")]
    organization: String,
    #[arg(long, env = "IH_MUSE_INTERVAL_SECONDS", default_value_t = 5)]
    interval_seconds: u64,
    /// Namespace whose pods are observed (the pod's own, via the downward API).
    #[arg(long, env = "POD_NAMESPACE")]
    namespace: String,
    /// Observe every namespace (pods, workloads, Warning events), not only
    /// `--namespace`; needs the cluster-wide read grants (README, RBAC).
    #[arg(long, env = "IH_K8S_ALL_NAMESPACES", default_value_t = false)]
    all_namespaces: bool,
    /// Identity of the cluster root; defaults to the cluster name, else the API host.
    #[arg(long, env = "CLUSTER_UID")]
    cluster_uid: Option<String>,
    /// Name of this cluster: the Muse instance in its producer id.
    #[arg(long, env = "IH_K8S_CLUSTER_NAME")]
    cluster_name: Option<String>,
    /// Kubernetes API base URL; default from KUBERNETES_SERVICE_HOST/PORT.
    #[arg(long)]
    api_url: Option<String>,
    /// Kubernetes API bearer token file; default the service account token.
    #[arg(long)]
    api_token_file: Option<PathBuf>,
    /// CA that signs the API server certificate; default the service account CA.
    #[arg(long)]
    api_ca_file: Option<PathBuf>,
    /// Bound on the unsent batches kept while no Poet answers, in
    /// compressed bytes. Beyond it the oldest data is thinned (every second,
    /// fourth, ... sample kept), then dropped; both are reported to Poet.
    #[arg(long, env = "IH_MUSE_BACKLOG_MAX_BYTES", default_value_t = DEFAULT_BACKLOG_MAX_BYTES)]
    backlog_max_bytes: usize,
    /// Batches replayed per interval once a Poet answers again, so a
    /// backlog does not flood it (at least 2, so the backlog drains).
    #[arg(long, env = "IH_MUSE_REPLAY_BATCHES_PER_INTERVAL", default_value_t = DEFAULT_REPLAY_BATCHES_PER_INTERVAL)]
    replay_batches_per_interval: usize,
    /// Collect and send one batch, then exit.
    #[arg(long)]
    once: bool,
    /// Collect and send this many batches, then exit.
    #[arg(long, env = "IH_MUSE_SAMPLE_COUNT")]
    samples: Option<u32>,
}

fn now_unix_nano() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX)
        })
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Args::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ih-muse-k8s: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    if args.once && args.samples.is_some() {
        return Err("--once and --samples cannot be used together".into());
    }
    let token_path = args.poet_token_path.to_string_lossy().into_owned();
    let poet_token = ih_muse::secret_file::read_secret_file(&token_path, "Poet token")?;
    let client = match &args.poet_ca_file {
        Some(path) => {
            let pem = std::fs::read(path)
                .map_err(|error| format!("cannot read Poet CA {}: {error}", path.display()))?;
            GraphPoetClient::cluster_private_tls(args.poet_url.clone(), poet_token, &pem)?
        }
        None => GraphPoetClient::cluster(args.poet_url.clone(), poet_token)?,
    };
    let api_config = ApiConfig::resolve(
        args.api_url.as_deref(),
        args.api_token_file.as_deref(),
        args.api_ca_file.as_deref(),
        |name| std::env::var(name).ok(),
    )?;
    let identity = MuseIdentity::new(args.cluster_name.as_deref(), &api_config.host());
    let cluster_uid = args
        .cluster_uid
        .clone()
        .unwrap_or_else(|| identity.instance.clone());
    let api = KubeApi::new(api_config)?;
    let interval = Duration::from_secs(args.interval_seconds.max(1));
    let mut muse = K8sMuse::new(
        identity.clone(),
        &args.organization,
        &cluster_uid,
        &args.namespace,
        u64::try_from(interval.as_nanos()).unwrap_or(u64::MAX),
    )?
    .with_cluster_name(args.cluster_name.as_deref())
    .with_all_namespaces(args.all_namespaces)
    .with_backlog(BacklogConfig {
        max_bytes: args.backlog_max_bytes,
        max_thinning: DEFAULT_MAX_THINNING,
    });
    let replay = args.replay_batches_per_interval.max(2);
    println!(
        "Kubernetes Muse {} {} watching {} of cluster {cluster_uid} via {}; sending to {} (failover in that order) every {}s",
        identity.source_id(),
        identity.source_revision(),
        if args.all_namespaces {
            "every namespace".to_string()
        } else {
            format!("namespace {}", args.namespace)
        },
        api.config().base_url,
        args.poet_url.join(", "),
        interval.as_secs()
    );

    let limit = args.samples.or_else(|| args.once.then_some(1));
    let mut collected = 0_u32;
    let mut cadence = tokio::time::interval(interval);
    cadence.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = cadence.tick() => {}
            _ = tokio::signal::ctrl_c() => return Ok(()),
        }
        let now = now_unix_nano();
        match muse.collect(&api, |message| eprintln!("{message}")).await {
            Ok(snapshot) => {
                let request = muse.intake(&snapshot, now);
                let (entities, observations, events) = (
                    request.batch.entities.len(),
                    request.batch.observations.len(),
                    request.batch.events.len(),
                );
                for event in &request.batch.events {
                    if let Some(ih_muse_proto::AttributeValue::String(summary)) = &event.body {
                        println!("event {}: {summary}", event.event_id);
                    }
                }
                let loss = muse.enqueue(request, now);
                if !loss.is_empty() {
                    let total = muse.lost();
                    eprintln!(
                        "Poet unreachable; backlog at its {} byte bound: thinned {} and dropped {} oldest interval(s) (totals {} and {}); reported to Poet",
                        args.backlog_max_bytes,
                        loss.thinned_intervals,
                        loss.dropped_intervals,
                        total.thinned_intervals,
                        total.dropped_intervals
                    );
                }
                println!("collected at {now}: {entities} entities, {observations} observations, {events} events");
            }
            // Retried next interval; what is queued still goes out below.
            Err(error) => eprintln!("collection error: {error}"),
        }
        collected += 1;
        let last = limit.is_some_and(|limit| collected >= limit);
        // Paced replay; the last of a bounded run sends everything.
        let budget = if last { usize::MAX } else { replay };
        match muse
            .send_pending(budget, |request| {
                let client = client.clone();
                async move { client.publish(&request).await }
            })
            .await
        {
            Ok(sent) if sent > 0 => println!(
                "[ACCEPTED] {sent} batch(es) by {}; {} queued ({} bytes)",
                client.preferred_endpoint(),
                muse.pending(),
                muse.pending_bytes()
            ),
            Ok(_) => {}
            Err(error) => eprintln!(
                "[FAILED] {error}; {} batch(es) queued for retry",
                muse.pending()
            ),
        }
        if last {
            return if muse.pending() == 0 {
                Ok(())
            } else {
                Err("unsent batches at exit".into())
            };
        }
    }
}
