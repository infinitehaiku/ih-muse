//! The kernel-faults Muse's dashboard, defined in code.
//!
//! It travels to Poet in `GraphBatch.dashboards` until a Poet acknowledges
//! a batch that carried it (`ih_muse::dashboards::DashboardDelivery`). Every
//! panel names a metric this Muse sends (the `*_METRIC` constants), so a
//! renamed metric breaks the build or the tests, not the dashboard.

use ih_muse_proto::dashboard::{
    GoldenSignal, PanelAggregation, PanelGroupBy, PanelKind, PanelSize, PanelSpec, PanelStream,
};
use ih_muse_proto::{DashboardAppliesTo, DashboardBlock, DashboardDefinition};

use crate::{
    CPU_ATTRIBUTE, CPU_FAULTS_METRIC, FAULTS_METRIC, KIND_ATTRIBUTE, MUSE_KIND, SKIPPED_METRIC,
};

/// Id of the kernel-faults dashboard.
pub const FAULTS_DASHBOARD_ID: &str = "kernel.faults";

/// Every dashboard this Muse defines.
pub fn dashboard_definitions() -> Vec<DashboardDefinition> {
    vec![faults_dashboard()]
}

/// Kernel faults: over time per node and by kind, the CPUs that fault
/// most, and the newest fault lines as written by the kernel.
pub fn faults_dashboard() -> DashboardDefinition {
    DashboardDefinition {
        id: FAULTS_DASHBOARD_ID.into(),
        revision: 1,
        title: "Kernel faults".into(),
        description: "Segfaults, general protection faults, machine-check and EDAC errors, I/O and filesystem errors and OOM kills the kernel logged on each node since boot, per kind and per CPU, with the lines themselves.".into(),
        muse_kind: MUSE_KIND.into(),
        muse_versions: Some(concat!(">=", env!("CARGO_PKG_VERSION")).into()),
        // The batches' root is the cluster (`cluster -> node`), which Poet
        // names by the identity kind `kubernetes`.
        applies_to: DashboardAppliesTo::MuseRoots {
            entity_kinds: Vec::new(),
            identity_kinds: vec!["kubernetes".into()],
        },
        panels: vec![
            panel("faults_per_node", "Faults per second, per node", FAULTS_METRIC, PanelAggregation::Rate)
                .signal(GoldenSignal::Errors)
                .group(PanelGroupBy::Entity),
            panel("faults_by_kind", "Faults per second, by kind", FAULTS_METRIC, PanelAggregation::Rate)
                .group(PanelGroupBy::Attribute(KIND_ATTRIBUTE.into())),
            panel("faults_per_cpu", "Faults per second, per CPU", CPU_FAULTS_METRIC, PanelAggregation::Rate)
                .group(PanelGroupBy::Attribute(CPU_ATTRIBUTE.into())),
            panel("top_cpus", "CPUs with the most faults since boot", CPU_FAULTS_METRIC, PanelAggregation::Last)
                .kind(PanelKind::TopList)
                .group(PanelGroupBy::Attribute(CPU_ATTRIBUTE.into()))
                .top(16)
                .size(6, 5),
            panel("totals_by_kind", "Faults since boot, by kind", FAULTS_METRIC, PanelAggregation::Last)
                .kind(PanelKind::TopList)
                .group(PanelGroupBy::Attribute(KIND_ATTRIBUTE.into()))
                .size(6, 5),
            PanelSpec {
                id: "latest_lines".into(),
                title: "Latest fault lines".into(),
                kind: PanelKind::Events,
                stream: Some(PanelStream {
                    search: None,
                    errors_only: false,
                    limit: Some(20),
                }),
                ..Default::default()
            }
            .size(12, 5),
            panel("skipped", "Fault lines that could not be read", SKIPPED_METRIC, PanelAggregation::Last)
                .group(PanelGroupBy::Attribute(KIND_ATTRIBUTE.into())),
        ],
        blocks: vec![
            block("Over time", "Faults per second on each node, by kind and per logical CPU. Nothing is drawn when the kernel logged no fault.", &["faults_per_node", "faults_by_kind", "faults_per_cpu"]),
            block("Since boot", "Totals of the current boot: a CPU that faults far more than the others points at hardware.", &["top_cpus", "totals_by_kind"]),
            block("Lines", "The kernel's own lines, newest first.", &["latest_lines", "skipped"]),
        ],
        columns: Some(3),
    }
}

fn block(label: &str, text: &str, panels: &[&str]) -> DashboardBlock {
    DashboardBlock {
        label: label.into(),
        text: (!text.is_empty()).then(|| text.into()),
        panels: panels.iter().map(|panel| panel.to_string()).collect(),
        ..Default::default()
    }
}

fn panel(id: &str, title: &str, metric: &str, aggregation: PanelAggregation) -> PanelSpec {
    PanelSpec {
        id: id.into(),
        title: title.into(),
        metric: metric.into(),
        aggregation,
        ..Default::default()
    }
}

/// Small builder steps that keep the panel list readable.
trait PanelSpecExt {
    fn signal(self, signal: GoldenSignal) -> Self;
    fn group(self, group: PanelGroupBy) -> Self;
    fn top(self, top_n: usize) -> Self;
    fn kind(self, kind: PanelKind) -> Self;
    fn size(self, w: u8, h: u8) -> Self;
}

impl PanelSpecExt for PanelSpec {
    fn signal(mut self, signal: GoldenSignal) -> Self {
        self.signal = Some(signal);
        self
    }

    fn group(mut self, group: PanelGroupBy) -> Self {
        self.group_by = Some(group);
        self
    }

    fn top(mut self, top_n: usize) -> Self {
        self.top_n = Some(top_n);
        self
    }

    fn kind(mut self, kind: PanelKind) -> Self {
        self.kind = kind;
        self
    }

    fn size(mut self, w: u8, h: u8) -> Self {
        self.size = Some(PanelSize { w, h });
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::METRICS;

    #[test]
    fn the_definition_is_valid_and_reads_only_metrics_this_muse_sends() {
        let definition = faults_dashboard();
        definition.validate().unwrap();
        ih_muse::dashboards::check_definitions(&dashboard_definitions()).unwrap();
        for panel in &definition.panels {
            assert!(
                panel.metric.is_empty() || METRICS.contains(&panel.metric.as_str()),
                "{} reads {}",
                panel.id,
                panel.metric
            );
        }
        // Every panel is drawn in a block.
        let listed: Vec<&String> = definition.blocks.iter().flat_map(|b| &b.panels).collect();
        for panel in &definition.panels {
            assert!(listed.contains(&&panel.id), "{} is in no block", panel.id);
        }
        assert!(definition.id.starts_with(&format!("{MUSE_KIND}.")));
    }
}
