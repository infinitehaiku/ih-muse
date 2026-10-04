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
    CONTAINER_RESTARTS_METRIC, CPU_USAGE_METRIC, LEVEL_ATTRIBUTE, MEMORY_USAGE_METRIC,
    POD_NOT_READY_METRIC, POD_RUNNING_METRIC,
};
use crate::identity::MUSE_KIND;

/// Id of the cluster dashboard.
pub const CLUSTER_DASHBOARD_ID: &str = "k8s.cluster";

/// Every default dashboard this Muse defines.
pub fn dashboard_definitions() -> Vec<DashboardDefinition> {
    vec![cluster_dashboard()]
}

/// The cluster: running and not-ready pods, restarts, and the CPU and
/// memory of containers and nodes. Same content as
/// `examples/dashboards/k8s-cluster.json`.
pub fn cluster_dashboard() -> DashboardDefinition {
    DashboardDefinition {
        id: CLUSTER_DASHBOARD_ID.into(),
        revision: 1,
        title: "Kubernetes cluster".into(),
        description: "Running pods, pods not ready, container restarts and the CPU and memory of containers and nodes, observed by the Kubernetes Muse.".into(),
        muse_kind: MUSE_KIND.into(),
        muse_versions: Some(concat!(">=", env!("CARGO_PKG_VERSION")).into()),
        applies_to: DashboardAppliesTo::MuseRoots {
            entity_kinds: Vec::new(),
            identity_kinds: vec!["cluster".into()],
        },
        panels: vec![
            panel("pods_running", "Running pods", POD_RUNNING_METRIC, PanelAggregation::Sum)
                .signal(GoldenSignal::Traffic),
            panel("pods_not_ready", "Pods not ready", POD_NOT_READY_METRIC, PanelAggregation::Sum)
                .signal(GoldenSignal::Errors)
                .thresholds(1.0, 3.0),
            panel("restarts", "Container restarts", CONTAINER_RESTARTS_METRIC, PanelAggregation::Rate)
                .signal(GoldenSignal::Errors)
                .top(8),
            panel("cpu", "Container CPU (cores)", CPU_USAGE_METRIC, PanelAggregation::Avg)
                .signal(GoldenSignal::Saturation)
                .level("container")
                .top(8),
            panel("memory", "Container memory", MEMORY_USAGE_METRIC, PanelAggregation::Avg)
                .level("container")
                .top(8),
            panel("node_cpu", "Node CPU (cores)", CPU_USAGE_METRIC, PanelAggregation::Avg)
                .level("node")
                .top(8),
        ],
        blocks: vec![DashboardBlock {
            label: "Cluster: what uses it".into(),
            text: Some("The 8 containers that use the most memory and the 8 busiest nodes.".into()),
            panels: vec!["memory".into(), "node_cpu".into()],
        }],
        columns: Some(3),
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
