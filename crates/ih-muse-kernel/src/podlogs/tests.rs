//! The tailer over a simulated `/var/log/pods` tree: what is sent, in
//! which resource, and that rotation, restarts, truncation, checkpoints,
//! budgets and unacknowledged exports neither lose nor repeat a line.

use std::path::{Path, PathBuf};

use super::otlp::decode::{all, attributes, one, scalar};
use super::*;

const T0: u64 = 1_760_600_000_000_000_000;
const NOW: u64 = T0 + 9_000_000_000;

/// A node's `/var/log` with kubelet's `pods` and `containers` trees.
struct Node {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Node {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        std::fs::create_dir_all(root.join("pods")).unwrap();
        std::fs::create_dir_all(root.join("containers")).unwrap();
        Self { _dir: dir, root }
    }

    fn container_dir(&self, ns: &str, pod: &str, uid: &str, container: &str) -> PathBuf {
        let dir = self.root.join(format!("pods/{ns}_{pod}_{uid}/{container}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn append(&self, path: &Path, lines: &[&str]) {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        for (i, text) in lines.iter().enumerate() {
            writeln!(file, "{} stdout F {text}", time(i as u64)).unwrap();
        }
    }

    fn link(&self, ns: &str, pod: &str, uid: &str, container: &str, restart: u32, id: &str) {
        std::os::unix::fs::symlink(
            format!("/var/log/pods/{ns}_{pod}_{uid}/{container}/{restart}.log"),
            self.root
                .join(format!("containers/{pod}_{ns}_{container}-{id}.log")),
        )
        .unwrap();
    }

    fn config(&self) -> PodLogsConfig {
        let mut config =
            PodLogsConfig::new(self.root.join("pods"), "amedes".into(), "cluster-1".into());
        config.cluster_name = Some("k-lab".into());
        config.start_at_end = false;
        config
    }
}

fn time(offset_ms: u64) -> String {
    chrono::DateTime::from_timestamp_nanos((T0 + offset_ms * 1_000_000) as i64)
        .to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)
}

/// Resource attributes as (key, value) text.
type Resource = Vec<(String, String)>;

/// (resource attributes, bodies) per resource of an export.
fn decoded(export: &[u8]) -> Vec<(Resource, Vec<String>)> {
    all(export, 1)
        .iter()
        .map(|resource_logs| {
            let resource = attributes(&one(resource_logs, 1), 1);
            let bodies = all(&one(resource_logs, 2), 2)
                .iter()
                .map(|record| String::from_utf8(one(&one(record, 5), 1)).unwrap())
                .collect();
            (resource, bodies)
        })
        .collect()
}

fn bodies(batch: &Batch) -> Vec<String> {
    decoded(&batch.export)
        .into_iter()
        .flat_map(|(_, bodies)| bodies)
        .collect()
}

fn attribute<'a>(resource: &'a [(String, String)], key: &str) -> Option<&'a str> {
    resource
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// Polls and acknowledges until nothing is new; every body sent.
fn drain(logs: &mut PodLogs) -> Vec<String> {
    let mut sent = Vec::new();
    while let Some(batch) = logs.poll(NOW) {
        sent.extend(bodies(&batch));
        logs.commit(&batch, false);
    }
    sent
}

#[test]
fn a_container_file_is_sent_under_the_identity_the_k8s_muse_uses() {
    let node = Node::new();
    let dir = node.container_dir("shop", "web-7d9f8b6c5d-x2x4z", "uid-1", "app");
    node.append(&dir.join("0.log"), &["ERROR db down", "ready"]);
    let id = "c0ffee".repeat(10) + "abcd";
    node.link("shop", "web-7d9f8b6c5d-x2x4z", "uid-1", "app", 0, &id);
    let mut logs = PodLogs::new(node.config(), None);
    let batch = logs.poll(NOW).unwrap();
    let resources = decoded(&batch.export);
    assert_eq!(resources.len(), 1);
    let (resource, sent) = &resources[0];
    assert_eq!(sent, &["ERROR db down", "ready"]);
    for (key, value) in [
        ("service.name", "web"),
        ("k8s.cluster.uid", "cluster-1"),
        ("k8s.cluster.name", "k-lab"),
        ("k8s.namespace.name", "shop"),
        ("k8s.pod.name", "web-7d9f8b6c5d-x2x4z"),
        ("k8s.pod.uid", "uid-1"),
        ("k8s.container.name", "app"),
        ("k8s.container.restart_count", "0"),
        ("container.id", id.as_str()),
        ("k8s.node.name", "amedes"),
    ] {
        assert_eq!(attribute(resource, key), Some(value), "{key}");
    }
    let records = all(&one(&all(&batch.export, 1)[0], 2), 2);
    assert_eq!(scalar(&records[0], 1), Some(T0), "the runtime's time");
    assert_eq!(scalar(&records[0], 11), Some(NOW), "observed now");
    assert_eq!(scalar(&records[0], 2), Some(17), "ERROR");
    assert_eq!(scalar(&records[1], 2), None, "no level stated");
    assert_eq!(
        attributes(&records[0], 6),
        [
            ("log.iostream".to_string(), "stdout".to_string()),
            ("log.file.name".to_string(), "0.log".to_string())
        ]
    );
    logs.commit(&batch, false);
    assert_eq!(logs.stats().records_sent, 2);
    assert!(logs.poll(NOW).is_none(), "nothing new");
    node.append(&dir.join("0.log"), &["later"]);
    assert_eq!(drain(&mut logs), ["later"]);
}

#[test]
fn an_unacknowledged_export_is_read_again_and_nothing_more_is_read() {
    let node = Node::new();
    let dir = node.container_dir("shop", "web-0", "uid-1", "app");
    node.append(&dir.join("0.log"), &["one", "two"]);
    let mut logs = PodLogs::new(node.config(), None);
    let first = logs.poll(NOW).unwrap();
    // The Poet is busy (429): no commit. The same lines come again.
    node.append(&dir.join("0.log"), &["three"]);
    let again = logs.poll(NOW).unwrap();
    assert_eq!(bodies(&first), ["one", "two"]);
    assert_eq!(bodies(&again), ["one", "two", "three"]);
    logs.commit(&again, false);
    assert!(logs.poll(NOW).is_none());
}

#[test]
fn rotation_reads_the_rest_of_the_renamed_file_then_the_new_one() {
    let node = Node::new();
    let dir = node.container_dir("shop", "web-0", "uid-1", "app");
    let live = dir.join("0.log");
    node.append(&live, &["a", "b"]);
    let mut logs = PodLogs::new(node.config(), None);
    assert_eq!(drain(&mut logs), ["a", "b"]);
    node.append(&live, &["c"]);
    // kubelet renames the live file and the runtime opens a new one.
    std::fs::rename(&live, dir.join("0.log.20261016-075100")).unwrap();
    node.append(&live, &["d"]);
    assert_eq!(drain(&mut logs), ["c", "d"]);
    // The rotated file is compressed (renamed .gz) and the old deleted.
    std::fs::remove_file(dir.join("0.log.20261016-075100")).unwrap();
    node.append(&live, &["e"]);
    assert_eq!(drain(&mut logs), ["e"]);
    assert_eq!(logs.stats().bytes_lost, 0);
}

#[test]
fn a_file_deleted_before_it_was_read_counts_its_lost_bytes() {
    let node = Node::new();
    let dir = node.container_dir("shop", "web-0", "uid-1", "app");
    let live = dir.join("0.log");
    node.append(&live, &["a"]);
    let mut logs = PodLogs::new(node.config(), None);
    let batch = logs.poll(NOW).unwrap();
    // Not acknowledged, and kubelet deletes the file.
    let size = std::fs::metadata(&live).unwrap().len();
    std::fs::remove_file(&live).unwrap();
    assert!(logs.poll(NOW).is_none());
    logs.commit(&batch, false);
    assert_eq!(logs.stats().bytes_lost, size);
}

#[test]
fn a_container_restart_is_its_own_resource_with_its_own_id() {
    let node = Node::new();
    let dir = node.container_dir("shop", "web-0", "uid-1", "app");
    node.append(&dir.join("0.log"), &["first life"]);
    let mut logs = PodLogs::new(node.config(), None);
    drain(&mut logs);
    node.append(&dir.join("1.log"), &["second life"]);
    node.link("shop", "web-0", "uid-1", "app", 1, &"1d".repeat(32));
    let batch = logs.poll(NOW).unwrap();
    let resources = decoded(&batch.export);
    assert_eq!(resources.len(), 1);
    assert_eq!(resources[0].1, ["second life"]);
    assert_eq!(
        attribute(&resources[0].0, "k8s.container.restart_count"),
        Some("1")
    );
    assert_eq!(
        attribute(&resources[0].0, "container.id"),
        Some("1d".repeat(32).as_str())
    );
    assert_eq!(attribute(&resources[0].0, "service.name"), Some("web"));
}

#[test]
fn a_checkpoint_resumes_without_repeats_and_a_first_start_begins_at_the_end() {
    let node = Node::new();
    let dir = node.container_dir("shop", "web-0", "uid-1", "app");
    node.append(&dir.join("0.log"), &["old"]);
    // First start, from the end: what was there is not sent.
    let mut config = node.config();
    config.start_at_end = true;
    let mut logs = PodLogs::new(config.clone(), None);
    assert!(logs.poll(NOW).is_none());
    node.append(&dir.join("0.log"), &["new"]);
    assert_eq!(drain(&mut logs), ["new"]);
    // A file created after the first scan is read from its start.
    let other = node.container_dir("shop", "web-1", "uid-2", "app");
    node.append(&other.join("0.log"), &["born later"]);
    assert_eq!(drain(&mut logs), ["born later"]);
    let saved = node.root.join("pod-logs.json");
    logs.checkpoint().save(&saved).unwrap();
    // Restart: lines written while it was down, nothing twice.
    node.append(&dir.join("0.log"), &["while down"]);
    let mut restarted = PodLogs::new(config, Checkpoint::load(&saved));
    assert_eq!(drain(&mut restarted), ["while down"]);
}

#[test]
fn a_truncated_or_replaced_file_is_read_again_from_its_start() {
    let node = Node::new();
    let dir = node.container_dir("shop", "web-0", "uid-1", "app");
    let live = dir.join("0.log");
    node.append(&live, &["a long first line", "and a second"]);
    let mut logs = PodLogs::new(node.config(), None);
    drain(&mut logs);
    // Truncated in place (copytruncate): shorter than the offset.
    std::fs::write(&live, b"").unwrap();
    node.append(&live, &["fresh"]);
    assert_eq!(drain(&mut logs), ["fresh"]);
    assert_eq!(logs.stats().restarts, 1);
    // Rewritten in place with other bytes, longer than before (an inode
    // reused by another file): its start differs.
    std::fs::write(&live, b"").unwrap();
    node.append(&live, &["different start", "x", "y"]);
    assert_eq!(drain(&mut logs), ["different start", "x", "y"]);
    assert_eq!(logs.stats().restarts, 2);
}

#[test]
fn a_budget_bounds_each_export_and_every_file_gets_its_turn() {
    let node = Node::new();
    let lines: Vec<String> = (0..50)
        .map(|i| format!("line {i:02} {}", "x".repeat(60)))
        .collect();
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    for pod in ["a-0", "b-0", "c-0"] {
        let dir = node.container_dir("shop", pod, &format!("uid-{pod}"), "app");
        node.append(&dir.join("0.log"), &refs);
    }
    let mut config = node.config();
    config.max_batch_bytes = 2_000;
    config.max_file_bytes = 1_000;
    let mut logs = PodLogs::new(config, None);
    let mut per_pod: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    let mut exports = 0;
    let mut first_pods = Vec::new();
    while let Some(batch) = logs.poll(NOW) {
        assert!(batch.bytes_read <= 2_000, "{}", batch.bytes_read);
        exports += 1;
        for (resource, sent) in decoded(&batch.export) {
            let pod = attribute(&resource, "k8s.pod.name").unwrap().to_string();
            if exports <= 3 {
                first_pods.push(pod.clone());
            }
            per_pod.entry(pod).or_default().extend(sent);
        }
        logs.commit(&batch, false);
    }
    assert!(exports > 5, "{exports}");
    assert!(
        ["a-0", "b-0", "c-0"]
            .iter()
            .all(|pod| first_pods.contains(&pod.to_string())),
        "every pod is read early: {first_pods:?}"
    );
    for sent in per_pod.values() {
        assert_eq!(sent, &lines, "in order, each once");
    }
}

#[test]
fn filters_include_and_exclude_namespaces_and_pods() {
    let node = Node::new();
    for (ns, pod) in [
        ("shop", "web-0"),
        ("shop", "debug-0"),
        ("kube-system", "dns-0"),
        ("ih", "poet-0"),
    ] {
        let dir = node.container_dir(ns, pod, &format!("uid-{ns}-{pod}"), "app");
        node.append(&dir.join("0.log"), &[&format!("{ns}/{pod}")]);
    }
    let run = |filter: Filter| {
        let mut config = node.config();
        config.filter = filter;
        let mut sent = drain(&mut PodLogs::new(config, None));
        sent.sort();
        sent
    };
    assert_eq!(run(Filter::default()).len(), 4);
    assert_eq!(
        run(Filter {
            exclude_namespaces: vec!["kube-*".into()],
            exclude_pods: vec!["shop/debug-*".into()],
            ..Filter::default()
        }),
        ["ih/poet-0", "shop/web-0"]
    );
    assert_eq!(
        run(Filter {
            include_namespaces: vec!["shop".into()],
            include_pods: vec!["ih/poet-?".into()],
            ..Filter::default()
        }),
        ["ih/poet-0", "shop/debug-0", "shop/web-0"]
    );
}

#[test]
fn a_refused_export_moves_on_and_is_counted() {
    let node = Node::new();
    let dir = node.container_dir("shop", "web-0", "uid-1", "app");
    node.append(&dir.join("0.log"), &["bad"]);
    let mut logs = PodLogs::new(node.config(), None);
    let batch = logs.poll(NOW).unwrap();
    logs.commit(&batch, true);
    assert!(logs.poll(NOW).is_none());
    assert_eq!(logs.stats().exports_refused, 1);
    assert_eq!(logs.stats().records_sent, 0);
}

#[test]
fn workloads_are_read_from_generated_pod_names() {
    assert_eq!(
        workload_of("ih-next-kabuki-6c9f7b8d5f-k2x7q"),
        "ih-next-kabuki"
    );
    assert_eq!(
        workload_of("ih-next-kernel-muse-q8z2w"),
        "ih-next-kernel-muse"
    );
    assert_eq!(workload_of("ih-next-poet-2"), "ih-next-poet");
    assert_eq!(workload_of("standalone"), "standalone");
    assert_eq!(workload_of("my-app"), "my-app", "not a generated suffix");
}

#[test]
fn globs_match_like_a_shell() {
    assert!(glob("kube-*", "kube-system"));
    assert!(glob("*", ""));
    assert!(glob("a*b*c", "axxbyyc"));
    assert!(glob("poet-?", "poet-1"));
    assert!(!glob("poet-?", "poet-10"));
    assert!(!glob("kube-*", "ih-kube"));
}
