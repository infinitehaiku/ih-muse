//! The Kubernetes Muse's default dashboard, defined in code.
//!
//! It travels to Poet in `GraphBatch.dashboards` until a Poet acknowledges a
//! batch that carried it (`ih_muse::dashboards::DashboardDelivery`). Poet
//! stores it as data and Kabuki renders it. Every panel names a metric this
//! Muse emits (the `*_METRIC` constants), so a renamed metric breaks the
//! build or the tests, not the dashboard.

use ih_muse_proto::dashboard::{
    FilterOp, GoldenSignal, PanelAggregation, PanelFilter, PanelGroupBy, PanelSpec, PanelThresholds,
};
use ih_muse_proto::{DashboardAppliesTo, DashboardBlock, DashboardDefinition};

use crate::graph::{
    CONTAINER_CPU_LIMIT_UTILIZATION_METRIC, CONTAINER_MEMORY_LIMIT_UTILIZATION_METRIC,
    CONTAINER_OOM_KILLS_METRIC, CONTAINER_RESTARTS_METRIC, CONTAINER_WAITING_METRIC,
    CPU_USAGE_METRIC, DEPLOYMENT_AVAILABLE_METRIC, DEPLOYMENT_DESIRED_METRIC, LEVEL_ATTRIBUTE,
    MEMORY_USAGE_METRIC, NODE_CONDITION_METRIC, NODE_CPU_UTILIZATION_METRIC,
    NODE_MEMORY_REQUEST_UTILIZATION_METRIC, NODE_MEMORY_UTILIZATION_METRIC, POD_NOT_READY_METRIC,
    POD_RUNNING_METRIC, POD_UNSCHEDULABLE_METRIC, STATEFULSET_READY_METRIC,
    WAITING_REASON_ATTRIBUTE,
};
use crate::identity::MUSE_KIND;

/// Id of the cluster dashboard.
pub const CLUSTER_DASHBOARD_ID: &str = "k8s.cluster";

/// Every default dashboard this Muse defines.
pub fn dashboard_definitions() -> Vec<DashboardDefinition> {
    vec![cluster_dashboard()]
}

/// The cluster, revision 3 (k8s and Redis metrics research, 2026-10-04):
/// a health row of 0/1 flags summed (nodes not ready or under pressure,
/// pods not ready, containers crash-looping or waiting by reason, pending
/// pods), then stability (restarts, OOMKills), saturation as % of limits
/// and allocatable, workloads (desired vs available) and raw usage. The
/// kubelet panels (CPU throttling, working set, volumes) wait for phase B.
/// Same content as `examples/dashboards/k8s-cluster.json`.
pub fn cluster_dashboard() -> DashboardDefinition {
    DashboardDefinition {
        id: CLUSTER_DASHBOARD_ID.into(),
        // 2 named the identity kind `cluster`, which never matched (k-lab
        // branch Poet only); 3 is the first that applies.
        revision: 3,
        title: "Kubernetes cluster".into(),
        description: "Nodes, pods and containers that are unhealthy now, restarts and OOMKills, memory and CPU against limits and allocatable, workloads and usage, observed by the Kubernetes Muse in every namespace.".into(),
        muse_kind: MUSE_KIND.into(),
        muse_versions: Some(concat!(">=", env!("CARGO_PKG_VERSION")).into()),
        // Poet names a cluster root's identity kind `kubernetes` (as for
        // every Kubernetes element); `cluster` never matched, so Poets
        // answered with their built-in profile (k-lab, 2026-10-04).
        applies_to: DashboardAppliesTo::MuseRoots {
            entity_kinds: Vec::new(),
            identity_kinds: vec!["kubernetes".into()],
        },
        panels: vec![
            panel("pods_not_ready", "Pods not ready", POD_NOT_READY_METRIC, PanelAggregation::Sum)
                .signal(GoldenSignal::Errors)
                .thresholds(1.0, 3.0),
            panel("nodes_not_ready", "Nodes not ready", NODE_CONDITION_METRIC, PanelAggregation::Sum)
                .signal(GoldenSignal::Errors)
                .condition("Ready", FilterOp::Eq, "false")
                .thresholds(1.0, 1.0),
            panel("nodes_pressure", "Nodes under memory, disk or PID pressure", NODE_CONDITION_METRIC, PanelAggregation::Sum)
                .signal(GoldenSignal::Errors)
                .condition("Ready", FilterOp::Ne, "true")
                .thresholds(1.0, 1.0),
            panel("waiting", "Crash-looping or waiting containers", CONTAINER_WAITING_METRIC, PanelAggregation::Sum)
                .signal(GoldenSignal::Errors)
                .by(WAITING_REASON_ATTRIBUTE)
                .thresholds(1.0, 3.0),
            panel("pods_pending", "Pending pods (unschedulable)", POD_UNSCHEDULABLE_METRIC, PanelAggregation::Sum)
                .signal(GoldenSignal::Errors)
                .thresholds(1.0, 3.0),
            panel("restarts", "Container restarts", CONTAINER_RESTARTS_METRIC, PanelAggregation::Rate)
                .signal(GoldenSignal::Errors)
                .top(8),
            panel("oom_kills", "OOMKills", CONTAINER_OOM_KILLS_METRIC, PanelAggregation::Rate)
                .signal(GoldenSignal::Errors)
                .top(8),
            panel("memory_limit", "Memory % of limit", CONTAINER_MEMORY_LIMIT_UTILIZATION_METRIC, PanelAggregation::Max)
                .signal(GoldenSignal::Saturation)
                .top(8)
                .thresholds(0.8, 0.9),
            panel("cpu_limit", "CPU % of limit", CONTAINER_CPU_LIMIT_UTILIZATION_METRIC, PanelAggregation::Max)
                .top(8)
                .thresholds(0.8, 0.9),
            panel("node_memory_utilization", "Node memory % of allocatable", NODE_MEMORY_UTILIZATION_METRIC, PanelAggregation::Max)
                .top(8)
                .thresholds(0.8, 0.9),
            panel("node_cpu_utilization", "Node CPU % of allocatable", NODE_CPU_UTILIZATION_METRIC, PanelAggregation::Max)
                .top(8)
                .thresholds(0.8, 0.9),
            panel("node_memory_requests", "Node memory requested % of allocatable", NODE_MEMORY_REQUEST_UTILIZATION_METRIC, PanelAggregation::Max)
                .top(8)
                .thresholds(0.9, 1.0),
            panel("deployments_desired", "Deployment pods desired", DEPLOYMENT_DESIRED_METRIC, PanelAggregation::Last)
                .top(8),
            panel("deployments_available", "Deployment pods available", DEPLOYMENT_AVAILABLE_METRIC, PanelAggregation::Last)
                .top(8),
            panel("statefulsets_ready", "StatefulSet pods ready", STATEFULSET_READY_METRIC, PanelAggregation::Last)
                .top(8),
            panel("pods_running", "Running pods", POD_RUNNING_METRIC, PanelAggregation::Sum)
                .signal(GoldenSignal::Traffic),
            panel("cpu", "Container CPU (cores)", CPU_USAGE_METRIC, PanelAggregation::Avg)
                .level("container")
                .top(8),
            panel("memory", "Container memory", MEMORY_USAGE_METRIC, PanelAggregation::Avg)
                .level("container")
                .top(8),
            panel("node_cpu", "Node CPU (cores)", CPU_USAGE_METRIC, PanelAggregation::Avg)
                .level("node")
                .top(8),
        ],
        blocks: vec![
            block("Health now", "Counts of nodes, pods and containers that are not healthy, in every namespace.", &["pods_not_ready", "nodes_not_ready", "nodes_pressure", "waiting", "pods_pending"]),
            block("Stability", "Restarts and OOMKills per second, the 8 worst containers.", &["restarts", "oom_kills"]),
            block("Saturation", "Usage against limits and allocatable: 100 % of a memory limit is an OOMKill.", &["memory_limit", "cpu_limit", "node_memory_utilization", "node_cpu_utilization", "node_memory_requests"]),
            block("Workloads", "Desired against available or ready pods.", &["deployments_desired", "deployments_available", "statefulsets_ready"]),
            block("Usage", "Running pods, and the cores and bytes the busiest containers and nodes use.", &["pods_running", "cpu", "memory", "node_cpu"]),
        ],
        columns: Some(3),
    }
}

fn block(label: &str, text: &str, panels: &[&str]) -> DashboardBlock {
    DashboardBlock {
        label: label.into(),
        text: Some(text.into()),
        panels: panels.iter().map(|panel| panel.to_string()).collect(),
    }
}

fn panel(id: &str, title: &str, metric: &str, aggregation: PanelAggregation) -> PanelSpec {
    PanelSpec {
        id: id.into(),
        title: title.into(),
        metric: metric.into(),
        aggregation,
        filters: Vec::new(),
        group_by: None,
        top_n: None,
        signal: None,
        thresholds: None,
    }
}

/// Small builder steps that keep the panel list above readable.
trait PanelSpecExt {
    fn signal(self, signal: GoldenSignal) -> Self;
    fn thresholds(self, warning: f64, critical: f64) -> Self;
    fn level(self, level: &str) -> Self;
    fn top(self, top_n: usize) -> Self;
    fn by(self, attribute: &str) -> Self;
    fn condition(self, kind: &str, op: FilterOp, status: &str) -> Self;
}

impl PanelSpecExt for PanelSpec {
    fn signal(mut self, signal: GoldenSignal) -> Self {
        self.signal = Some(signal);
        self
    }

    fn thresholds(mut self, warning: f64, critical: f64) -> Self {
        self.thresholds = Some(PanelThresholds {
            warning,
            critical,
            higher_is_worse: true,
        });
        self
    }

    /// Only the usage values of one `k8s.level` (node, pod or container).
    fn level(mut self, level: &str) -> Self {
        self.filters.push(PanelFilter {
            key: LEVEL_ATTRIBUTE.into(),
            op: FilterOp::Eq,
            value: serde_json::Value::String(level.into()),
        });
        self
    }

    /// One series per value of an observation attribute (a reason).
    fn by(mut self, attribute: &str) -> Self {
        self.group_by = Some(PanelGroupBy::Attribute(attribute.into()));
        self
    }

    /// Node conditions: `Ready` itself (`Eq`) or every other type (`Ne`),
    /// with the given status.
    fn condition(mut self, kind: &str, op: FilterOp, status: &str) -> Self {
        self.filters.push(PanelFilter {
            key: "k8s.node.condition.type".into(),
            op,
            value: serde_json::Value::String(kind.into()),
        });
        // `Ne Ready` counts the pressure conditions that are true.
        let status = if op == FilterOp::Ne { "true" } else { status };
        self.filters.push(PanelFilter {
            key: "k8s.node.condition.status".into(),
            op: FilterOp::Eq,
            value: serde_json::Value::String(status.into()),
        });
        self
    }

    /// One series per entity, the top `top_n`.
    fn top(mut self, top_n: usize) -> Self {
        self.group_by = Some(PanelGroupBy::Entity);
        self.top_n = Some(top_n);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = include_str!("../../../examples/dashboards/k8s-cluster.json");

    #[test]
    fn the_definition_validates_and_is_deliverable() {
        let definitions = dashboard_definitions();
        ih_muse::dashboards::check_definitions(&definitions).unwrap();
        for definition in &definitions {
            assert_eq!(definition.muse_kind, MUSE_KIND);
            assert!(definition.id.starts_with("k8s."));
        }
        ih_muse::dashboards::DashboardDelivery::new(definitions).unwrap();
    }

    /// The code-built definition equals the SDK's reference JSON apart from
    /// the version requirement only the Muse knows. Compared as typed values,
    /// since serde defaults may be written or omitted.
    #[test]
    #[ignore = "prints the definition for examples/dashboards/k8s-cluster.json"]
    fn print_example() {
        let mut definition = cluster_dashboard();
        definition.muse_versions = None;
        println!("{}", serde_json::to_string_pretty(&definition).unwrap());
    }

    #[test]
    fn cluster_dashboard_matches_the_sdk_example() {
        let mut example: DashboardDefinition = serde_json::from_str(EXAMPLE).unwrap();
        assert_eq!(example.muse_versions, None);
        example.muse_versions = Some(concat!(">=", env!("CARGO_PKG_VERSION")).into());
        assert_eq!(cluster_dashboard(), example);
        let round_trip: DashboardDefinition =
            serde_json::from_str(&serde_json::to_string(&cluster_dashboard()).unwrap()).unwrap();
        assert_eq!(round_trip, cluster_dashboard());
    }
}
