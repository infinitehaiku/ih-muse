//! One Kubernetes snapshot as one `ih.graph.v1` batch.
//!
//! A port of `collect_k8s_graph` in `ih-infra/scripts/k8s_muse.py`: the same
//! entities, identities, relations, metric names, units, instruments,
//! attributes and availability, in the same order, so Poet's stored history
//! and dashboards continue across the switch:
//!
//! ```text
//! Cluster -> Node                  (contains)
//! Cluster -> Namespace -> Pod      (contains; the namespace keyed by its UID)
//! Pod -> Node                      (scheduled_on)
//! Pod -> Container                 (contains)
//! ```
//!
//! Only the provenance names this Muse ([`GraphConfig::source_id`] and
//! [`GraphConfig::source_revision`]).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use ih_muse_proto::{
    AggregationTemporality, AttributeValue, Availability, Entity, EntityIdentity, EntityKey, Event,
    EventKind, GraphBatch, InstrumentationScope, JoinStatus, MetricDescriptor, MetricInstrument,
    MetricObservation, MetricUnit, MissingReason, Number, Observation, OrganizationId, Provenance,
    RelationKind, SpatialAggregation, TemporalRelation, TimeRange, TypedMetricValue, UnitDisplay,
    ValueDomain, GRAPH_CONTRACT_REVISION, GRAPH_SCHEMA_VERSION,
};

use crate::events::{DetectedEvent, Detector, Target};
use crate::model::{
    parse_quantity, Deployment, KubeEvent, Namespace, Node, NodeMetrics, Pod, PodMetrics,
    ReplicaSet, StatefulSet,
};

/// CPU in cores, at node, pod and container level (`k8s.level`).
pub const CPU_USAGE_METRIC: &str = "k8s.cpu.usage";
/// Memory in bytes, at node, pod and container level (`k8s.level`).
pub const MEMORY_USAGE_METRIC: &str = "k8s.memory.usage";
/// 1 while the pod is in the Running phase, else 0.
pub const POD_RUNNING_METRIC: &str = "k8s.pod.phase.running";
/// 1 while a Running or Pending pod is not ready, else 0.
pub const POD_NOT_READY_METRIC: &str = "k8s.pod.not_ready";
/// Cumulative restarts of a container since its pod started.
pub const CONTAINER_RESTARTS_METRIC: &str = "k8s.container.restarts";
/// The metrics of the first Muse (the Python one's), kept by name.
pub const METRICS: [&str; 5] = [
    CPU_USAGE_METRIC,
    MEMORY_USAGE_METRIC,
    POD_RUNNING_METRIC,
    POD_NOT_READY_METRIC,
    CONTAINER_RESTARTS_METRIC,
];

// Phase A (docs: k8s and Redis metrics research, 2026-10-04): API state
// under OpenTelemetry semantic-convention names. Ratios have unit `1`.

/// Per node, condition type and status: 1 for the current status, else 0
/// (attributes `k8s.node.condition.type`, `k8s.node.condition.status`).
pub const NODE_CONDITION_METRIC: &str = "k8s.node.condition.status";
pub const NODE_CPU_ALLOCATABLE_METRIC: &str = "k8s.node.cpu.allocatable";
pub const NODE_MEMORY_ALLOCATABLE_METRIC: &str = "k8s.node.memory.allocatable";
/// Node usage / allocatable (metrics-server usage).
pub const NODE_CPU_UTILIZATION_METRIC: &str = "k8s.node.cpu.utilization";
pub const NODE_MEMORY_UTILIZATION_METRIC: &str = "k8s.node.memory.utilization";
/// Sum of the requests of the node's running and pending pods / allocatable.
pub const NODE_CPU_REQUEST_UTILIZATION_METRIC: &str = "k8s.node.cpu.request_utilization";
pub const NODE_MEMORY_REQUEST_UTILIZATION_METRIC: &str = "k8s.node.memory.request_utilization";
pub const CONTAINER_CPU_REQUEST_METRIC: &str = "k8s.container.cpu.request";
pub const CONTAINER_CPU_LIMIT_METRIC: &str = "k8s.container.cpu.limit";
pub const CONTAINER_MEMORY_REQUEST_METRIC: &str = "k8s.container.memory.request";
pub const CONTAINER_MEMORY_LIMIT_METRIC: &str = "k8s.container.memory.limit";
/// Usage / limit; not sent for a container without that limit.
pub const CONTAINER_CPU_LIMIT_UTILIZATION_METRIC: &str = "k8s.container.cpu.limit.utilization";
pub const CONTAINER_MEMORY_LIMIT_UTILIZATION_METRIC: &str =
    "k8s.container.memory.limit.utilization";
/// 1 while the container waits for a problem reason (attribute
/// `k8s.container.status.reason`), else 0.
pub const CONTAINER_WAITING_METRIC: &str = "k8s.container.waiting";
/// OOMKills seen since the Muse first saw the container (cumulative).
pub const CONTAINER_OOM_KILLS_METRIC: &str = "k8s.container.oom_kills";
/// 1 while the pod cannot be scheduled (`PodScheduled=False/Unschedulable`).
pub const POD_UNSCHEDULABLE_METRIC: &str = "k8s.pod.unschedulable";
pub const DEPLOYMENT_DESIRED_METRIC: &str = "k8s.deployment.pod.desired";
pub const DEPLOYMENT_AVAILABLE_METRIC: &str = "k8s.deployment.pod.available";
pub const STATEFULSET_DESIRED_METRIC: &str = "k8s.statefulset.pod.desired";
pub const STATEFULSET_READY_METRIC: &str = "k8s.statefulset.pod.ready";
/// Every phase A metric.
pub const PHASE_A_METRICS: [&str; 20] = [
    NODE_CONDITION_METRIC,
    NODE_CPU_ALLOCATABLE_METRIC,
    NODE_MEMORY_ALLOCATABLE_METRIC,
    NODE_CPU_UTILIZATION_METRIC,
    NODE_MEMORY_UTILIZATION_METRIC,
    NODE_CPU_REQUEST_UTILIZATION_METRIC,
    NODE_MEMORY_REQUEST_UTILIZATION_METRIC,
    CONTAINER_CPU_REQUEST_METRIC,
    CONTAINER_CPU_LIMIT_METRIC,
    CONTAINER_MEMORY_REQUEST_METRIC,
    CONTAINER_MEMORY_LIMIT_METRIC,
    CONTAINER_CPU_LIMIT_UTILIZATION_METRIC,
    CONTAINER_MEMORY_LIMIT_UTILIZATION_METRIC,
    CONTAINER_WAITING_METRIC,
    CONTAINER_OOM_KILLS_METRIC,
    POD_UNSCHEDULABLE_METRIC,
    DEPLOYMENT_DESIRED_METRIC,
    DEPLOYMENT_AVAILABLE_METRIC,
    STATEFULSET_DESIRED_METRIC,
    STATEFULSET_READY_METRIC,
];
/// Observation attribute of [`CONTAINER_WAITING_METRIC`].
pub const WAITING_REASON_ATTRIBUTE: &str = "k8s.container.status.reason";
/// The instrumentation scope of change and problem events.
pub const EVENT_SCOPE_NAME: &str = "ih.k8s.events";
/// Version of [`EVENT_SCOPE_NAME`]: the event set.
pub const EVENT_SCOPE_VERSION: &str = "1.0.0";
/// Observation attribute naming the level a usage value belongs to.
pub const LEVEL_ATTRIBUTE: &str = "k8s.level";
/// The instrumentation scope of every observation (as the Python Muse).
pub const SCOPE_NAME: &str = "ih.k8s.metrics";
/// Version of [`SCOPE_NAME`]: the metric set, not the Muse.
pub const SCOPE_VERSION: &str = "2.0.0";
/// Built-in Poet profile the cluster root names (kept for older Poets).
pub const DASHBOARD_PROFILE: &str = "orchestrator.kubernetes";

const FOREVER: TimeRange = TimeRange {
    from_unix_nano: 0,
    to_unix_nano: u64::MAX,
};

/// Who and what one batch describes.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphConfig {
    pub organization: String,
    /// The cluster root's identity (`k8s.cluster.uid`).
    pub cluster_uid: String,
    /// The cluster's name people read (`k-lab`, its `k8s.cluster.name`);
    /// `None` names it by its UID.
    pub cluster_name: Option<String>,
    /// The namespace whose pods are listed (the Muse's own in
    /// all-namespace mode).
    pub namespace: String,
    /// Every namespace's pods, workloads and warnings, not only `namespace`'s.
    pub all_namespaces: bool,
    /// Collection cadence; bounds the availability windows.
    pub expected_interval_ns: u64,
    /// Provenance producer id (see `crate::identity`).
    pub source_id: String,
    pub source_revision: String,
}

/// What one collection read from the API. `None` metrics mean
/// metrics-server did not answer: every usage value becomes a gap.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub nodes: Vec<Node>,
    pub node_metrics: Option<Vec<NodeMetrics>>,
    pub pods: Vec<Pod>,
    pub pod_metrics: Option<Vec<PodMetrics>>,
    /// Without it the namespace entity is left out (never keyed by name).
    pub namespace_uid: Option<String>,
    /// Every namespace (all-namespace mode): entities keyed by UID.
    pub namespaces: Vec<Namespace>,
    pub deployments: Vec<Deployment>,
    pub statefulsets: Vec<StatefulSet>,
    /// Each Deployment revision's template (rollout change details).
    pub replicasets: Vec<ReplicaSet>,
    /// Kubernetes `Warning` events; `None` when the Events API did not answer.
    pub warnings: Option<Vec<KubeEvent>>,
}

/// One metric's descriptor fields that differ between metrics.
struct Metric {
    name: &'static str,
    ucum: &'static str,
    display: UnitDisplay,
    instrument: MetricInstrument,
}

fn metric(name: &'static str) -> Metric {
    let (ucum, display, instrument) = match name {
        MEMORY_USAGE_METRIC => ("By", UnitDisplay::Bytes, MetricInstrument::Gauge),
        CONTAINER_RESTARTS_METRIC => (
            "{restart}",
            UnitDisplay::Number,
            MetricInstrument::Sum {
                monotonic: true,
                temporality: AggregationTemporality::Cumulative,
            },
        ),
        NODE_MEMORY_ALLOCATABLE_METRIC
        | CONTAINER_MEMORY_REQUEST_METRIC
        | CONTAINER_MEMORY_LIMIT_METRIC => ("By", UnitDisplay::Bytes, MetricInstrument::Gauge),
        NODE_CPU_ALLOCATABLE_METRIC | CONTAINER_CPU_REQUEST_METRIC | CONTAINER_CPU_LIMIT_METRIC => {
            ("{cpu}", UnitDisplay::Number, MetricInstrument::Gauge)
        }
        NODE_CPU_UTILIZATION_METRIC
        | NODE_MEMORY_UTILIZATION_METRIC
        | NODE_CPU_REQUEST_UTILIZATION_METRIC
        | NODE_MEMORY_REQUEST_UTILIZATION_METRIC
        | CONTAINER_CPU_LIMIT_UTILIZATION_METRIC
        | CONTAINER_MEMORY_LIMIT_UTILIZATION_METRIC => {
            ("1", UnitDisplay::Ratio, MetricInstrument::Gauge)
        }
        NODE_CONDITION_METRIC => ("{node}", UnitDisplay::Number, MetricInstrument::Gauge),
        CONTAINER_WAITING_METRIC => ("{container}", UnitDisplay::Number, MetricInstrument::Gauge),
        POD_UNSCHEDULABLE_METRIC
        | DEPLOYMENT_DESIRED_METRIC
        | DEPLOYMENT_AVAILABLE_METRIC
        | STATEFULSET_DESIRED_METRIC
        | STATEFULSET_READY_METRIC => ("{pod}", UnitDisplay::Number, MetricInstrument::Gauge),
        CONTAINER_OOM_KILLS_METRIC => (
            "{event}",
            UnitDisplay::Number,
            MetricInstrument::Sum {
                monotonic: true,
                temporality: AggregationTemporality::Cumulative,
            },
        ),
        _ => ("1", UnitDisplay::Number, MetricInstrument::Gauge),
    };
    Metric {
        name,
        ucum,
        display,
        instrument,
    }
}

fn description(name: &str) -> &'static str {
    match name {
        CPU_USAGE_METRIC => "Kubernetes CPU usage",
        MEMORY_USAGE_METRIC => "Kubernetes memory usage",
        POD_RUNNING_METRIC => "Pod is in the Running phase",
        POD_NOT_READY_METRIC => "Pod that should serve is not ready",
        CONTAINER_RESTARTS_METRIC => "Container restarts since the pod started",
        NODE_CONDITION_METRIC => "Node condition: 1 for the current status",
        NODE_CPU_ALLOCATABLE_METRIC => "Node CPU allocatable to pods",
        NODE_MEMORY_ALLOCATABLE_METRIC => "Node memory allocatable to pods",
        NODE_CPU_UTILIZATION_METRIC => "Node CPU usage / allocatable",
        NODE_MEMORY_UTILIZATION_METRIC => "Node memory usage / allocatable",
        NODE_CPU_REQUEST_UTILIZATION_METRIC => "Node CPU requests / allocatable",
        NODE_MEMORY_REQUEST_UTILIZATION_METRIC => "Node memory requests / allocatable",
        CONTAINER_CPU_REQUEST_METRIC => "Container CPU request",
        CONTAINER_CPU_LIMIT_METRIC => "Container CPU limit",
        CONTAINER_MEMORY_REQUEST_METRIC => "Container memory request",
        CONTAINER_MEMORY_LIMIT_METRIC => "Container memory limit",
        CONTAINER_CPU_LIMIT_UTILIZATION_METRIC => "Container CPU usage / limit",
        CONTAINER_MEMORY_LIMIT_UTILIZATION_METRIC => "Container memory usage / limit",
        CONTAINER_WAITING_METRIC => "Container waiting for a problem reason",
        CONTAINER_OOM_KILLS_METRIC => "Container OOMKills seen by the Muse",
        POD_UNSCHEDULABLE_METRIC => "Pod cannot be scheduled",
        DEPLOYMENT_DESIRED_METRIC => "Deployment desired pods",
        DEPLOYMENT_AVAILABLE_METRIC => "Deployment available pods",
        STATEFULSET_DESIRED_METRIC => "StatefulSet desired pods",
        STATEFULSET_READY_METRIC => "StatefulSet ready pods",
        _ => "",
    }
}

/// A value to record: a measurement or an explicit gap (never a zero).
#[derive(Clone, Copy)]
enum Value {
    F64(Option<f64>),
    I64(i64),
}

/// Collects the batch while it is built.
struct Builder<'a> {
    config: &'a GraphConfig,
    now: u64,
    entities: Vec<Entity>,
    relations: Vec<TemporalRelation>,
    observations: Vec<Observation>,
}

impl Builder<'_> {
    fn provenance(&self) -> Provenance {
        Provenance {
            source_id: self.config.source_id.clone(),
            source_revision: self.config.source_revision.clone(),
            observed_at_unix_nano: self.now,
            join_status: JoinStatus::Resolved,
        }
    }

    fn key(&self, identity: EntityIdentity) -> EntityKey {
        EntityKey {
            organization: OrganizationId(self.config.organization.clone()),
            identity,
        }
    }

    fn kubernetes(&self, resource_kind: &str, resource_uid: &str) -> EntityKey {
        self.key(EntityIdentity::Kubernetes {
            cluster_uid: self.config.cluster_uid.clone(),
            resource_kind: resource_kind.into(),
            resource_uid: resource_uid.into(),
        })
    }

    fn entity(&mut self, key: &EntityKey, attributes: Vec<(&str, AttributeValue)>) {
        self.entities.push(Entity {
            key: key.clone(),
            lifetime: FOREVER,
            attributes: attributes
                .into_iter()
                .map(|(name, value)| (name.to_string(), value))
                .collect(),
        });
    }

    fn relation(&mut self, subject: &EntityKey, object: &EntityKey, kind: RelationKind) {
        let provenance = self.provenance();
        self.relations.push(TemporalRelation {
            subject: subject.clone(),
            object: object.clone(),
            kind,
            valid_time: FOREVER,
            provenance,
            attributes: BTreeMap::new(),
        });
    }

    fn observe(
        &mut self,
        entity: &EntityKey,
        name: &'static str,
        value: Value,
        level: &str,
        start_time: Option<u64>,
    ) {
        self.observe_with(
            entity,
            name,
            value,
            BTreeMap::from([(LEVEL_ATTRIBUTE.to_string(), string(level))]),
            start_time,
        );
    }

    /// A phase A observation: its own attributes, no `k8s.level`.
    fn record(
        &mut self,
        entity: &EntityKey,
        name: &'static str,
        value: Value,
        attributes: &[(&str, &str)],
        start_time: Option<u64>,
    ) {
        self.observe_with(
            entity,
            name,
            value,
            attributes
                .iter()
                .map(|(key, value)| (key.to_string(), string(*value)))
                .collect(),
            start_time,
        );
    }

    fn observe_with(
        &mut self,
        entity: &EntityKey,
        name: &'static str,
        value: Value,
        attributes: BTreeMap<String, AttributeValue>,
        start_time: Option<u64>,
    ) {
        let metric = metric(name);
        let sum = matches!(metric.instrument, MetricInstrument::Sum { .. });
        let number = match value {
            Value::F64(Some(value)) => Some(Number::F64(value)),
            Value::F64(None) => None,
            Value::I64(value) => Some(Number::I64(value)),
        };
        let value = match number {
            Some(number) => MetricObservation::Measured {
                value: if sum {
                    TypedMetricValue::Sum(number)
                } else {
                    TypedMetricValue::Gauge(number)
                },
            },
            None => MetricObservation::Missing {
                reason: MissingReason::SourceUnavailable,
            },
        };
        let provenance = self.provenance();
        self.observations.push(Observation {
            entity: entity.clone(),
            scope: InstrumentationScope {
                name: SCOPE_NAME.into(),
                version: Some(SCOPE_VERSION.into()),
                schema_url: None,
                attributes: BTreeMap::new(),
            },
            descriptor: MetricDescriptor {
                name: metric.name.into(),
                description: description(metric.name).into(),
                unit: MetricUnit {
                    ucum: metric.ucum.into(),
                    display: metric.display,
                },
                value_domain: ValueDomain::Unbounded,
                instrument: metric.instrument,
                spatial_aggregation: SpatialAggregation::Sum,
            },
            ratio_denominator: None,
            attributes,
            start_time_unix_nano: start_time,
            time_unix_nano: self.now,
            value,
            provenance,
        });
    }
}

fn string(value: impl Into<String>) -> AttributeValue {
    AttributeValue::String(value.into())
}

/// Parses an RFC 3339 time (`2026-10-01T08:00:00Z`) to Unix nanoseconds.
fn unix_nano(text: Option<&str>) -> Option<u64> {
    let time = chrono::DateTime::parse_from_rfc3339(text?).ok()?;
    u64::try_from(time.timestamp_nanos_opt()?).ok()
}

/// Usage values, present only when the map has any entry (an empty usage
/// map is a gap, as in the Python Muse).
fn usage(map: Option<&BTreeMap<String, String>>, resource: &str) -> Option<f64> {
    let map = map.filter(|map| !map.is_empty())?;
    Some(crate::model::parse_quantity(
        map.get(resource).map_or("0", String::as_str),
    ))
}

/// Builds the batch for `snapshot` at `now` (Unix nanoseconds), without
/// events or OOMKill counts.
pub fn collect_graph(snapshot: &Snapshot, config: &GraphConfig, now: u64) -> GraphBatch {
    collect_graph_with(snapshot, config, now, None, &[])
}

/// Sum of one resource's requests per node over its running and pending
/// pods (what the scheduler has promised).
fn node_requests(pods: &[Pod], resource: &str) -> HashMap<String, f64> {
    let mut sums = HashMap::new();
    for pod in pods {
        let phase = pod.status.phase.as_deref().unwrap_or("Unknown");
        let Some(node) = pod.spec.node_name.as_ref() else {
            continue;
        };
        if !matches!(phase, "Running" | "Pending") {
            continue;
        }
        let requested: f64 = pod
            .spec
            .containers
            .iter()
            .filter_map(|container| container.resources.requests.get(resource))
            .map(|quantity| parse_quantity(quantity))
            .sum();
        *sums.entry(node.clone()).or_insert(0.0) += requested;
    }
    sums
}

/// `part / whole`, or a gap when there is no whole to divide by.
fn ratio(part: Option<f64>, whole: Option<f64>) -> Option<f64> {
    match (part, whole) {
        (Some(part), Some(whole)) if whole > 0.0 => Some(part / whole),
        _ => None,
    }
}

/// Builds the batch for `snapshot` at `now`, with `events` (from the
/// [`Detector`], which also holds the OOMKill counters).
pub fn collect_graph_with(
    snapshot: &Snapshot,
    config: &GraphConfig,
    now: u64,
    detector: Option<&Detector>,
    events: &[DetectedEvent],
) -> GraphBatch {
    let mut b = Builder {
        config,
        now,
        entities: Vec::new(),
        relations: Vec::new(),
        observations: Vec::new(),
    };

    let cluster = b.key(EntityIdentity::Cluster {
        cluster_uid: config.cluster_uid.clone(),
    });
    b.entity(
        &cluster,
        vec![
            (
                "k8s.cluster.name",
                string(
                    config
                        .cluster_name
                        .as_deref()
                        .map(str::trim)
                        .filter(|name| !name.is_empty())
                        .unwrap_or(&config.cluster_uid),
                ),
            ),
            ("k8s.cluster.uid", string(&config.cluster_uid)),
            ("ih.dashboard.profile", string(DASHBOARD_PROFILE)),
        ],
    );
    // Namespace entities by name: every namespace in all-namespace mode,
    // else the Muse's own (when its UID is known).
    let mut namespaces: HashMap<String, EntityKey> = HashMap::new();
    if config.all_namespaces {
        for item in &snapshot.namespaces {
            if item.metadata.uid.is_empty() {
                continue;
            }
            let key = b.kubernetes("namespace", &item.metadata.uid);
            b.entity(
                &key,
                vec![("k8s.namespace.name", string(&item.metadata.name))],
            );
            b.relation(&cluster, &key, RelationKind::Contains);
            namespaces.insert(item.metadata.name.clone(), key);
        }
    } else if let Some(uid) = snapshot.namespace_uid.as_deref() {
        let key = b.kubernetes("namespace", uid);
        b.entity(
            &key,
            vec![("k8s.namespace.name", string(&config.namespace))],
        );
        b.relation(&cluster, &key, RelationKind::Contains);
        namespaces.insert(config.namespace.clone(), key);
    }
    let namespace_of = |pod_namespace: &str| -> String {
        if config.all_namespaces && !pod_namespace.is_empty() {
            pod_namespace.to_string()
        } else {
            config.namespace.clone()
        }
    };
    let cpu_requests = node_requests(&snapshot.pods, "cpu");
    let memory_requests = node_requests(&snapshot.pods, "memory");

    let node_usage: HashMap<&str, &BTreeMap<String, String>> = snapshot
        .node_metrics
        .iter()
        .flatten()
        .map(|item| (item.metadata.name.as_str(), &item.usage))
        .collect();
    let mut node_keys = HashMap::new();
    for node in &snapshot.nodes {
        let name = node.metadata.name.as_str();
        let key = b.kubernetes("node", &node.metadata.uid);
        let allocatable = |resource: &str| {
            string(
                node.status
                    .allocatable
                    .get(resource)
                    .cloned()
                    .unwrap_or_default(),
            )
        };
        b.entity(
            &key,
            vec![
                ("k8s.node.name", string(name)),
                ("k8s.node.cpu.allocatable", allocatable("cpu")),
                ("k8s.node.memory.allocatable", allocatable("memory")),
                ("k8s.node.arch", string(&node.status.node_info.architecture)),
                (
                    "k8s.node.os",
                    string(&node.status.node_info.operating_system),
                ),
            ],
        );
        b.relation(&cluster, &key, RelationKind::Contains);
        let measured = node_usage.get(name).copied();
        b.observe(
            &key,
            CPU_USAGE_METRIC,
            Value::F64(usage(measured, "cpu")),
            "node",
            None,
        );
        b.observe(
            &key,
            MEMORY_USAGE_METRIC,
            Value::F64(usage(measured, "memory")),
            "node",
            None,
        );
        // Phase A: conditions, allocatable as numbers, % of allocatable.
        for condition in &node.status.conditions {
            for status in ["true", "false", "unknown"] {
                let current = condition.status.eq_ignore_ascii_case(status);
                b.record(
                    &key,
                    NODE_CONDITION_METRIC,
                    Value::I64(i64::from(current)),
                    &[
                        ("k8s.node.condition.type", condition.kind.as_str()),
                        ("k8s.node.condition.status", status),
                    ],
                    None,
                );
            }
        }
        let allocatable_cpu = node
            .status
            .allocatable
            .get("cpu")
            .map(|quantity| parse_quantity(quantity));
        let allocatable_memory = node
            .status
            .allocatable
            .get("memory")
            .map(|quantity| parse_quantity(quantity));
        if let Some(cpu) = allocatable_cpu {
            b.record(
                &key,
                NODE_CPU_ALLOCATABLE_METRIC,
                Value::F64(Some(cpu)),
                &[],
                None,
            );
        }
        if let Some(memory) = allocatable_memory {
            b.record(
                &key,
                NODE_MEMORY_ALLOCATABLE_METRIC,
                Value::F64(Some(memory)),
                &[],
                None,
            );
        }
        b.record(
            &key,
            NODE_CPU_UTILIZATION_METRIC,
            Value::F64(ratio(usage(measured, "cpu"), allocatable_cpu)),
            &[],
            None,
        );
        b.record(
            &key,
            NODE_MEMORY_UTILIZATION_METRIC,
            Value::F64(ratio(usage(measured, "memory"), allocatable_memory)),
            &[],
            None,
        );
        b.record(
            &key,
            NODE_CPU_REQUEST_UTILIZATION_METRIC,
            Value::F64(ratio(
                Some(cpu_requests.get(name).copied().unwrap_or(0.0)),
                allocatable_cpu,
            )),
            &[],
            None,
        );
        b.record(
            &key,
            NODE_MEMORY_REQUEST_UTILIZATION_METRIC,
            Value::F64(ratio(
                Some(memory_requests.get(name).copied().unwrap_or(0.0)),
                allocatable_memory,
            )),
            &[],
            None,
        );
        node_keys.insert(name.to_string(), key);
    }

    let pod_usage: HashMap<(String, &str), &Vec<crate::model::ContainerMetrics>> = snapshot
        .pod_metrics
        .iter()
        .flatten()
        .map(|item| {
            (
                (
                    namespace_of(&item.metadata.namespace),
                    item.metadata.name.as_str(),
                ),
                &item.containers,
            )
        })
        .collect();
    let mut pod_keys: HashMap<String, EntityKey> = HashMap::new();
    for pod in &snapshot.pods {
        let uid = pod.metadata.uid.as_str();
        let status = &pod.status;
        let phase = status.phase.as_deref().unwrap_or("Unknown");
        let start_text = status
            .start_time
            .as_deref()
            .filter(|text| !text.is_empty())
            .or(pod.metadata.creation_timestamp.as_deref());
        // Clock skew must not invalidate the counter.
        let started = unix_nano(start_text).map(|started| started.min(now).max(1));
        let key = b.kubernetes("pod", uid);
        let pod_namespace = namespace_of(&pod.metadata.namespace);
        b.entity(
            &key,
            vec![
                ("k8s.pod.name", string(&pod.metadata.name)),
                (
                    "k8s.pod.ip",
                    string(status.pod_ip.clone().unwrap_or_default()),
                ),
                ("k8s.pod.phase", string(phase)),
                ("k8s.namespace.name", string(&pod_namespace)),
            ],
        );
        if let Some(namespace) = namespaces.get(&pod_namespace) {
            b.relation(namespace, &key, RelationKind::Contains);
        }
        pod_keys.insert(uid.to_string(), key.clone());
        if let Some(node) = pod
            .spec
            .node_name
            .as_ref()
            .and_then(|name| node_keys.get(name))
        {
            b.relation(&key, node, RelationKind::ScheduledOn);
        }
        let running = phase == "Running";
        b.observe(
            &key,
            POD_RUNNING_METRIC,
            Value::I64(i64::from(running)),
            "pod",
            None,
        );
        let should_serve = matches!(phase, "Running" | "Pending");
        let ready = status
            .conditions
            .iter()
            .any(|condition| condition.kind == "Ready" && condition.status == "True");
        b.observe(
            &key,
            POD_NOT_READY_METRIC,
            Value::I64(i64::from(should_serve && !ready)),
            "pod",
            None,
        );
        let unschedulable = status.conditions.iter().any(|condition| {
            condition.kind == "PodScheduled"
                && condition.status == "False"
                && condition.reason.as_deref() == Some("Unschedulable")
        });
        b.record(
            &key,
            POD_UNSCHEDULABLE_METRIC,
            Value::I64(i64::from(unschedulable)),
            &[],
            None,
        );

        let by_container: HashMap<&str, &BTreeMap<String, String>> = pod_usage
            .get(&(pod_namespace.clone(), pod.metadata.name.as_str()))
            .into_iter()
            .flat_map(|containers| containers.iter())
            .map(|container| (container.name.as_str(), &container.usage))
            .collect();
        let (mut pod_cpu, mut pod_memory, mut measured_any) = (0.0, 0.0, false);
        for container in &status.container_statuses {
            let name = container.name.as_str();
            let container_id = match container.container_id.as_deref() {
                Some(raw) if !raw.is_empty() => raw.rsplit("://").next().unwrap_or(raw).to_string(),
                _ => format!("{uid}-{name}"),
            };
            let container_key = b.key(EntityIdentity::Container {
                cluster_uid: config.cluster_uid.clone(),
                pod_uid: uid.into(),
                container_name: name.into(),
                container_id: container_id.clone(),
            });
            // A missing or empty state reads as "unknown" for the attribute
            // and as running for usage, exactly as in the Python Muse.
            let state = container.state.as_ref().filter(|state| !state.is_empty());
            let state_name = state
                .and_then(|state| state.keys().next().cloned())
                .unwrap_or_else(|| "unknown".into());
            b.entity(
                &container_key,
                vec![
                    ("container.name", string(name)),
                    ("container.id", string(&container_id)),
                    ("container.ready", AttributeValue::Bool(container.ready)),
                    (
                        "container.restart_count",
                        AttributeValue::I64(container.restart_count),
                    ),
                    ("container.state", string(state_name)),
                ],
            );
            b.relation(&key, &container_key, RelationKind::Contains);
            b.observe(
                &container_key,
                CONTAINER_RESTARTS_METRIC,
                Value::I64(container.restart_count),
                "container",
                started,
            );
            // Only a running container has usage; any other state is a gap.
            let is_running = state.map_or(true, |state| state.contains_key("running"));
            let measured = if is_running {
                by_container.get(name).copied()
            } else {
                None
            };
            let cpu = usage(measured, "cpu");
            let memory = usage(measured, "memory");
            b.observe(
                &container_key,
                CPU_USAGE_METRIC,
                Value::F64(cpu),
                "container",
                None,
            );
            b.observe(
                &container_key,
                MEMORY_USAGE_METRIC,
                Value::F64(memory),
                "container",
                None,
            );
            // Phase A: requests, limits, % of limit, waiting, OOMKills.
            let spec = pod.spec.containers.iter().find(|spec| spec.name == name);
            let quantity = |section: &BTreeMap<String, String>, resource: &str| {
                section
                    .get(resource)
                    .map(|quantity| parse_quantity(quantity))
            };
            if let Some(spec) = spec {
                let resources = &spec.resources;
                for (metric, value) in [
                    (
                        CONTAINER_CPU_REQUEST_METRIC,
                        quantity(&resources.requests, "cpu"),
                    ),
                    (
                        CONTAINER_CPU_LIMIT_METRIC,
                        quantity(&resources.limits, "cpu"),
                    ),
                    (
                        CONTAINER_MEMORY_REQUEST_METRIC,
                        quantity(&resources.requests, "memory"),
                    ),
                    (
                        CONTAINER_MEMORY_LIMIT_METRIC,
                        quantity(&resources.limits, "memory"),
                    ),
                ] {
                    if let Some(value) = value {
                        b.record(&container_key, metric, Value::F64(Some(value)), &[], None);
                    }
                }
                if let Some(limit) = quantity(&resources.limits, "cpu") {
                    b.record(
                        &container_key,
                        CONTAINER_CPU_LIMIT_UTILIZATION_METRIC,
                        Value::F64(ratio(cpu, Some(limit))),
                        &[],
                        None,
                    );
                }
                if let Some(limit) = quantity(&resources.limits, "memory") {
                    b.record(
                        &container_key,
                        CONTAINER_MEMORY_LIMIT_UTILIZATION_METRIC,
                        Value::F64(ratio(memory, Some(limit))),
                        &[],
                        None,
                    );
                }
            }
            match container
                .waiting_reason()
                .filter(|reason| !matches!(*reason, "ContainerCreating" | "PodInitializing"))
            {
                Some(reason) => b.record(
                    &container_key,
                    CONTAINER_WAITING_METRIC,
                    Value::I64(1),
                    &[(WAITING_REASON_ATTRIBUTE, reason)],
                    None,
                ),
                None => b.record(
                    &container_key,
                    CONTAINER_WAITING_METRIC,
                    Value::I64(0),
                    &[],
                    None,
                ),
            }
            if let Some(counter) = detector.and_then(|detector| detector.oom_kills(uid, name)) {
                b.record(
                    &container_key,
                    CONTAINER_OOM_KILLS_METRIC,
                    Value::I64(counter.kills),
                    &[],
                    Some(counter.since_unix_nano.min(now).max(1)),
                );
            }
            if let Some(cpu) = cpu {
                measured_any = true;
                pod_cpu += cpu;
                pod_memory += memory.unwrap_or(0.0);
            }
        }
        b.observe(
            &key,
            CPU_USAGE_METRIC,
            Value::F64(measured_any.then_some(pod_cpu)),
            "pod",
            None,
        );
        b.observe(
            &key,
            MEMORY_USAGE_METRIC,
            Value::F64(measured_any.then_some(pod_memory)),
            "pod",
            None,
        );
    }

    // Workloads: Namespace -> Deployment/StatefulSet, desired vs available.
    let mut workload_keys: HashMap<String, EntityKey> = HashMap::new();
    for (kind, items) in [
        ("deployment", &snapshot.deployments),
        ("statefulset", &snapshot.statefulsets),
    ] {
        for workload in items {
            let meta = &workload.metadata;
            if meta.uid.is_empty() {
                continue;
            }
            let key = b.kubernetes(kind, &meta.uid);
            let workload_namespace = namespace_of(&meta.namespace);
            let name_attribute = if kind == "deployment" {
                "k8s.deployment.name"
            } else {
                "k8s.statefulset.name"
            };
            b.entity(
                &key,
                vec![
                    (name_attribute, string(&meta.name)),
                    ("k8s.namespace.name", string(&workload_namespace)),
                ],
            );
            if let Some(namespace) = namespaces.get(&workload_namespace) {
                b.relation(namespace, &key, RelationKind::Contains);
            }
            let desired = workload.spec.replicas.unwrap_or(1);
            let (desired_metric, current_metric, current) = if kind == "deployment" {
                (
                    DEPLOYMENT_DESIRED_METRIC,
                    DEPLOYMENT_AVAILABLE_METRIC,
                    workload.status.available_replicas,
                )
            } else {
                (
                    STATEFULSET_DESIRED_METRIC,
                    STATEFULSET_READY_METRIC,
                    workload.status.ready_replicas,
                )
            };
            b.record(&key, desired_metric, Value::I64(desired), &[], None);
            b.record(&key, current_metric, Value::I64(current), &[], None);
            workload_keys.insert(meta.uid.clone(), key);
        }
    }

    // Events on the element they concern; one no longer in the batch (a
    // deleted pod) falls back to its namespace, then the cluster.
    let mut batch_events = Vec::with_capacity(events.len());
    for event in events {
        let fallback = |namespace: &str| namespaces.get(namespace).cloned();
        let entity = match &event.target {
            Target::Pod { uid, namespace } => pod_keys
                .get(uid)
                .cloned()
                .or_else(|| fallback(&namespace_of(namespace))),
            Target::Node { uid } => node_keys
                .values()
                .find(|key| {
                    matches!(&key.identity, EntityIdentity::Kubernetes { resource_uid, .. } if resource_uid == uid)
                })
                .cloned(),
            Target::Workload { uid, namespace, .. } => workload_keys
                .get(uid)
                .cloned()
                .or_else(|| fallback(&namespace_of(namespace))),
            Target::Namespace { name } => fallback(&namespace_of(name)),
            Target::Cluster => None,
        }
        .unwrap_or_else(|| cluster.clone());
        let mut attributes: BTreeMap<String, AttributeValue> = event
            .attributes
            .iter()
            .map(|(name, value)| (name.clone(), string(value.clone())))
            .collect();
        attributes.insert("event.name".into(), string(event.name));
        attributes.insert("ih.event.severity".into(), string(event.severity.as_str()));
        attributes.insert("k8s.cluster.name".into(), string(cluster_label(config)));
        let time = event.time_unix_nano.max(1);
        batch_events.push(Event {
            entity,
            kind: EventKind::Domain,
            event_id: format!("k8s:{}:{}", config.cluster_uid, event.id),
            time: TimeRange {
                from_unix_nano: time,
                to_unix_nano: time.saturating_add(1),
            },
            scope: InstrumentationScope {
                name: EVENT_SCOPE_NAME.into(),
                version: Some(EVENT_SCOPE_VERSION.into()),
                schema_url: None,
                attributes: BTreeMap::new(),
            },
            attributes,
            body: Some(string(event.summary.clone())),
            provenance: b.provenance(),
        });
    }

    // Availability states each series' real cadence, so a new metric never
    // looks absent and Poet bounds interpolation by the true interval.
    let interval = config.expected_interval_ns;
    let mut seen = BTreeSet::new();
    let mut availability = Vec::new();
    for observation in &b.observations {
        if !seen.insert((&observation.entity, observation.descriptor.name.as_str())) {
            continue;
        }
        availability.push(Availability {
            entity: observation.entity.clone(),
            metric_name: Some(observation.descriptor.name.clone()),
            available_time: TimeRange {
                from_unix_nano: now.saturating_sub(interval),
                to_unix_nano: now + 1,
            },
            resolution_unix_nano: interval,
            coverage_fraction: 1.0,
            retained_from_unix_nano: now.saturating_sub(interval),
            pressure_policy: None,
            provenance: b.provenance(),
        });
    }

    GraphBatch {
        schema_version: GRAPH_SCHEMA_VERSION,
        contract_revision: GRAPH_CONTRACT_REVISION.into(),
        entities: dedup_last_wins(b.entities, |entity| entity.key.clone()),
        relations: dedup_last_wins(b.relations, |relation| {
            (
                relation.subject.clone(),
                relation.object.clone(),
                relation.kind.clone(),
            )
        }),
        observations: b.observations,
        events: batch_events,
        derivations: Vec::new(),
        availability,
        dashboards: Vec::new(),
    }
}

fn cluster_label(config: &GraphConfig) -> &str {
    config
        .cluster_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(&config.cluster_uid)
}

/// Keeps one item per key at the position of its first occurrence with the
/// value of its last (a Python dict built in order).
fn dedup_last_wins<T, K: Ord>(items: Vec<T>, key: impl Fn(&T) -> K) -> Vec<T> {
    let mut position = BTreeMap::new();
    let mut out: Vec<T> = Vec::with_capacity(items.len());
    for item in items {
        match position.get(&key(&item)) {
            Some(&index) => out[index] = item,
            None => {
                position.insert(key(&item), out.len());
                out.push(item);
            }
        }
    }
    out
}
