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
    AggregationTemporality, AttributeValue, Availability, Entity, EntityIdentity, EntityKey,
    GraphBatch, InstrumentationScope, JoinStatus, MetricDescriptor, MetricInstrument,
    MetricObservation, MetricUnit, MissingReason, Number, Observation, OrganizationId, Provenance,
    RelationKind, SpatialAggregation, TemporalRelation, TimeRange, TypedMetricValue, UnitDisplay,
    ValueDomain, GRAPH_CONTRACT_REVISION, GRAPH_SCHEMA_VERSION,
};

use crate::model::{Node, NodeMetrics, Pod, PodMetrics};

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
/// Every metric this Muse emits.
pub const METRICS: [&str; 5] = [
    CPU_USAGE_METRIC,
    MEMORY_USAGE_METRIC,
    POD_RUNNING_METRIC,
    POD_NOT_READY_METRIC,
    CONTAINER_RESTARTS_METRIC,
];
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
    /// The cluster root's identity and its `k8s.cluster.name`.
    pub cluster_uid: String,
    /// The namespace whose pods are listed.
    pub namespace: String,
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
            attributes: BTreeMap::from([(
                LEVEL_ATTRIBUTE.to_string(),
                AttributeValue::String(level.into()),
            )]),
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

/// Builds the batch for `snapshot` at `now` (Unix nanoseconds).
pub fn collect_graph(snapshot: &Snapshot, config: &GraphConfig, now: u64) -> GraphBatch {
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
            ("k8s.cluster.name", string(&config.cluster_uid)),
            ("ih.dashboard.profile", string(DASHBOARD_PROFILE)),
        ],
    );
    let namespace = snapshot.namespace_uid.as_deref().map(|uid| {
        let key = b.kubernetes("namespace", uid);
        b.entity(
            &key,
            vec![("k8s.namespace.name", string(&config.namespace))],
        );
        b.relation(&cluster, &key, RelationKind::Contains);
        key
    });

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
        node_keys.insert(name.to_string(), key);
    }

    let pod_usage: HashMap<&str, &Vec<crate::model::ContainerMetrics>> = snapshot
        .pod_metrics
        .iter()
        .flatten()
        .map(|item| (item.metadata.name.as_str(), &item.containers))
        .collect();
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
        b.entity(
            &key,
            vec![
                ("k8s.pod.name", string(&pod.metadata.name)),
                (
                    "k8s.pod.ip",
                    string(status.pod_ip.clone().unwrap_or_default()),
                ),
                ("k8s.pod.phase", string(phase)),
                ("k8s.namespace.name", string(&config.namespace)),
            ],
        );
        if let Some(namespace) = &namespace {
            b.relation(namespace, &key, RelationKind::Contains);
        }
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

        let by_container: HashMap<&str, &BTreeMap<String, String>> = pod_usage
            .get(pod.metadata.name.as_str())
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
        events: Vec::new(),
        derivations: Vec::new(),
        availability,
        dashboards: Vec::new(),
    }
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
