//! The Rust Muse against recorded Kubernetes API answers (no cluster, no
//! network beyond a loopback test server).
//!
//! `fixtures/expected-*.json` are the batches the reference Python Muse
//! (`ih-infra/scripts/k8s_muse.py`, `collect_k8s_graph`) builds from the same
//! fixtures; `fixtures/gen_expected.py` regenerates them.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use ih_muse_k8s::api::{ApiConfig, KubeApi};
use ih_muse_k8s::dashboards::dashboard_definitions;
use ih_muse_k8s::graph::{collect_graph, GraphConfig, Snapshot, LEVEL_ATTRIBUTE, METRICS};
use ih_muse_k8s::identity::MuseIdentity;
use ih_muse_k8s::model::{List, Namespace, Node, NodeMetrics, Pod, PodMetrics};
use ih_muse_k8s::K8sMuse;
use ih_muse_proto::{AttributeValue, GraphBatch};
use serde_json::Value;

const NOW: u64 = 1_791_108_000_000_000_000; // 2026-10-04T10:00:00Z, as in gen_expected.py
const NAMESPACE: &str = "infinite-haiku-p2";
const CLUSTER: &str = "5f51b1b5-8445-47c0-a9c7-222bb2c92398";

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(fixture_path(name)).unwrap()
}

fn parse<T: serde::de::DeserializeOwned>(name: &str) -> T {
    serde_json::from_str(&fixture(name)).unwrap_or_else(|error| panic!("{name}: {error}"))
}

fn snapshot(metrics: bool, namespace: bool) -> Snapshot {
    Snapshot {
        nodes: parse::<List<Node>>("nodes.json").items,
        node_metrics: metrics.then(|| parse::<List<NodeMetrics>>("node-metrics.json").items),
        pods: parse::<List<Pod>>("pods.json").items,
        pod_metrics: metrics.then(|| parse::<List<PodMetrics>>("pod-metrics.json").items),
        namespace_uid: namespace.then(|| parse::<Namespace>("namespace.json").metadata.uid),
    }
}

/// The Python Muse's configuration and provenance, for byte-level parity.
fn python_config() -> GraphConfig {
    GraphConfig {
        organization: "ih-local".into(),
        cluster_uid: CLUSTER.into(),
        namespace: NAMESPACE.into(),
        expected_interval_ns: 5_000_000_000,
        source_id: "ih-k8s-muse".into(),
        source_revision: "v2".into(),
    }
}

/// Structural JSON equality (floats within 1e-12 relative); returns the
/// path of the first difference.
fn diff(path: &str, actual: &Value, expected: &Value) -> Option<String> {
    match (actual, expected) {
        (Value::Object(a), Value::Object(e)) => {
            let keys: BTreeSet<&String> = a.keys().chain(e.keys()).collect();
            keys.into_iter()
                .find_map(|key| match (a.get(key), e.get(key)) {
                    (Some(x), Some(y)) => diff(&format!("{path}.{key}"), x, y),
                    (x, y) => Some(format!("{path}.{key}: {x:?} != {y:?}")),
                })
        }
        (Value::Array(a), Value::Array(e)) => {
            if a.len() != e.len() {
                return Some(format!("{path}: length {} != {}", a.len(), e.len()));
            }
            a.iter()
                .zip(e)
                .enumerate()
                .find_map(|(i, (x, y))| diff(&format!("{path}[{i}]"), x, y))
        }
        (Value::Number(a), Value::Number(e)) if a.is_f64() || e.is_f64() => {
            let (a, e) = (a.as_f64().unwrap(), e.as_f64().unwrap());
            ((a - e).abs() > 1e-12 * e.abs().max(1.0)).then(|| format!("{path}: {a} != {e}"))
        }
        _ => (actual != expected).then(|| format!("{path}: {actual} != {expected}")),
    }
}

fn assert_matches_python(batch: &GraphBatch, expected_file: &str) {
    batch.validate().expect("the batch is a valid graph");
    let actual = serde_json::to_value(batch).unwrap();
    let expected: Value = serde_json::from_str(&fixture(expected_file)).unwrap();
    if let Some(difference) = diff("batch", &actual, &expected) {
        panic!("differs from {expected_file} at {difference}");
    }
}

#[test]
fn fixtures_parse() {
    let snapshot = snapshot(true, true);
    assert_eq!(snapshot.nodes.len(), 2);
    assert_eq!(snapshot.nodes[0].status.node_info.architecture, "arm64");
    assert_eq!(snapshot.nodes[1].status.allocatable["memory"], "3.5Gi");
    assert_eq!(snapshot.pods.len(), 5);
    assert_eq!(
        snapshot.pods[1].status.container_statuses[1].restart_count,
        5
    );
    assert_eq!(snapshot.pods[2].spec.node_name, None);
    assert_eq!(
        snapshot.node_metrics.as_ref().unwrap()[0].usage["cpu"],
        "236528738n"
    );
    assert_eq!(
        snapshot.pod_metrics.as_ref().unwrap()[1].containers.len(),
        2
    );
    assert_eq!(
        snapshot.namespace_uid.as_deref(),
        Some("9a8b7c6d-1111-4222-8333-444455556666")
    );
}

#[test]
fn the_graph_matches_what_the_python_muse_emits() {
    assert_matches_python(
        &collect_graph(&snapshot(true, true), &python_config(), NOW),
        "expected-full.json",
    );
}

#[test]
fn without_metrics_server_or_namespace_uid_the_graph_matches_python_too() {
    let batch = collect_graph(&snapshot(false, false), &python_config(), NOW);
    assert_matches_python(&batch, "expected-no-metrics.json");
    assert!(
        batch.observations.iter().all(|observation| {
            observation.descriptor.name.ends_with(".restarts")
                || observation.descriptor.name.starts_with("k8s.pod.")
                || matches!(
                    observation.value,
                    ih_muse_proto::MetricObservation::Missing { .. }
                )
        }),
        "usage without metrics-server is a gap, never 0"
    );
}

#[test]
fn every_dashboard_panel_metric_and_level_is_emitted() {
    let batch = collect_graph(&snapshot(true, true), &python_config(), NOW);
    let mut levels: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    for observation in &batch.observations {
        let level = match observation.attributes.get(LEVEL_ATTRIBUTE) {
            Some(AttributeValue::String(level)) => level.clone(),
            _ => String::new(),
        };
        levels
            .entry(observation.descriptor.name.as_str())
            .or_default()
            .insert(level);
    }
    assert_eq!(
        levels.keys().copied().collect::<BTreeSet<_>>(),
        METRICS.into_iter().collect()
    );
    for definition in dashboard_definitions() {
        for panel in &definition.panels {
            let emitted = levels.get(panel.metric.as_str()).unwrap_or_else(|| {
                panic!(
                    "panel {} names {}, which the Muse does not emit",
                    panel.id, panel.metric
                )
            });
            for filter in panel
                .filters
                .iter()
                .filter(|filter| filter.key == LEVEL_ATTRIBUTE)
            {
                let level = filter.value.as_str().unwrap();
                assert!(
                    emitted.contains(level),
                    "panel {} filters on level {level}, never emitted",
                    panel.id
                );
            }
        }
    }
}

fn muse() -> K8sMuse {
    K8sMuse::new(
        MuseIdentity::new(Some("k-lab"), "10.43.0.1"),
        "ih-local",
        CLUSTER,
        NAMESPACE,
        5_000_000_000,
    )
    .unwrap()
}

#[test]
fn the_first_batch_carries_the_definition_until_a_poet_acknowledges_it() {
    let mut muse = muse();
    let snapshot = snapshot(true, true);
    let first = muse.intake(&snapshot, NOW);
    first.validate().expect("intake request is valid");
    assert_eq!(first.batch.dashboards, dashboard_definitions());
    assert_eq!(first.delivery_id, format!("ih-k8s-muse:k-lab:{NOW}"));
    let provenance = &first.batch.observations[0].provenance;
    assert_eq!(provenance.source_id, "ih-k8s-muse:k-lab");
    assert_eq!(provenance.source_revision, env!("CARGO_PKG_VERSION"));

    // Unacknowledged (no Poet yet): the next batch carries it too.
    let second = muse.intake(&snapshot, NOW + 5_000_000_000);
    assert_eq!(second.batch.dashboards, first.batch.dashboards);
    muse.acknowledge(&second);
    let later = muse.intake(&snapshot, NOW + 10_000_000_000);
    assert!(
        later.batch.dashboards.is_empty(),
        "sent until acknowledged, then left out"
    );
    assert!(!serde_json::to_string(&later)
        .unwrap()
        .contains("\"dashboards\""));
}

#[tokio::test]
async fn queued_batches_survive_an_unavailable_poet_and_a_rejection_is_dropped() {
    use ih_muse_core::MuseError;
    let mut muse = muse();
    let snapshot = snapshot(true, true);
    muse.enqueue(muse.intake(&snapshot, NOW));
    muse.enqueue(muse.intake(&snapshot, NOW + 1));
    let down = muse
        .send_pending(|_| async { Err(MuseError::Unavailable("down".into())) })
        .await;
    assert!(down.is_err());
    assert_eq!(muse.pending(), 2, "nothing lost while no Poet answers");
    assert!(!muse.intake(&snapshot, NOW + 2).batch.dashboards.is_empty());

    let rejected = muse
        .send_pending(|_| async { Err(MuseError::Validation("HTTP 400".into())) })
        .await;
    assert!(rejected.is_err());
    assert_eq!(muse.pending(), 1, "a rejected batch is not retried forever");

    let sent = muse.send_pending(|request| async move {
        assert!(!request.batch.dashboards.is_empty());
        Ok(())
    });
    assert_eq!(sent.await.unwrap(), 1);
    assert_eq!(muse.pending(), 0);
    assert!(
        muse.intake(&snapshot, NOW + 3).batch.dashboards.is_empty(),
        "acknowledged once accepted"
    );
    for _ in 0..(ih_muse_k8s::MAX_PENDING_BATCHES + 3) {
        muse.enqueue(muse.intake(&snapshot, NOW));
    }
    assert_eq!(muse.pending(), ih_muse_k8s::MAX_PENDING_BATCHES);
    assert_eq!(muse.dropped(), 4, "one rejected and three over the bound");
}

/// A loopback HTTP server that answers the Muse's API paths from the
/// fixtures (503 for metrics when `metrics` is false) and records the
/// Authorization headers it saw.
async fn api_server(
    metrics: bool,
) -> (String, Arc<std::sync::Mutex<Vec<String>>>, Arc<AtomicUsize>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let auth = Arc::new(std::sync::Mutex::new(Vec::new()));
    let hits = Arc::new(AtomicUsize::new(0));
    let (seen, counter) = (auth.clone(), hits.clone());
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            counter.fetch_add(1, Ordering::SeqCst);
            let mut buffer = vec![0_u8; 16 * 1024];
            let read = socket.read(&mut buffer).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            let path = request.split_whitespace().nth(1).unwrap_or("").to_string();
            if let Some(line) = request
                .lines()
                .find(|line| line.to_ascii_lowercase().starts_with("authorization:"))
            {
                seen.lock().unwrap().push(line.to_string());
            }
            let file = match path.as_str() {
                "/api/v1/nodes" => Some("nodes.json"),
                "/apis/metrics.k8s.io/v1beta1/nodes" if metrics => Some("node-metrics.json"),
                "/api/v1/namespaces/infinite-haiku-p2/pods" => Some("pods.json"),
                "/apis/metrics.k8s.io/v1beta1/namespaces/infinite-haiku-p2/pods" if metrics => {
                    Some("pod-metrics.json")
                }
                "/api/v1/namespaces/infinite-haiku-p2" => Some("namespace.json"),
                _ => None,
            };
            let reply = match file {
                Some(file) => {
                    let body = fixture(file);
                    format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                }
                None => "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
            };
            let _ = socket.write_all(reply.as_bytes()).await;
        }
    });
    (format!("http://{address}"), auth, hits)
}

/// A token file in a per-test directory removed on drop (pass, fail, panic).
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "ih-muse-k8s-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if std::env::var_os("KEEP_TEST_ARTIFACTS").is_none() {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[tokio::test]
async fn the_api_client_reads_the_recorded_answers_and_builds_the_same_graph() {
    let (url, auth, _) = api_server(true).await;
    let scratch = Scratch::new();
    let token = scratch.0.join("token");
    std::fs::write(&token, "sa-token-1\n").unwrap();
    let config = ApiConfig::resolve(Some(&url), Some(&token), None, |_| None).unwrap();
    let api = KubeApi::new(config).unwrap();
    let mut muse = K8sMuse::new(
        MuseIdentity::new(None, "127.0.0.1"),
        "ih-local",
        CLUSTER,
        NAMESPACE,
        5_000_000_000,
    )
    .unwrap();
    let warnings = std::sync::Mutex::new(Vec::new());
    let read = muse
        .collect(&api, |message| warnings.lock().unwrap().push(message))
        .await
        .unwrap();
    assert_eq!(read, snapshot(true, true));
    assert!(warnings.lock().unwrap().is_empty());
    assert!(auth
        .lock()
        .unwrap()
        .iter()
        .all(|line| line.ends_with("Bearer sa-token-1")));

    // A rotated token is read on the next request.
    std::fs::write(&token, "sa-token-2").unwrap();
    muse.collect(&api, |_| {}).await.unwrap();
    assert!(auth
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .ends_with("Bearer sa-token-2"));

    let mut config = python_config();
    config.source_id = muse.graph_config().source_id.clone();
    config.source_revision = muse.graph_config().source_revision.clone();
    assert_eq!(muse.graph_config(), &config);
    assert_eq!(muse.graph_config().source_id, "ih-k8s-muse:127.0.0.1");
}

#[tokio::test]
async fn metrics_server_down_gives_gaps_and_warnings_not_a_failed_collection() {
    let (url, _, _) = api_server(false).await;
    let api = KubeApi::new(ApiConfig::resolve(Some(&url), None, None, |_| None).unwrap()).unwrap();
    let mut muse = muse();
    let warnings = std::sync::Mutex::new(Vec::new());
    let read = muse
        .collect(&api, |message| warnings.lock().unwrap().push(message))
        .await
        .unwrap();
    assert_eq!(read, snapshot(false, true));
    let warnings = warnings.into_inner().unwrap();
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(
        warnings.iter().all(|warning| warning.contains("HTTP 503")),
        "{warnings:?}"
    );
}
