//! The Pods and Nodes overviews (WP k8s-pack): the Kubernetes Muse's
//! metrics laid out as Datadog's "Kubernetes Pods Overview" and "Nodes
//! Overview" read, with the panel kinds of the dashboard contract (text,
//! stat, donut, top list, table, logs, traces) on banded sections.
//!
//! Only metrics this Muse sends are drawn. What those overviews show and
//! this Muse does not send (network, disk I/O, images, owner kinds, pod
//! phases other than Running, pods per node, events as a series) is left
//! out, never drawn empty (`target/dogfood-fixes/k8s-pack/RESULTS.md`
//! lists the gaps). Ratios are lines with threshold lines, never stacked
//! and never a donut.

use ih_muse_proto::dashboard::{
    FilterOp, GoldenSignal, PanelAggregation, PanelKind, PanelSpec, PanelStream, SectionColor,
};
use ih_muse_proto::{DashboardAppliesTo, DashboardBlock, DashboardDefinition};

use super::{block, panel, PanelSpecExt};
use crate::graph::{
    CONTAINER_CPU_LIMIT_METRIC, CONTAINER_CPU_LIMIT_UTILIZATION_METRIC,
    CONTAINER_CPU_REQUEST_METRIC, CONTAINER_MEMORY_LIMIT_METRIC,
    CONTAINER_MEMORY_LIMIT_UTILIZATION_METRIC, CONTAINER_MEMORY_REQUEST_METRIC,
    CONTAINER_RESTARTS_METRIC, CONTAINER_WAITING_METRIC, CPU_USAGE_METRIC,
    DEPLOYMENT_AVAILABLE_METRIC, DEPLOYMENT_DESIRED_METRIC, LEVEL_ATTRIBUTE, MEMORY_USAGE_METRIC,
    NODE_CONDITION_METRIC, NODE_CPU_ALLOCATABLE_METRIC, NODE_CPU_REQUEST_UTILIZATION_METRIC,
    NODE_CPU_UTILIZATION_METRIC, NODE_MEMORY_ALLOCATABLE_METRIC,
    NODE_MEMORY_REQUEST_UTILIZATION_METRIC, NODE_MEMORY_UTILIZATION_METRIC, POD_NOT_READY_METRIC,
    POD_RUNNING_METRIC, POD_UNSCHEDULABLE_METRIC, STATEFULSET_DESIRED_METRIC,
    STATEFULSET_READY_METRIC, WAITING_REASON_ATTRIBUTE,
};
use crate::identity::MUSE_KIND;

/// Id of the Pods overview.
pub const PODS_DASHBOARD_ID: &str = "k8s.pods";
/// Id of the Nodes overview.
pub const NODES_DASHBOARD_ID: &str = "k8s.nodes";

/// Rows of a per-node panel: every node of any cluster this Muse watches
/// (the most series a board offers, so no node is cut).
const EVERY_NODE: usize = 500;
/// Rows of a "busiest" table or list.
const BUSIEST: usize = 10;

/// Applies to the cluster root, as the cluster dashboard. The root's
/// `ih.dashboard.profile` still names the cluster dashboard, so these two
/// are opened from the dashboard library (Duplicate on the k-lab source).
fn cluster_roots() -> DashboardAppliesTo {
    DashboardAppliesTo::MuseRoots {
        entity_kinds: Vec::new(),
        identity_kinds: vec!["kubernetes".into()],
    }
}

fn definition(
    id: &str,
    title: &str,
    description: &str,
    panels: Vec<PanelSpec>,
    blocks: Vec<DashboardBlock>,
) -> DashboardDefinition {
    DashboardDefinition {
        id: id.into(),
        revision: 1,
        title: title.into(),
        description: description.into(),
        muse_kind: MUSE_KIND.into(),
        muse_versions: Some(concat!(">=", env!("CARGO_PKG_VERSION")).into()),
        applies_to: cluster_roots(),
        panels,
        blocks,
        columns: None,
    }
}

/// A section with a colour band and a share of the page's width.
fn section(
    label: &str,
    text: &str,
    color: SectionColor,
    width: u8,
    panels: &[&str],
) -> DashboardBlock {
    DashboardBlock {
        color: Some(color),
        width: (width < 12).then_some(width),
        ..block(label, text, panels)
    }
}

/// A Markdown panel.
fn text(id: &str, title: &str, markdown: &str) -> PanelSpec {
    PanelSpec {
        id: id.into(),
        title: title.into(),
        kind: PanelKind::Text,
        text: Some(markdown.into()),
        ..Default::default()
    }
}

/// A logs or traces panel of the cluster (its lines and spans, and those
/// of every element under it).
fn stream(id: &str, title: &str, kind: PanelKind, errors_only: bool, limit: u32) -> PanelSpec {
    PanelSpec {
        id: id.into(),
        title: title.into(),
        kind,
        stream: Some(PanelStream {
            search: None,
            errors_only,
            limit: Some(limit),
        }),
        ..Default::default()
    }
}

/// The Pods overview (Datadog's "Kubernetes Pods Overview").
pub fn pods_dashboard() -> DashboardDefinition {
    let pods = |id: &str, title: &str, metric: &str, aggregation| {
        panel(id, title, metric, aggregation).level("pod")
    };
    let panels = vec![
        text(
            "intro",
            "About this overview",
            "**Pods in every namespace**, as the Kubernetes Muse reads them from the API and metrics-server.\n\n\
             - *Pod health*: running, not ready and unschedulable pods, and why containers wait.\n\
             - *Resource utilization*: the cores and bytes pods use, and the containers closest to their limits.\n\
             - *Logs and traces*: errors and failed traces of everything in the cluster.\n\n\
             Network, disk I/O, images and owner kinds are not shown: the Muse does not send them yet.",
        )
        .size(12, 2),
        // Pod health
        panel("running", "Running pods", POD_RUNNING_METRIC, PanelAggregation::Sum)
            .kind(PanelKind::Stat)
            .signal(GoldenSignal::Traffic)
            .size(4, 2),
        panel("not_ready", "Pods not ready", POD_NOT_READY_METRIC, PanelAggregation::Sum)
            .kind(PanelKind::Stat)
            .signal(GoldenSignal::Errors)
            .thresholds(1.0, 3.0)
            .size(4, 2),
        panel("unschedulable", "Pods pending (unschedulable)", POD_UNSCHEDULABLE_METRIC, PanelAggregation::Sum)
            .kind(PanelKind::Stat)
            .signal(GoldenSignal::Errors)
            .thresholds(1.0, 3.0)
            .size(4, 2),
        panel("running_over_time", "Running pods over time", POD_RUNNING_METRIC, PanelAggregation::Sum)
            .size(6, 3),
        panel("not_ready_over_time", "Pods not ready over time", POD_NOT_READY_METRIC, PanelAggregation::Sum)
            .thresholds(1.0, 3.0)
            .size(6, 3),
        // Only the observations that carry a reason: the healthy ones
        // (value 0, no reason) are not a slice.
        panel("waiting_by_reason", "Waiting containers by reason", CONTAINER_WAITING_METRIC, PanelAggregation::Sum)
            .kind(PanelKind::Donut)
            .by(WAITING_REASON_ATTRIBUTE)
            .filter(WAITING_REASON_ATTRIBUTE, FilterOp::Prefix, "")
            .size(12, 4),
        // Resource utilization
        pods("pod_cpu", "Pod CPU (cores)", CPU_USAGE_METRIC, PanelAggregation::Avg)
            .top(8)
            .signal(GoldenSignal::Saturation)
            .size(12, 3),
        pods("pod_memory", "Pod memory", MEMORY_USAGE_METRIC, PanelAggregation::Avg)
            .top(8)
            .signal(GoldenSignal::Saturation)
            .size(12, 3),
        panel("container_memory_limit", "Container memory % of limit", CONTAINER_MEMORY_LIMIT_UTILIZATION_METRIC, PanelAggregation::Max)
            .top(8)
            .thresholds(0.8, 1.0)
            .size(12, 3),
        // Busiest pods and containers
        pods("cpu_pods", "CPU-intensive pods", CPU_USAGE_METRIC, PanelAggregation::Last)
            .kind(PanelKind::Table)
            .top(BUSIEST)
            .column("Memory", MEMORY_USAGE_METRIC, PanelAggregation::Last, &[(LEVEL_ATTRIBUTE, "pod")])
            .column("Running", POD_RUNNING_METRIC, PanelAggregation::Last, &[])
            .column("Not ready", POD_NOT_READY_METRIC, PanelAggregation::Last, &[])
            .size(6, 5),
        pods("memory_pods", "Memory-intensive pods", MEMORY_USAGE_METRIC, PanelAggregation::Last)
            .kind(PanelKind::Table)
            .top(BUSIEST)
            .column("CPU (cores)", CPU_USAGE_METRIC, PanelAggregation::Last, &[(LEVEL_ATTRIBUTE, "pod")])
            .column("Running", POD_RUNNING_METRIC, PanelAggregation::Last, &[])
            .column("Not ready", POD_NOT_READY_METRIC, PanelAggregation::Last, &[])
            .size(6, 5),
        // Rows are containers by name; same-named containers of different
        // pods share a row, so every column is the worst (Max) of them.
        panel("cpu_limits", "Containers closest to their CPU limit", CONTAINER_CPU_LIMIT_UTILIZATION_METRIC, PanelAggregation::Max)
            .kind(PanelKind::Table)
            .top(BUSIEST)
            .thresholds(0.8, 1.0)
            .column("CPU (cores)", CPU_USAGE_METRIC, PanelAggregation::Max, &[(LEVEL_ATTRIBUTE, "container")])
            .column("Limit (cores)", CONTAINER_CPU_LIMIT_METRIC, PanelAggregation::Max, &[])
            .column("Request (cores)", CONTAINER_CPU_REQUEST_METRIC, PanelAggregation::Max, &[])
            .column("Restarts", CONTAINER_RESTARTS_METRIC, PanelAggregation::Max, &[])
            .size(6, 5),
        panel("memory_limits", "Containers closest to their memory limit", CONTAINER_MEMORY_LIMIT_UTILIZATION_METRIC, PanelAggregation::Max)
            .kind(PanelKind::Table)
            .top(BUSIEST)
            .thresholds(0.8, 1.0)
            .column("Memory", MEMORY_USAGE_METRIC, PanelAggregation::Max, &[(LEVEL_ATTRIBUTE, "container")])
            .column("Limit", CONTAINER_MEMORY_LIMIT_METRIC, PanelAggregation::Max, &[])
            .column("Request", CONTAINER_MEMORY_REQUEST_METRIC, PanelAggregation::Max, &[])
            .column("Restarts", CONTAINER_RESTARTS_METRIC, PanelAggregation::Max, &[])
            .size(6, 5),
        // Inventory and attention
        panel("attention", "Pods requiring attention", POD_NOT_READY_METRIC, PanelAggregation::Last)
            .kind(PanelKind::Table)
            .top(BUSIEST)
            .column("Unschedulable", POD_UNSCHEDULABLE_METRIC, PanelAggregation::Last, &[])
            .column("Running", POD_RUNNING_METRIC, PanelAggregation::Last, &[])
            .column("CPU (cores)", CPU_USAGE_METRIC, PanelAggregation::Last, &[(LEVEL_ATTRIBUTE, "pod")])
            .size(6, 5),
        panel("restarts", "Most restarted containers", CONTAINER_RESTARTS_METRIC, PanelAggregation::Max)
            .kind(PanelKind::TopList)
            .top(BUSIEST)
            .size(6, 5),
        panel("deployments", "Deployments: desired pods", DEPLOYMENT_DESIRED_METRIC, PanelAggregation::Last)
            .kind(PanelKind::Table)
            .top(BUSIEST)
            .column("Available", DEPLOYMENT_AVAILABLE_METRIC, PanelAggregation::Last, &[])
            .size(6, 4),
        panel("statefulsets", "StatefulSets: desired pods", STATEFULSET_DESIRED_METRIC, PanelAggregation::Last)
            .kind(PanelKind::Table)
            .top(BUSIEST)
            .column("Ready", STATEFULSET_READY_METRIC, PanelAggregation::Last, &[])
            .size(6, 4),
        // Logs and traces
        stream("errors", "Errors logged in the cluster", PanelKind::Logs, true, 30).size(6, 5),
        stream("failed_traces", "Failed traces in the cluster", PanelKind::Traces, true, 20).size(6, 5),
    ];
    let blocks = vec![
        section("Overview", "", SectionColor::Gray, 12, &["intro"]),
        section(
            "Pod health",
            "Pods running, not ready and unschedulable now and over the window; why containers wait.",
            SectionColor::Green,
            6,
            &["running", "not_ready", "unschedulable", "running_over_time", "not_ready_over_time", "waiting_by_reason"],
        ),
        section(
            "Resource utilization",
            "Cores and bytes the 8 busiest pods use, and memory against limits (100 % of a memory limit is an OOMKill).",
            SectionColor::Blue,
            6,
            &["pod_cpu", "pod_memory", "container_memory_limit"],
        ),
        section(
            "Busiest pods and containers",
            "The 10 busiest pods, and the containers closest to their limits. Container rows are by container name: same-named containers of different pods share a row and show the worst of them.",
            SectionColor::Purple,
            12,
            &["cpu_pods", "memory_pods", "cpu_limits", "memory_limits"],
        ),
        section(
            "Inventory and attention",
            "Pods not ready first, restarts since each pod started, and each workload's desired against available or ready pods.",
            SectionColor::Teal,
            12,
            &["attention", "restarts", "deployments", "statefulsets"],
        ),
        section(
            "Logs and traces",
            "Errors logged and failed traces of everything in the cluster, newest first.",
            SectionColor::Orange,
            12,
            &["errors", "failed_traces"],
        ),
    ];
    definition(
        PODS_DASHBOARD_ID,
        "Kubernetes pods overview",
        "Pods in every namespace: running, not ready and unschedulable pods, why containers wait, the busiest pods and the containers closest to their limits, restarts, workloads, and the cluster's errors and failed traces.",
        panels,
        blocks,
    )
}

/// The Nodes overview (Datadog's "Kubernetes Nodes Overview").
pub fn nodes_dashboard() -> DashboardDefinition {
    // The Ready condition rows whose status compares to `status`.
    let ready = |spec: PanelSpec, op: FilterOp, status: &str| {
        spec.filter("k8s.node.condition.type", FilterOp::Eq, "Ready")
            .filter("k8s.node.condition.status", op, status)
    };
    let panels = vec![
        text(
            "intro",
            "About this overview",
            "**Every node of the cluster**: how many are ready, what they can give pods, and how full they are.\n\n\
             Memory and CPU are shown as % of allocatable with lines at 80 % and 100 %. \
             Pods per node and events per node are not shown: the Muse does not send them as metrics yet \
             (Kubernetes events appear as markers on every graph).",
        )
        .size(12, 2),
        // KPIs. Each node reports one current status per condition type,
        // so the Ready rows count nodes.
        panel("nodes", "Nodes", NODE_CONDITION_METRIC, PanelAggregation::Sum)
            .filter("k8s.node.condition.type", FilterOp::Eq, "Ready")
            .kind(PanelKind::Stat)
            .size(2, 2),
        panel("cpu_total", "Total CPU (cores)", NODE_CPU_ALLOCATABLE_METRIC, PanelAggregation::Sum)
            .kind(PanelKind::Stat)
            .size(2, 2),
        panel("memory_total", "Total memory", NODE_MEMORY_ALLOCATABLE_METRIC, PanelAggregation::Sum)
            .kind(PanelKind::Stat)
            .size(2, 2),
        ready(panel("not_ready", "Nodes not ready", NODE_CONDITION_METRIC, PanelAggregation::Sum), FilterOp::Ne, "true")
            .kind(PanelKind::Stat)
            .signal(GoldenSignal::Errors)
            .thresholds(1.0, 1.0)
            .size(2, 2),
        ready(panel("kubelets_up", "Kubelets up (node Ready)", NODE_CONDITION_METRIC, PanelAggregation::Sum), FilterOp::Eq, "true")
            .kind(PanelKind::Stat)
            .size(2, 2),
        panel("pressure", "Nodes under pressure", NODE_CONDITION_METRIC, PanelAggregation::Sum)
            .condition("Ready", FilterOp::Ne, "true")
            .kind(PanelKind::Stat)
            .signal(GoldenSignal::Errors)
            .thresholds(1.0, 1.0)
            .size(2, 2),
        // Utilization: lines per node, never stacked.
        panel("memory_utilization", "Memory % of allocatable per node", NODE_MEMORY_UTILIZATION_METRIC, PanelAggregation::Max)
            .top(EVERY_NODE)
            .signal(GoldenSignal::Saturation)
            .thresholds(0.8, 1.0)
            .size(6, 4),
        panel("cpu_utilization", "CPU % of allocatable per node", NODE_CPU_UTILIZATION_METRIC, PanelAggregation::Max)
            .top(EVERY_NODE)
            .signal(GoldenSignal::Saturation)
            .thresholds(0.8, 1.0)
            .size(6, 4),
        panel("memory_requests", "Memory requested % of allocatable per node", NODE_MEMORY_REQUEST_UTILIZATION_METRIC, PanelAggregation::Max)
            .top(EVERY_NODE)
            .thresholds(0.8, 1.0)
            .size(6, 3),
        panel("cpu_requests", "CPU requested % of allocatable per node", NODE_CPU_REQUEST_UTILIZATION_METRIC, PanelAggregation::Max)
            .top(EVERY_NODE)
            .thresholds(0.8, 1.0)
            .size(6, 3),
        // Conditions: 1 where the condition holds now.
        ready(panel("conditions", "Node conditions", NODE_CONDITION_METRIC, PanelAggregation::Last), FilterOp::Eq, "true")
            .kind(PanelKind::Table)
            .top(EVERY_NODE)
            .condition_column("Memory pressure", "MemoryPressure")
            .condition_column("Disk pressure", "DiskPressure")
            .condition_column("PID pressure", "PIDPressure")
            .condition_column("Network unavailable", "NetworkUnavailable")
            .size(12, 5),
        // Allocatable and usage per node.
        panel("allocatable", "Allocatable resources per node", NODE_CPU_ALLOCATABLE_METRIC, PanelAggregation::Last)
            .kind(PanelKind::Table)
            .top(EVERY_NODE)
            .column("Memory", NODE_MEMORY_ALLOCATABLE_METRIC, PanelAggregation::Last, &[])
            .column("CPU used (cores)", CPU_USAGE_METRIC, PanelAggregation::Last, &[(LEVEL_ATTRIBUTE, "node")])
            .column("Memory used", MEMORY_USAGE_METRIC, PanelAggregation::Last, &[(LEVEL_ATTRIBUTE, "node")])
            .column("CPU requested %", NODE_CPU_REQUEST_UTILIZATION_METRIC, PanelAggregation::Last, &[])
            .column("Memory requested %", NODE_MEMORY_REQUEST_UTILIZATION_METRIC, PanelAggregation::Last, &[])
            .size(12, 5),
        // Parts of the cluster's used memory: bytes add up, ratios never.
        panel("memory_by_node", "Memory used by node", MEMORY_USAGE_METRIC, PanelAggregation::Last)
            .level("node")
            .kind(PanelKind::Donut)
            .top(EVERY_NODE)
            .size(6, 4),
        panel("cpu_by_node", "CPU used by node (cores)", CPU_USAGE_METRIC, PanelAggregation::Last)
            .level("node")
            .kind(PanelKind::TopList)
            .top(EVERY_NODE)
            .size(6, 4),
        stream("log", "Log lines in the cluster", PanelKind::Logs, false, 50).size(12, 5),
    ];
    let blocks = vec![
        section(
            "Overview",
            "",
            SectionColor::Blue,
            12,
            &["intro", "nodes", "cpu_total", "memory_total", "not_ready", "kubelets_up", "pressure"],
        ),
        section(
            "Utilization",
            "Usage and requests as % of what each node can give pods; dashed lines at 80 % and 100 %.",
            SectionColor::Purple,
            12,
            &["memory_utilization", "cpu_utilization", "memory_requests", "cpu_requests"],
        ),
        section(
            "Conditions",
            "Ready should be 1 and every pressure 0.",
            SectionColor::Red,
            6,
            &["conditions"],
        ),
        section(
            "Capacity",
            "What each node can give pods, what it uses and what is promised.",
            SectionColor::Teal,
            6,
            &["allocatable", "memory_by_node", "cpu_by_node"],
        ),
        section(
            "Log",
            "Log lines of everything in the cluster, newest first.",
            SectionColor::Orange,
            12,
            &["log"],
        ),
    ];
    definition(
        NODES_DASHBOARD_ID,
        "Kubernetes nodes overview",
        "Every node: how many are ready, total CPU and memory, memory and CPU used and requested as % of allocatable with 80 % and 100 % lines, node conditions, allocatable resources and the cluster's log lines.",
        panels,
        blocks,
    )
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use ih_muse_proto::dashboard::PanelGroupBy;

    use super::*;
    use crate::graph::{METRICS, PHASE_A_METRICS};

    /// Metrics whose values are ratios (unit `1`): never parts of a whole.
    const RATIOS: [&str; 6] = [
        NODE_CPU_UTILIZATION_METRIC,
        NODE_MEMORY_UTILIZATION_METRIC,
        NODE_CPU_REQUEST_UTILIZATION_METRIC,
        NODE_MEMORY_REQUEST_UTILIZATION_METRIC,
        CONTAINER_CPU_LIMIT_UTILIZATION_METRIC,
        CONTAINER_MEMORY_LIMIT_UTILIZATION_METRIC,
    ];

    fn kinds(definition: &DashboardDefinition) -> BTreeSet<&'static str> {
        definition
            .panels
            .iter()
            .map(|panel| match panel.kind {
                PanelKind::TimeSeries => "time_series",
                PanelKind::Stat => "stat",
                PanelKind::Counter => "counter",
                PanelKind::Donut => "donut",
                PanelKind::TopList => "top_list",
                PanelKind::Table => "table",
                PanelKind::Text => "text",
                PanelKind::Logs => "logs",
                PanelKind::Traces => "traces",
                PanelKind::Events => "events",
                PanelKind::Trace => "trace",
                PanelKind::Query => "query",
            })
            .collect()
    }

    /// Section label -> (colour, width, panel kinds in order).
    fn sections(
        definition: &DashboardDefinition,
    ) -> BTreeMap<&str, (Option<SectionColor>, Option<u8>, Vec<PanelKind>)> {
        let kind = |id: &String| {
            definition
                .panels
                .iter()
                .find(|panel| &panel.id == id)
                .unwrap()
                .kind
        };
        definition
            .blocks
            .iter()
            .map(|block| {
                (
                    block.label.as_str(),
                    (block.color, block.width, block.panels.iter().map(kind).collect()),
                )
            })
            .collect()
    }

    /// Rules both overviews keep: every panel is in a banded section and
    /// sized; every metric is one the Muse sends; ratios are lines (with
    /// threshold lines), never a donut or a summed row; donuts sum bytes.
    fn keeps_the_rules(definition: &DashboardDefinition) {
        definition.validate().unwrap();
        let sent: BTreeSet<&str> = METRICS.iter().chain(PHASE_A_METRICS.iter()).copied().collect();
        let placed: BTreeSet<&String> = definition.blocks.iter().flat_map(|block| &block.panels).collect();
        for panel in &definition.panels {
            assert!(placed.contains(&panel.id), "{} is in no section", panel.id);
            assert!(panel.size.is_some(), "{} has no size", panel.id);
            let metrics = std::iter::once(panel.metric.as_str())
                .filter(|metric| !metric.is_empty())
                .chain(panel.columns.iter().map(|column| column.metric.as_str()));
            for metric in metrics {
                assert!(sent.contains(metric), "{}: {metric} is not sent by the Muse", panel.id);
            }
            if RATIOS.contains(&panel.metric.as_str()) {
                assert!(
                    matches!(panel.kind, PanelKind::TimeSeries | PanelKind::Table),
                    "{}: a ratio is drawn as {:?}",
                    panel.id,
                    panel.kind
                );
                assert_eq!(panel.aggregation, PanelAggregation::Max, "{}: ratios are never summed", panel.id);
                let thresholds = panel.thresholds.as_ref().expect("ratio thresholds");
                assert_eq!((thresholds.warning, thresholds.critical), (0.8, 1.0), "{}", panel.id);
            }
            for column in &panel.columns {
                if RATIOS.contains(&column.metric.as_str()) {
                    assert!(!matches!(column.aggregation, PanelAggregation::Sum), "{}", panel.id);
                }
            }
            if panel.kind == PanelKind::Donut {
                assert!(panel.aggregation.additive(), "{}", panel.id);
                assert!(panel.group_by.is_some(), "{}", panel.id);
            }
        }
        for block in &definition.blocks {
            assert!(block.color.is_some(), "{} has no band", block.label);
        }
    }

    #[test]
    fn the_pods_overview_reads_like_datadogs() {
        let pods = pods_dashboard();
        keeps_the_rules(&pods);
        assert_eq!(
            kinds(&pods),
            ["donut", "logs", "stat", "table", "text", "time_series", "top_list", "traces"].into()
        );
        let sections = sections(&pods);
        use PanelKind as K;
        use SectionColor as C;
        assert_eq!(
            sections.keys().copied().collect::<Vec<_>>(),
            [
                "Busiest pods and containers",
                "Inventory and attention",
                "Logs and traces",
                "Overview",
                "Pod health",
                "Resource utilization"
            ]
        );
        assert_eq!(sections["Overview"], (Some(C::Gray), None, vec![K::Text]));
        // Pod health and resource utilization side by side, as in Datadog.
        assert_eq!(
            sections["Pod health"],
            (
                Some(C::Green),
                Some(6),
                vec![K::Stat, K::Stat, K::Stat, K::TimeSeries, K::TimeSeries, K::Donut]
            )
        );
        assert_eq!(
            sections["Resource utilization"],
            (Some(C::Blue), Some(6), vec![K::TimeSeries; 3])
        );
        assert_eq!(sections["Busiest pods and containers"], (Some(C::Purple), None, vec![K::Table; 4]));
        assert_eq!(
            sections["Inventory and attention"],
            (Some(C::Teal), None, vec![K::Table, K::TopList, K::Table, K::Table])
        );
        assert_eq!(sections["Logs and traces"], (Some(C::Orange), None, vec![K::Logs, K::Traces]));
        // Sizes differ: a wide intro, small numbers, tall tables.
        let sizes: BTreeSet<(u8, u8)> = pods
            .panels
            .iter()
            .map(|panel| panel.size.map(|size| (size.w, size.h)).unwrap())
            .collect();
        assert!(sizes.len() >= 5, "{sizes:?}");
        // The donut has only the containers that wait for a reason.
        let donut = pods.panels.iter().find(|panel| panel.kind == K::Donut).unwrap();
        assert_eq!(donut.group_by, Some(PanelGroupBy::Attribute(WAITING_REASON_ATTRIBUTE.into())));
        assert!(donut.filters.iter().any(|filter| filter.op == FilterOp::Prefix));
        // Errors and failed traces only.
        for stream in pods.panels.iter().filter_map(|panel| panel.stream.as_ref()) {
            assert!(stream.errors_only);
        }
    }

    #[test]
    fn the_nodes_overview_reads_like_datadogs() {
        let nodes = nodes_dashboard();
        keeps_the_rules(&nodes);
        assert_eq!(
            kinds(&nodes),
            ["donut", "logs", "stat", "table", "text", "time_series", "top_list"].into()
        );
        let sections = sections(&nodes);
        use PanelKind as K;
        use SectionColor as C;
        assert_eq!(
            sections["Overview"],
            (Some(C::Blue), None, vec![K::Text, K::Stat, K::Stat, K::Stat, K::Stat, K::Stat, K::Stat])
        );
        assert_eq!(sections["Utilization"], (Some(C::Purple), None, vec![K::TimeSeries; 4]));
        assert_eq!(sections["Conditions"], (Some(C::Red), Some(6), vec![K::Table]));
        assert_eq!(
            sections["Capacity"],
            (Some(C::Teal), Some(6), vec![K::Table, K::Donut, K::TopList])
        );
        assert_eq!(sections["Log"], (Some(C::Orange), None, vec![K::Logs]));
        // The stats fill one row of 12 under the intro; the warning ones
        // colour by their thresholds.
        let stats: Vec<&PanelSpec> = nodes.panels.iter().filter(|panel| panel.kind == K::Stat).collect();
        assert_eq!(stats.iter().map(|panel| panel.size.unwrap().w).sum::<u8>(), 12);
        assert!(stats.iter().filter(|panel| panel.thresholds.is_some()).count() >= 2);
        // Per-node panels keep every node.
        for panel in nodes.panels.iter().filter(|panel| panel.group_by == Some(PanelGroupBy::Entity)) {
            assert_eq!(panel.top_n, Some(EVERY_NODE), "{}", panel.id);
        }
        // Node conditions: Ready, then the four pressure conditions.
        let conditions = nodes.panels.iter().find(|panel| panel.id == "conditions").unwrap();
        assert_eq!(
            conditions.columns.iter().map(|column| column.title.as_str()).collect::<Vec<_>>(),
            ["Memory pressure", "Disk pressure", "PID pressure", "Network unavailable"]
        );
    }
}
