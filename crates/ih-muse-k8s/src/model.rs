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

/// Name, UID and creation time of any object.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ObjectMeta {
    pub name: String,
    pub uid: String,
    pub creation_timestamp: Option<String>,
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
}

/// The read part of `Pod.status`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct PodStatus {
    pub phase: Option<String>,
    #[serde(rename = "podIP")]
    pub pod_ip: Option<String>,
    pub start_time: Option<String>,
    pub conditions: Vec<PodCondition>,
    pub container_statuses: Vec<ContainerStatus>,
}

/// One `Pod.status.conditions` entry (`Ready`, `True`).
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct PodCondition {
    #[serde(rename = "type")]
    pub kind: String,
    pub status: String,
}

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
