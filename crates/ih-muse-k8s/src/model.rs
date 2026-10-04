//! The parts of Kubernetes API objects the Muse reads.
//!
//! Every field is optional or defaulted: a missing field becomes the same
//! default the reference Python Muse (`ih-infra/scripts/k8s_muse.py`) uses,
//! so an unusual object never stops a collection.

use std::collections::BTreeMap;

use serde::Deserialize;

/// A `*List` answer (`/api/v1/nodes`, `/api/v1/namespaces/{ns}/pods`, ...).
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct List<T> {
    #[serde(default = "Vec::new")]
    pub items: Vec<T>,
}

/// Name, UID, namespace, labels and owners of any object.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ObjectMeta {
    pub name: String,
    pub uid: String,
    /// Empty for cluster-scoped objects (nodes, namespaces).
    pub namespace: String,
    pub creation_timestamp: Option<String>,
    /// `metadata.generation`: bumped by every spec change of a workload.
    pub generation: i64,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub owner_references: Vec<OwnerReference>,
}

/// One `metadata.ownerReferences` entry (a ReplicaSet's Deployment).
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct OwnerReference {
    pub kind: String,
    pub name: String,
    pub uid: String,
}

/// A `Namespace`; only its UID is read (the namespace entity is keyed by it).
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Namespace {
    pub metadata: ObjectMeta,
}

/// A `Node`: name, UID, allocatable resources and platform.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Node {
    pub metadata: ObjectMeta,
    pub status: NodeStatus,
}

/// The read part of `Node.status`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct NodeStatus {
    /// Quantities as the API writes them (`"4"`, `"7834060Ki"`).
    pub allocatable: BTreeMap<String, String>,
    pub node_info: NodeInfo,
    /// `Ready`, `MemoryPressure`, `DiskPressure`, `PIDPressure`, ...
    pub conditions: Vec<Condition>,
}

/// A status condition of a node, pod or workload, with when it last changed.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Condition {
    #[serde(rename = "type")]
    pub kind: String,
    /// `True`, `False` or `Unknown`.
    pub status: String,
    pub reason: Option<String>,
    pub message: Option<String>,
    pub last_transition_time: Option<String>,
    /// Deployments' `Progressing` condition moves this, not the transition.
    pub last_update_time: Option<String>,
}

/// The read part of `Node.status.nodeInfo`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct NodeInfo {
    pub architecture: String,
    pub operating_system: String,
}

/// A `Pod`: identity, node and status.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Pod {
    pub metadata: ObjectMeta,
    pub spec: PodSpec,
    pub status: PodStatus,
}

/// The read part of `Pod.spec`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct PodSpec {
    pub node_name: Option<String>,
    pub containers: Vec<ContainerSpec>,
}

/// One `Pod.spec.containers` entry: its image and resources.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ContainerSpec {
    pub name: String,
    pub image: String,
    pub resources: Resources,
}

/// A container's `requests` and `limits` (quantities as the API writes them).
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Resources {
    pub requests: BTreeMap<String, String>,
    pub limits: BTreeMap<String, String>,
}

/// The read part of `Pod.status`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct PodStatus {
    pub phase: Option<String>,
    #[serde(rename = "podIP")]
    pub pod_ip: Option<String>,
    pub start_time: Option<String>,
    pub conditions: Vec<Condition>,
    pub container_statuses: Vec<ContainerStatus>,
}

/// `Pod.status.conditions` entries share the node conditions' shape.
pub type PodCondition = Condition;

/// One `Pod.status.containerStatuses` entry.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ContainerStatus {
    pub name: String,
    #[serde(rename = "containerID")]
    pub container_id: Option<String>,
    pub ready: bool,
    pub restart_count: i64,
    /// `{"running": {...}}`, `{"waiting": {...}}` or `{"terminated": {...}}`.
    pub state: Option<serde_json::Map<String, serde_json::Value>>,
    /// The state before the last restart (`{"terminated": {"reason": ...}}`).
    pub last_state: Option<serde_json::Map<String, serde_json::Value>>,
    /// The image as the spec names it (`redis:7`).
    pub image: String,
    /// The resolved image with its digest (`docker.io/library/redis@sha256:...`).
    #[serde(rename = "imageID")]
    pub image_id: String,
}

impl ContainerStatus {
    /// `state.waiting.reason` (`CrashLoopBackOff`), if waiting.
    pub fn waiting_reason(&self) -> Option<&str> {
        state_field(self.state.as_ref(), "waiting", "reason")
    }

    /// `state.waiting.message`, if waiting.
    pub fn waiting_message(&self) -> Option<&str> {
        state_field(self.state.as_ref(), "waiting", "message")
    }

    /// The last termination (`lastState.terminated`): reason, exit code and
    /// when it ended.
    pub fn last_termination(&self) -> Option<Termination> {
        let terminated = self.last_state.as_ref()?.get("terminated")?.as_object()?;
        let text = |name: &str| {
            terminated
                .get(name)
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        };
        Some(Termination {
            reason: text("reason"),
            exit_code: terminated
                .get("exitCode")
                .and_then(serde_json::Value::as_i64),
            finished_at: text("finishedAt"),
        })
    }
}

/// How a container last ended (`lastState.terminated`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Termination {
    /// `OOMKilled`, `Error`, `Completed`, ...
    pub reason: Option<String>,
    pub exit_code: Option<i64>,
    pub finished_at: Option<String>,
}

fn state_field<'a>(
    state: Option<&'a serde_json::Map<String, serde_json::Value>>,
    phase: &str,
    field: &str,
) -> Option<&'a str> {
    state?.get(phase)?.get(field)?.as_str()
}

/// An `apps/v1` `Deployment`: desired replicas, its pod template and rollout
/// status.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Deployment {
    pub metadata: ObjectMeta,
    pub spec: WorkloadSpec,
    pub status: WorkloadStatus,
}

/// An `apps/v1` `StatefulSet` (same read shape as a Deployment).
pub type StatefulSet = Deployment;

/// An `apps/v1` `ReplicaSet`: the pod template of one Deployment revision.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ReplicaSet {
    pub metadata: ObjectMeta,
    pub spec: WorkloadSpec,
}

/// The read part of a workload's spec. The template stays raw JSON: the
/// Muse compares templates, it does not interpret every field.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct WorkloadSpec {
    /// `None` means the API default, 1.
    pub replicas: Option<i64>,
    pub template: serde_json::Value,
}

/// The read part of a Deployment's or StatefulSet's status.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct WorkloadStatus {
    pub observed_generation: i64,
    pub replicas: i64,
    pub updated_replicas: i64,
    pub ready_replicas: i64,
    pub available_replicas: i64,
    /// StatefulSet only: the revision the pods run and the one being rolled out.
    pub current_revision: Option<String>,
    pub update_revision: Option<String>,
    pub conditions: Vec<Condition>,
}

/// A core `v1` `Event` (the Events API), read for `type=Warning`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct KubeEvent {
    pub metadata: ObjectMeta,
    pub involved_object: ObjectReference,
    pub reason: String,
    pub message: String,
    /// `Warning` or `Normal`.
    #[serde(rename = "type")]
    pub kind: String,
    pub count: Option<i64>,
    pub first_timestamp: Option<String>,
    pub last_timestamp: Option<String>,
    /// Set instead of the timestamps by `events.k8s.io` writers.
    pub event_time: Option<String>,
    pub series: Option<EventSeries>,
}

/// What a Kubernetes Event is about.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ObjectReference {
    pub kind: String,
    pub namespace: String,
    pub name: String,
    pub uid: String,
}

/// A repeated Kubernetes Event (`series`).
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct EventSeries {
    pub count: i64,
    pub last_observed_time: Option<String>,
}

/// One `metrics.k8s.io/v1beta1` `NodeMetrics` item.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct NodeMetrics {
    pub metadata: ObjectMeta,
    /// `cpu` and `memory` quantities.
    pub usage: BTreeMap<String, String>,
}

/// One `metrics.k8s.io/v1beta1` `PodMetrics` item.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct PodMetrics {
    pub metadata: ObjectMeta,
    pub containers: Vec<ContainerMetrics>,
}

/// One container of a `PodMetrics` item.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ContainerMetrics {
    pub name: String,
    pub usage: BTreeMap<String, String>,
}

/// Parses a Kubernetes quantity (`500m`, `236528738n`, `1275996Ki`, `40Mi`)
/// exactly as the reference Python Muse does: binary suffixes first, then
/// decimal ones, and 0 for anything it cannot read. Exponent suffixes
/// (`E`, `P`, `Ei`, `Pi`) are not handled there either and read as 0.
pub fn parse_quantity(text: &str) -> f64 {
    const SUFFIXES: [(&str, f64); 11] = [
        ("Ki", 1024.0),
        ("Mi", 1024.0 * 1024.0),
        ("Gi", 1024.0 * 1024.0 * 1024.0),
        ("Ti", 1024.0 * 1024.0 * 1024.0 * 1024.0),
        ("n", 1e-9),
        ("u", 1e-6),
        ("m", 1e-3),
        ("k", 1e3),
        ("M", 1e6),
        ("G", 1e9),
        ("T", 1e12),
    ];
    let text = text.trim();
    if text.is_empty() {
        return 0.0;
    }
    let (number, factor) = SUFFIXES
        .iter()
        .find_map(|(suffix, factor)| text.strip_suffix(suffix).map(|number| (number, *factor)))
        .unwrap_or((text, 1.0));
    match number.trim().parse::<f64>() {
        // A non-finite value cannot travel as JSON; treat it as unreadable.
        Ok(value) if (value * factor).is_finite() => value * factor,
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantities_parse_like_the_python_muse() {
        assert_eq!(parse_quantity("500m"), 500.0 * 1e-3);
        assert_eq!(parse_quantity("236528738n"), 236_528_738.0 * 1e-9);
        assert_eq!(parse_quantity("1275996Ki"), 1_275_996.0 * 1024.0);
        assert_eq!(parse_quantity("40Mi"), 40.0 * 1024.0 * 1024.0);
        assert_eq!(parse_quantity("2"), 2.0);
        assert_eq!(parse_quantity("1.5G"), 1.5e9);
        assert_eq!(parse_quantity(" 3k "), 3e3);
        assert_eq!(parse_quantity(""), 0.0);
        assert_eq!(parse_quantity("abc"), 0.0);
        assert_eq!(
            parse_quantity("1Ei"),
            0.0,
            "exponent suffixes are unreadable, as in Python"
        );
    }

    #[test]
    fn objects_parse_with_defaults_for_missing_fields() {
        let pod: Pod = serde_json::from_str(
            r#"{"metadata":{"name":"p","uid":"u"},"status":{"podIP":"10.0.0.9",
                "containerStatuses":[{"name":"c","containerID":"containerd://abc","restartCount":3,
                "state":{"waiting":{"reason":"CrashLoopBackOff"}}}]}}"#,
        )
        .unwrap();
        assert_eq!(pod.metadata.name, "p");
        assert_eq!(pod.status.pod_ip.as_deref(), Some("10.0.0.9"));
        assert_eq!(pod.status.phase, None);
        assert_eq!(pod.spec.node_name, None);
        let container = &pod.status.container_statuses[0];
        assert_eq!(container.restart_count, 3);
        assert!(!container.ready);
        assert!(container.state.as_ref().unwrap().contains_key("waiting"));
        let empty: List<Node> =
            serde_json::from_str(r#"{"kind":"NodeList","metadata":{}}"#).unwrap();
        assert!(empty.items.is_empty());
    }
}
