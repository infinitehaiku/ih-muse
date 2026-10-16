//! `ih-muse-kernel`, the node Muse: tails the node's kernel lines in the
//! host journal and sends its faults to Poet, and tails kubelet's pod log
//! files and sends them as OTLP logs ([`ih_muse_kernel::podlogs`]), both
//! with failover.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use clap::Parser;
use ih_muse_client::GraphPoetClient;
use ih_muse_k8s::api::{ApiConfig, KubeApi};
use ih_muse_k8s::backlog::{Backlog, BacklogConfig, DEFAULT_MAX_THINNING};
use ih_muse_k8s::model::Node;
use ih_muse_kernel::journal::{KernelJournal, ReadStats};
use ih_muse_kernel::podlogs::files::Checkpoint;
use ih_muse_kernel::podlogs::{otlp, Filter, PodLogStats, PodLogs, PodLogsConfig};
use ih_muse_kernel::{Cursor, KernelMuse, NodeConfig};

/// On, off, or on when its input exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
enum Switch {
    On,
    Off,
    Auto,
}

/// Where a first start begins reading the files already there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
enum StartAt {
    End,
    Beginning,
}

/// Poet, node, journal, pod log and cadence settings.
#[derive(Debug, Parser)]
#[command(
    about = "Send a node's kernel faults (segfaults, GPFs, MCE/EDAC, I/O and filesystem errors, OOM kills) to Infinite Haiku Poet"
)]
struct Args {
    /// Every Poet of the cluster, comma-separated (failover in that order).
    #[arg(
        long,
        env = "IH_MUSE_POET_URL",
        value_delimiter = ',',
        required_unless_present = "dry_run"
    )]
    poet_url: Vec<String>,
    /// CA that signs the Poets' HTTPS certificate (Poet's private TLS port).
    #[arg(long, env = "IH_MUSE_POET_CA_FILE")]
    poet_ca_file: Option<PathBuf>,
    /// Poet intake bearer token file.
    #[arg(
        long,
        env = "IH_MUSE_POET_TOKEN_PATH",
        required_unless_present = "dry_run"
    )]
    poet_token_path: Option<PathBuf>,
    #[arg(long, env = "IH_MUSE_ORGANIZATION", default_value = "local")]
    organization: String,
    /// The cluster root's identity, as the Kubernetes Muse sends it.
    #[arg(long, env = "CLUSTER_UID", default_value = "local")]
    cluster_uid: String,
    #[arg(long, env = "IH_K8S_CLUSTER_NAME")]
    cluster_name: Option<String>,
    /// This node (the pod's `spec.nodeName`).
    #[arg(long, env = "NODE_NAME")]
    node_name: String,
    /// The node's UID; read from the Kubernetes API (`get` on this node)
    /// when not given.
    #[arg(long, env = "NODE_UID")]
    node_uid: Option<String>,
    /// `host.name`; defaults to the node name (a pod's own host name is
    /// the pod's).
    #[arg(long, env = "IH_KERNEL_HOST_NAME")]
    host_name: Option<String>,
    /// Journal directories (each holds `<machine-id>/*.journal`), mounted
    /// read-only.
    #[arg(
        long,
        env = "IH_KERNEL_JOURNAL_DIRS",
        value_delimiter = ',',
        default_value = "/var/log/journal,/run/log/journal"
    )]
    journal_dir: Vec<PathBuf>,
    /// The host's boot id (the container shares the host's kernel).
    #[arg(long, default_value = "/proc/sys/kernel/random/boot_id")]
    boot_id_file: PathBuf,
    /// The host's uptime: the backfill window starts at boot.
    #[arg(long, default_value = "/proc/uptime")]
    uptime_file: PathBuf,
    /// What the Poets acknowledged, so a restart sends nothing twice.
    #[arg(
        long,
        env = "IH_KERNEL_CURSOR_FILE",
        default_value = "/var/lib/ih/kernel-muse/cursor.json"
    )]
    cursor_file: PathBuf,
    #[arg(long, env = "IH_MUSE_INTERVAL_SECONDS", default_value_t = 10)]
    interval_seconds: u64,
    /// Bound on unsent batches while no Poet answers (compressed bytes).
    #[arg(long, env = "IH_MUSE_BACKLOG_MAX_BYTES", default_value_t = 4 * 1024 * 1024)]
    backlog_max_bytes: usize,
    /// Kernel faults from the journal (`off` for pod logs only).
    #[arg(long, env = "IH_KERNEL_FAULTS", value_enum, default_value = "on")]
    kernel_faults: Switch,
    /// Pod logs: `auto` sends them when `--pod-logs-dir` exists.
    #[arg(long, env = "IH_PODLOGS", value_enum, default_value = "auto")]
    pod_logs: Switch,
    /// kubelet's `/var/log/pods`, mounted read-only.
    #[arg(long, env = "IH_PODLOGS_DIR", default_value = "/var/log/pods")]
    pod_logs_dir: PathBuf,
    /// kubelet's `/var/log/containers` (container ids); default: beside
    /// `--pod-logs-dir`.
    #[arg(long, env = "IH_PODLOGS_CONTAINERS_DIR")]
    pod_logs_containers_dir: Option<PathBuf>,
    /// The acknowledged read offsets, so a restart neither repeats nor
    /// skips lines.
    #[arg(
        long,
        env = "IH_PODLOGS_CHECKPOINT_FILE",
        default_value = "/var/lib/ih/kernel-muse/pod-logs.json"
    )]
    pod_logs_checkpoint_file: PathBuf,
    /// On a first start (no checkpoint), read the files already there from
    /// their end (only new lines) or from their beginning.
    #[arg(long, env = "IH_PODLOGS_START_AT", value_enum, default_value = "end")]
    pod_logs_start_at: StartAt,
    /// Namespaces whose pods' logs are sent (patterns with `*` and `?`;
    /// none: every namespace).
    #[arg(long, env = "IH_PODLOGS_INCLUDE_NAMESPACES", value_delimiter = ',')]
    pod_logs_include_namespaces: Vec<String>,
    #[arg(long, env = "IH_PODLOGS_EXCLUDE_NAMESPACES", value_delimiter = ',')]
    pod_logs_exclude_namespaces: Vec<String>,
    /// `<namespace>/<pod>` patterns to include beside the namespaces.
    #[arg(long, env = "IH_PODLOGS_INCLUDE_PODS", value_delimiter = ',')]
    pod_logs_include_pods: Vec<String>,
    #[arg(long, env = "IH_PODLOGS_EXCLUDE_PODS", value_delimiter = ',')]
    pod_logs_exclude_pods: Vec<String>,
    /// How often the pod log files are read.
    #[arg(long, env = "IH_PODLOGS_INTERVAL_MS", default_value_t = 2000)]
    pod_logs_interval_ms: u64,
    /// New log bytes per export (the memory bound: one export is held
    /// until a Poet takes it); at most 3 MiB (the intake takes 4 MiB).
    #[arg(long, env = "IH_PODLOGS_MAX_BATCH_BYTES", default_value_t = 1024 * 1024)]
    pod_logs_max_batch_bytes: usize,
    /// Read the boot's faults, print them and their totals, send nothing.
    #[arg(long)]
    dry_run: bool,
    /// One poll and send, then exit.
    #[arg(long)]
    once: bool,
}

fn now_unix_nano() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX)
        })
}

fn read_trimmed(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path)
        .map(|text| text.trim().to_string())
        .map_err(|error| format!("cannot read {}: {error}", path.display()))
}

/// The boot id as the journal writes it: 32 hex digits, no dashes.
fn boot_id(path: &Path) -> Result<String, String> {
    Ok(read_trimmed(path)?.replace('-', "").to_ascii_lowercase())
}

/// Wall-clock start of the boot in µs, a minute early (clock steps).
fn boot_start_usec(path: &Path) -> u64 {
    let uptime = read_trimmed(path)
        .ok()
        .and_then(|text| text.split_whitespace().next()?.parse::<f64>().ok())
        .unwrap_or(f64::MAX);
    let now_usec = now_unix_nano() / 1_000;
    let uptime_usec = (uptime * 1e6).min(now_usec as f64) as u64;
    now_usec
        .saturating_sub(uptime_usec)
        .saturating_sub(60_000_000)
}

fn report(stats: &ReadStats, last: &mut ReadStats) {
    let unusual = (
        stats.files_unreadable,
        stats.files_truncated,
        stats.messages_skipped,
    );
    if unusual
        != (
            last.files_unreadable,
            last.files_truncated,
            last.messages_skipped,
        )
        && unusual != (0, 0, 0)
    {
        eprintln!(
            "journal: {} file(s) read, {} unreadable, {} stopped early (being written or corrupt), {} compressed or empty message(s) skipped",
            stats.files_read, stats.files_unreadable, stats.files_truncated, stats.messages_skipped
        );
    }
    *last = stats.clone();
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Args::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ih-muse-kernel: {error}");
            ExitCode::FAILURE
        }
    }
}

/// The node's UID from the API, retried until it answers.
async fn node_uid(name: &str) -> Result<String, Box<dyn std::error::Error>> {
    let config = ApiConfig::resolve(None, None, None, |key| std::env::var(key).ok())?;
    let api = KubeApi::new(config)?;
    let mut delay = Duration::from_secs(2);
    loop {
        match api.get::<Node>(&format!("/api/v1/nodes/{name}")).await {
            Ok(node) if !node.metadata.uid.is_empty() => return Ok(node.metadata.uid),
            Ok(_) => eprintln!("node {name} has no UID yet"),
            Err(error) => eprintln!("node {name} UID unavailable: {error}"),
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(60));
    }
}

async fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    if args.dry_run {
        return dry_run(&args, &mut KernelJournal::new(args.journal_dir.clone()));
    }
    let pod_logs = match args.pod_logs {
        Switch::On => true,
        Switch::Off => false,
        Switch::Auto => args.pod_logs_dir.is_dir(),
    };
    if args.kernel_faults == Switch::Off && !pod_logs {
        return Err("kernel faults and pod logs are both off".into());
    }
    let token_path = args
        .poet_token_path
        .as_ref()
        .ok_or("--poet-token-path is required")?
        .to_string_lossy()
        .into_owned();
    let poet_token = ih_muse::secret_file::read_secret_file(&token_path, "Poet token")?;
    let client = match &args.poet_ca_file {
        Some(path) => {
            let pem = std::fs::read(path)
                .map_err(|error| format!("cannot read Poet CA {}: {error}", path.display()))?;
            GraphPoetClient::cluster_private_tls(args.poet_url.clone(), poet_token, &pem)?
        }
        None => GraphPoetClient::cluster(args.poet_url.clone(), poet_token)?,
    };
    let pod_task = pod_logs.then(|| {
        let config = pod_logs_config(&args);
        tokio::spawn(pod_logs_loop(
            client.clone(),
            config,
            args.pod_logs_checkpoint_file.clone(),
            Duration::from_millis(args.pod_logs_interval_ms.max(100)),
            args.once,
        ))
    });
    if args.kernel_faults == Switch::Off {
        return match pod_task {
            Some(task) => task.await?.map_err(|error| error.to_string().into()),
            None => Ok(()),
        };
    }
    let kernel = kernel_loop(&args, client).await;
    if let (Ok(()), Some(task), true) = (&kernel, pod_task, args.once) {
        return task.await?.map_err(|error| error.to_string().into());
    }
    kernel
}

/// The pod log settings from the command line.
fn pod_logs_config(args: &Args) -> PodLogsConfig {
    let mut config = PodLogsConfig::new(
        args.pod_logs_dir.clone(),
        args.node_name.clone(),
        args.cluster_uid.clone(),
    );
    if let Some(dir) = &args.pod_logs_containers_dir {
        config.containers_root = dir.clone();
    }
    config.cluster_name = args
        .cluster_name
        .clone()
        .filter(|name| !name.trim().is_empty());
    let patterns = |list: &[String]| -> Vec<String> {
        list.iter()
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect()
    };
    config.filter = Filter {
        include_namespaces: patterns(&args.pod_logs_include_namespaces),
        exclude_namespaces: patterns(&args.pod_logs_exclude_namespaces),
        include_pods: patterns(&args.pod_logs_include_pods),
        exclude_pods: patterns(&args.pod_logs_exclude_pods),
    };
    config.start_at_end = args.pod_logs_start_at == StartAt::End;
    config.max_batch_bytes = args
        .pod_logs_max_batch_bytes
        .clamp(4 * 1024, 3 * 1024 * 1024);
    config.max_file_bytes = config.max_file_bytes.min(config.max_batch_bytes);
    config
}

/// Exports sent per tick at most while files have more to read.
const EXPORTS_PER_TICK: usize = 16;

/// Reads the pod log files every `interval` and sends their new lines;
/// an export a Poet did not take is sent again before anything else is
/// read (the files hold the rest).
async fn pod_logs_loop(
    client: GraphPoetClient,
    config: PodLogsConfig,
    checkpoint_file: PathBuf,
    interval: Duration,
    once: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if let Some(parent) = checkpoint_file.parent() {
        let _created = std::fs::create_dir_all(parent);
    }
    let checkpoint = Checkpoint::load(&checkpoint_file);
    println!(
        "Pod logs: reading {} ({}), {}; sending OTLP logs to {}",
        config.pods_root.display(),
        config.containers_root.display(),
        match &checkpoint {
            Some(saved) => format!(
                "resuming {} file(s) from {}",
                saved.files.len(),
                checkpoint_file.display()
            ),
            None if config.start_at_end => "first start: new lines only".into(),
            None => "first start: from the beginning".into(),
        },
        client.preferred_endpoint()
    );
    let mut logs = PodLogs::new(config, checkpoint);
    let mut saved: Option<Checkpoint> = None;
    let mut pending = None;
    let mut reported = PodLogStats::default();
    let mut cadence = tokio::time::interval(interval);
    cadence.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = cadence.tick() => {}
            _ = tokio::signal::ctrl_c() => return Ok(()),
        }
        for _ in 0..EXPORTS_PER_TICK {
            if pending.is_none() {
                pending = logs.poll(now_unix_nano());
            }
            let Some(batch) = pending.as_ref() else { break };
            match client.publish_otlp_logs(&batch.export).await {
                Ok(answer) => {
                    let (rejected, message) = otlp::rejected_records(&answer);
                    if rejected > 0 {
                        eprintln!("[PARTIAL] pod logs: the Poet refused {rejected} of {} record(s): {message}", batch.records);
                    }
                    logs.commit(batch, false);
                    pending = None;
                }
                Err(ih_muse_core::MuseError::Validation(message)) => {
                    eprintln!(
                        "[REJECTED] pod logs export of {} record(s), skipped: {message}",
                        batch.records
                    );
                    logs.commit(batch, true);
                    pending = None;
                }
                Err(error) => {
                    eprintln!("[FAILED] pod logs: {error}; the export is kept and sent again");
                    break;
                }
            }
        }
        let checkpoint = logs.checkpoint();
        if saved.as_ref() != Some(&checkpoint) {
            match checkpoint.save(&checkpoint_file) {
                Ok(()) => saved = Some(checkpoint),
                Err(error) => eprintln!(
                    "cannot save pod log checkpoint {}: {error}",
                    checkpoint_file.display()
                ),
            }
        }
        let stats = logs.stats();
        if stats.records_sent != reported.records_sent
            || (
                stats.bytes_lost,
                stats.unparsed_lines,
                stats.restarts,
                stats.exports_refused,
            ) != (
                reported.bytes_lost,
                reported.unparsed_lines,
                reported.restarts,
                reported.exports_refused,
            )
        {
            println!(
                "[ACCEPTED] pod logs: {} record(s) from {} file(s) in total ({} bytes read; {} unparsed line(s), {} file restart(s), {} bytes lost to deletion, {} export(s) refused)",
                stats.records_sent, stats.files, stats.bytes_read, stats.unparsed_lines, stats.restarts, stats.bytes_lost, stats.exports_refused
            );
            reported = stats.clone();
        }
        if once {
            return if pending.is_none() {
                Ok(())
            } else {
                Err("unsent pod logs at exit".into())
            };
        }
    }
}

/// Reads the journal every interval and sends the faults.
async fn kernel_loop(
    args: &Args,
    client: GraphPoetClient,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut journal = KernelJournal::new(args.journal_dir.clone());
    let node_uid = match args.node_uid.clone().filter(|uid| !uid.trim().is_empty()) {
        Some(uid) => uid,
        None => node_uid(&args.node_name).await?,
    };
    let config = NodeConfig {
        organization: args.organization.clone(),
        cluster_uid: args.cluster_uid.clone(),
        cluster_name: args
            .cluster_name
            .clone()
            .filter(|name| !name.trim().is_empty()),
        node_name: args.node_name.clone(),
        node_uid,
        host_name: args
            .host_name
            .clone()
            .unwrap_or_else(|| args.node_name.clone()),
    };
    if let Some(parent) = args.cursor_file.parent() {
        let _created = std::fs::create_dir_all(parent);
    }
    let cursor = Cursor::load(&args.cursor_file);
    let mut saved = cursor.clone();
    let mut muse = KernelMuse::new(config, cursor)?;
    let interval = Duration::from_secs(args.interval_seconds.max(1));
    let mut backlog = Backlog::new(
        BacklogConfig {
            max_bytes: args.backlog_max_bytes,
            max_thinning: DEFAULT_MAX_THINNING,
        },
        u64::try_from(interval.as_nanos()).unwrap_or(u64::MAX),
    );
    println!(
        "Kernel Muse {} {} reading {} for node {} ({}); sending to {} every {}s",
        muse.config().source_id(),
        ih_muse_kernel::VERSION,
        args.journal_dir
            .iter()
            .map(|dir| dir.display().to_string())
            .collect::<Vec<_>>()
            .join(", "),
        muse.config().node_name,
        muse.config().node_uid,
        args.poet_url.join(", "),
        interval.as_secs()
    );
    let mut last_stats = ReadStats::default();
    let mut cadence = tokio::time::interval(interval);
    cadence.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = cadence.tick() => {}
            _ = tokio::signal::ctrl_c() => return Ok(()),
        }
        let now = now_unix_nano();
        match boot_id(&args.boot_id_file) {
            Ok(boot) => {
                let (records, stats) = journal.poll(&boot, boot_start_usec(&args.uptime_file));
                report(&stats, &mut last_stats);
                let (faults, skips) = muse.ingest(records);
                for new in &faults {
                    println!("fault {}: {}", new.fault.kind, new.record.message);
                }
                if !skips.is_empty() {
                    eprintln!("{} fault line(s) could not be read (counted)", skips.len());
                }
                if let Some(request) = muse.intake(&faults, &skips, now) {
                    let loss = backlog.push(&request, now);
                    if !loss.is_empty() {
                        eprintln!(
                            "Poet unreachable; backlog at its {} byte bound: dropped {} batch(es) with {} event(s) (totals {} and {})",
                            args.backlog_max_bytes,
                            loss.dropped_intervals,
                            loss.dropped_events,
                            backlog.total().dropped_intervals,
                            backlog.total().dropped_events
                        );
                    }
                }
            }
            Err(error) => eprintln!("{error}"),
        }
        let mut sent = 0;
        while let Some(request) = backlog.front() {
            match client.publish(&request).await {
                Ok(answer) => {
                    muse.acknowledge(&request, &answer);
                    backlog.pop_acknowledged();
                    sent += 1;
                }
                Err(ih_muse_core::MuseError::Validation(message)) => {
                    backlog.pop_rejected();
                    eprintln!("[REJECTED] {}: {message}", request.delivery_id);
                }
                Err(error) => {
                    eprintln!(
                        "[FAILED] {error}; {} batch(es) queued for retry",
                        backlog.len()
                    );
                    break;
                }
            }
        }
        if sent > 0 {
            println!(
                "[ACCEPTED] {sent} batch(es) by {}",
                client.preferred_endpoint()
            );
        }
        // Only what every queued request carried was acknowledged.
        if backlog.is_empty() && muse.acknowledged_cursor() != &saved {
            match muse.acknowledged_cursor().save(&args.cursor_file) {
                Ok(()) => saved = muse.acknowledged_cursor().clone(),
                Err(error) => {
                    eprintln!("cannot save cursor {}: {error}", args.cursor_file.display())
                }
            }
        }
        if args.once {
            return if backlog.is_empty() {
                Ok(())
            } else {
                Err("unsent batches at exit".into())
            };
        }
    }
}

/// Reads the boot and prints its faults and totals.
fn dry_run(args: &Args, journal: &mut KernelJournal) -> Result<(), Box<dyn std::error::Error>> {
    let boot = boot_id(&args.boot_id_file)?;
    let since = boot_start_usec(&args.uptime_file);
    let started = std::time::Instant::now();
    let (records, stats) = journal.poll(&boot, since);
    let elapsed = started.elapsed();
    let config = NodeConfig {
        organization: args.organization.clone(),
        cluster_uid: args.cluster_uid.clone(),
        cluster_name: args.cluster_name.clone(),
        node_name: args.node_name.clone(),
        node_uid: args.node_uid.clone().unwrap_or_default(),
        host_name: args
            .host_name
            .clone()
            .unwrap_or_else(|| args.node_name.clone()),
    };
    let lines = records.len();
    let mut muse = KernelMuse::new(config, Cursor::default())?;
    let (faults, skips) = muse.ingest(records);
    for new in &faults {
        let f = &new.fault.fields;
        println!(
            "{} {} {}[{}] cpu {} in {}",
            new.record.realtime_usec,
            new.fault.kind,
            f.process.as_deref().unwrap_or("-"),
            f.pid.map_or("-".into(), |pid| pid.to_string()),
            f.cpu.map_or("-".into(), |cpu| cpu.to_string()),
            f.object.as_deref().unwrap_or("-"),
        );
    }
    println!(
        "boot {boot}: {lines} kernel line(s), {} fault(s), {} unreadable fault line(s); read in {} ms; {stats:?}",
        faults.len(),
        skips.len(),
        elapsed.as_millis()
    );
    for (kind, total) in muse.totals() {
        println!("total {kind}: {total}");
    }
    for ((kind, cpu), total) in muse.cpu_totals() {
        println!("cpu {cpu} {kind}: {total}");
    }
    Ok(())
}
