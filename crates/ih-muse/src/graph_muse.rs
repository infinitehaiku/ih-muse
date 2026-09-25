//! Bounded native resource observation for the canonical IH temporal graph.
//!
//! This module deliberately measures only resources whose stable lifetime can
//! be established locally. Kubernetes names are discovery hints; a missing UID
//! is retained as an ambiguous relation instead of becoming an unsafe join.

use std::collections::BTreeMap;
#[cfg(target_os = "linux")]
use std::fs;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ih_muse_core::{MuseError, MuseResult};
use ih_muse_proto::{
    AttributeValue, Availability, Entity, EntityIdentity, EntityKey, GraphBatch,
    GraphIntakeRequest, InstrumentationScope, JoinStatus, MetricDescriptor, MetricInstrument,
    MetricObservation, MetricUnit, MissingReason, Number, Observation, OrganizationId, Provenance,
    RatioBasis, RelationKind, SpatialAggregation, TemporalRelation, TimeRange, TypedMetricValue,
    UnitDisplay, ValueDomain, GRAPH_CONTRACT_REVISION, GRAPH_INTAKE_CONTRACT_REVISION,
    GRAPH_INTAKE_SCHEMA_VERSION, GRAPH_SCHEMA_VERSION,
};

const SOURCE_REVISION: &str = "ih.muse.native.v1";
const DEFAULT_RESOLUTION_NS: u64 = 1_000_000_000;
const MAX_TRACKED_PROCESSES: usize = 32;
const MAX_DATABASE_REPLY_BYTES: usize = 1024 * 1024;

/// Trusted identity injected by a Kubernetes admission/downward-API profile.
///
/// `trusted` must be true only when cluster and resource UIDs originate from a
/// scoped deployment descriptor or verified Kubernetes discovery response.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KubernetesContext {
    pub trusted: bool,
    pub cluster_uid: Option<String>,
    pub namespace_uid: Option<String>,
    pub namespace_name: Option<String>,
    pub pod_uid: Option<String>,
    pub pod_name: Option<String>,
    pub container_name: Option<String>,
    pub container_id: Option<String>,
    pub node_uid: Option<String>,
    pub node_name: Option<String>,
    pub workload_uid: Option<String>,
    pub workload_kind: Option<String>,
}

impl KubernetesContext {
    /// Read an opt-in downward-API context. Missing metadata is deliberately
    /// represented as ambiguity, never guessed from a hostname or name alone.
    pub fn from_environment() -> Self {
        let value = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
        Self {
            trusted: matches!(
                value("IH_KUBERNETES_TRUSTED_CONTEXT").as_deref(),
                Some("true")
            ),
            cluster_uid: value("IH_KUBERNETES_CLUSTER_UID"),
            namespace_uid: value("IH_KUBERNETES_NAMESPACE_UID"),
            namespace_name: value("POD_NAMESPACE"),
            pod_uid: value("POD_UID"),
            pod_name: value("POD_NAME"),
            container_name: value("CONTAINER_NAME"),
            container_id: value("CONTAINER_ID"),
            node_uid: value("NODE_UID"),
            node_name: value("NODE_NAME"),
            workload_uid: value("WORKLOAD_UID"),
            workload_kind: value("WORKLOAD_KIND"),
        }
    }
}

/// One bounded native collection profile. IDs are stable configuration, not labels.
#[derive(Clone, Debug)]
pub struct GraphMuseConfig {
    pub organization: String,
    pub owner_id: String,
    pub source_id: String,
    pub environment_id: String,
    pub host_id: String,
    pub kubernetes: KubernetesContext,
    /// Optional runtime placement proven by the instrumented process itself.
    pub rustvello_runner: Option<RustvelloRunnerContext>,
    pub max_processes: usize,
}

/// Stable Rustvello runner identity paired with the process that hosts it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RustvelloRunnerContext {
    pub application_id: String,
    pub runner_id: String,
    pub pid: u32,
}

impl GraphMuseConfig {
    /// Reject broad or ambiguous producer identity before any collection starts.
    pub fn validate(&self) -> MuseResult<()> {
        for (name, value) in [
            ("organization", &self.organization),
            ("owner_id", &self.owner_id),
            ("source_id", &self.source_id),
            ("environment_id", &self.environment_id),
            ("host_id", &self.host_id),
        ] {
            if value.is_empty() || value.len() > 256 || value.contains(['\r', '\n', '\0']) {
                return Err(MuseError::Configuration(format!("invalid {name}")));
            }
        }
        if self.max_processes == 0 || self.max_processes > MAX_TRACKED_PROCESSES {
            return Err(MuseError::Configuration(format!(
                "max_processes must be in 1..={MAX_TRACKED_PROCESSES}"
            )));
        }
        if let Some(runner) = &self.rustvello_runner {
            for (name, value) in [
                ("rustvello application_id", &runner.application_id),
                ("rustvello runner_id", &runner.runner_id),
            ] {
                if value.is_empty() || value.len() > 256 || value.contains(['\r', '\n', '\0']) {
                    return Err(MuseError::Configuration(format!("invalid {name}")));
                }
            }
            if runner.pid == 0 {
                return Err(MuseError::Configuration(
                    "rustvello runner pid must be positive".into(),
                ));
            }
        }
        Ok(())
    }
}

/// Stateful collector needed to compute an honest node CPU interval ratio.
pub struct GraphMuse {
    config: GraphMuseConfig,
    boot_id: String,
    previous_node_cpu: Option<CpuTicks>,
}

/// Numeric database facts obtained through read-only interfaces or a qualified adapter.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DatabaseSnapshot {
    pub connections: Option<u64>,
    pub commands_or_transactions: Option<u64>,
    pub waits: Option<u64>,
    pub cache_hits: Option<u64>,
    pub cache_misses: Option<u64>,
    pub storage_bytes: Option<u64>,
    pub persistence_bytes: Option<u64>,
    pub replication_lag_bytes: Option<u64>,
    /// Cumulative PostgreSQL checkpoint writer time in seconds, when unit-qualified.
    pub checkpoint_write_seconds_total: Option<f64>,
}

/// Read-only Redis `INFO` response adapter. It never requests or stores keys.
pub struct RedisInfoAdapter;

impl RedisInfoAdapter {
    /// Parse a bounded `INFO ALL` response into source-neutral database facts.
    pub fn parse(response: &[u8]) -> MuseResult<DatabaseSnapshot> {
        if response.len() > MAX_DATABASE_REPLY_BYTES {
            return Err(MuseError::Validation(
                "Redis INFO response exceeds 1 MiB".into(),
            ));
        }
        let response = std::str::from_utf8(response)
            .map_err(|_| MuseError::Validation("Redis INFO response is not UTF-8".into()))?;
        let mut values = BTreeMap::new();
        for line in response.lines() {
            if let Some((key, value)) = line.split_once(':') {
                if key.len() <= 128 && value.len() <= 64 {
                    if let Ok(number) = value.trim().parse::<u64>() {
                        values.insert(key, number);
                    }
                }
            }
        }
        Ok(DatabaseSnapshot {
            connections: values.get("connected_clients").copied(),
            commands_or_transactions: values.get("total_commands_processed").copied(),
            waits: values.get("blocked_clients").copied(),
            cache_hits: values.get("keyspace_hits").copied(),
            cache_misses: values.get("keyspace_misses").copied(),
            storage_bytes: values.get("used_memory").copied(),
            persistence_bytes: values
                .get("aof_current_size")
                .copied()
                .or_else(|| values.get("rdb_last_cow_size").copied()),
            // master_repl_offset is an absolute byte position, not replication lag.
            replication_lag_bytes: None,
            checkpoint_write_seconds_total: None,
        })
    }
}

/// One unit-qualified point from the maintained PostgreSQL OTel receiver.
/// Callers must group points by database instance and namespace before conversion.
pub struct PostgresOtelPoint<'a> {
    pub name: &'a str,
    pub value: f64,
    pub unit: &'a str,
    pub kind: PostgresOtelKind,
    pub attributes: &'a [(&'a str, &'a str)],
}

/// OTel point shape and monotonicity required to interpret database values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostgresOtelKind {
    Gauge,
    CumulativeSum { monotonic: bool },
}

/// Maps a single PostgreSQL receiver scope into source-neutral database facts.
/// Transport, credentials and resource identity remain owned by the OTel receiver.
pub struct PostgresOtelAdapter;

impl PostgresOtelAdapter {
    /// Convert unit-qualified receiver points; ambiguous or repeated series stay absent.
    pub fn from_metrics(metrics: &[PostgresOtelPoint<'_>]) -> DatabaseSnapshot {
        let mut snapshot = DatabaseSnapshot::default();
        let mut commit_count = None;
        let mut rollback_count = None;
        let mut commit_series = 0;
        let mut rollback_series = 0;
        let mut size_series = 0;
        let mut checkpoint_count = 0;
        let mut replication_count = 0;
        for point in metrics {
            if !point.value.is_finite() || point.value < 0.0 || point.value >= u64::MAX as f64 {
                continue;
            }
            let value = point.value as u64;
            let attribute = |key| {
                point
                    .attributes
                    .iter()
                    .find_map(|(name, value)| (*name == key).then_some(*value))
            };
            match (point.name, point.unit, point.kind) {
                (
                    "postgresql.commits",
                    "1",
                    PostgresOtelKind::CumulativeSum { monotonic: true },
                ) if point.value.fract() == 0.0 => {
                    commit_series += 1;
                    commit_count = Some(value);
                }
                (
                    "postgresql.rollbacks",
                    "1",
                    PostgresOtelKind::CumulativeSum { monotonic: true },
                ) if point.value.fract() == 0.0 => {
                    rollback_series += 1;
                    rollback_count = Some(value);
                }
                (
                    "postgresql.db_size",
                    "By",
                    PostgresOtelKind::CumulativeSum { monotonic: false },
                ) if point.value.fract() == 0.0 => {
                    size_series += 1;
                    snapshot.storage_bytes = Some(value);
                }
                (
                    "postgresql.bgwriter.duration",
                    "ms",
                    PostgresOtelKind::CumulativeSum { monotonic: true },
                ) if attribute("type") == Some("write") => {
                    checkpoint_count += 1;
                    snapshot.checkpoint_write_seconds_total = Some(point.value / 1_000.0);
                }
                ("postgresql.replication.data_delay", "By", PostgresOtelKind::Gauge)
                    if point.value.fract() == 0.0 =>
                {
                    replication_count += 1;
                    snapshot.replication_lag_bytes = Some(value);
                }
                _ => {}
            }
        }
        if checkpoint_count != 1 {
            snapshot.checkpoint_write_seconds_total = None;
        }
        if replication_count != 1 {
            snapshot.replication_lag_bytes = None;
        }
        if size_series != 1 {
            snapshot.storage_bytes = None;
        }
        if commit_series == 1 && rollback_series == 1 {
            snapshot.commands_or_transactions = commit_count
                .zip(rollback_count)
                .and_then(|(commit, rollback)| commit.checked_add(rollback));
        }
        snapshot
    }
}

/// One unit-qualified point from the maintained MongoDB OTel receiver.
/// Callers must group points by database instance before conversion.
pub struct MongoOtelPoint<'a> {
    pub name: &'a str,
    pub value: f64,
    pub unit: &'a str,
    pub kind: MongoOtelKind,
    pub attributes: &'a [(&'a str, &'a str)],
}

/// OTel point shape and monotonicity required to interpret MongoDB values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MongoOtelKind {
    Gauge,
    CumulativeSum { monotonic: bool },
}

/// Maps a single MongoDB receiver scope into source-neutral database facts.
/// Transport, credentials, and query strings are never retained.
pub struct MongoOtelAdapter;

impl MongoOtelAdapter {
    /// Convert unit-qualified receiver points; ambiguous or repeated series stay absent.
    pub fn from_metrics(metrics: &[MongoOtelPoint<'_>]) -> DatabaseSnapshot {
        let mut snapshot = DatabaseSnapshot::default();
        let mut connection_count = 0;
        let mut op_count = 0;
        let mut size_count = 0;
        let mut hit_count = 0;
        let mut miss_count = 0;
        let mut repl_count = 0;
        for point in metrics {
            if !point.value.is_finite()
                || point.value < 0.0
                || point.value >= u64::MAX as f64
                || point.value.fract() != 0.0
            {
                continue;
            }
            let value = point.value as u64;
            let attribute = |key| {
                point
                    .attributes
                    .iter()
                    .find_map(|(name, val)| (*name == key).then_some(*val))
            };
            match (point.name, point.unit, point.kind) {
                ("mongodb.connections.current", "{connection}" | "1", MongoOtelKind::Gauge) => {
                    connection_count += 1;
                    snapshot.connections = Some(value);
                }
                (
                    "mongodb.operations",
                    "{operation}" | "1",
                    MongoOtelKind::CumulativeSum { monotonic: true },
                ) => {
                    op_count += 1;
                    snapshot.commands_or_transactions = Some(value);
                }
                (
                    "mongodb.data_size",
                    "By",
                    MongoOtelKind::Gauge | MongoOtelKind::CumulativeSum { monotonic: false },
                ) => {
                    size_count += 1;
                    snapshot.storage_bytes = Some(value);
                }
                (
                    "mongodb.cache.operations",
                    "{operation}" | "1",
                    MongoOtelKind::CumulativeSum { monotonic: true },
                ) if attribute("type") == Some("hit") => {
                    hit_count += 1;
                    snapshot.cache_hits = Some(value);
                }
                (
                    "mongodb.cache.operations",
                    "{operation}" | "1",
                    MongoOtelKind::CumulativeSum { monotonic: true },
                ) if attribute("type") == Some("miss") => {
                    miss_count += 1;
                    snapshot.cache_misses = Some(value);
                }
                ("mongodb.replication.lag.bytes", "By", MongoOtelKind::Gauge) => {
                    repl_count += 1;
                    snapshot.replication_lag_bytes = Some(value);
                }
                _ => {}
            }
        }
        if connection_count != 1 {
            snapshot.connections = None;
        }
        if op_count != 1 {
            snapshot.commands_or_transactions = None;
        }
        if size_count != 1 {
            snapshot.storage_bytes = None;
        }
        if hit_count != 1 {
            snapshot.cache_hits = None;
        }
        if miss_count != 1 {
            snapshot.cache_misses = None;
        }
        if repl_count != 1 {
            snapshot.replication_lag_bytes = None;
        }
        snapshot
    }
}

#[derive(Clone, Copy, Debug)]
struct CpuTicks {
    busy: u64,
    total: u64,
}

#[derive(Clone, Debug)]
struct ProcessFacts {
    start_time_unix_nano: u64,
    cpu_seconds: Option<f64>,
    rss_bytes: Option<u64>,
    peak_rss_bytes: Option<u64>,
}

/// One trace span from Java / Spring Boot OpenTelemetry instrumentation.
#[derive(Clone, Debug)]
pub struct ServiceTraceSpan<'a> {
    pub trace_id: &'a str,
    pub span_id: &'a str,
    pub parent_span_id: Option<&'a str>,
    pub service_name: &'a str,
    pub instance_id: Option<&'a str>,
    pub name: &'a str,
    pub start_time_unix_nano: u64,
    pub end_time_unix_nano: u64,
    pub http_route: Option<&'a str>,
    pub http_status_code: Option<u16>,
    /// Kubernetes downward-API pod UID, when present and trusted.
    pub k8s_pod_uid: Option<&'a str>,
    /// Kubernetes cluster UID, when present and trusted.
    pub k8s_cluster_uid: Option<&'a str>,
    /// Downward-API node name, when present.
    pub k8s_node_name: Option<&'a str>,
    /// Untrusted placement or ambiguous container/host join flag.
    pub ambiguous_placement: bool,
}

impl GraphMuse {
    /// Create a collector with a stable boot/lifetime discriminator.
    pub fn new(config: GraphMuseConfig) -> MuseResult<Self> {
        config.validate()?;
        Ok(Self {
            boot_id: boot_id(),
            config,
            previous_node_cpu: None,
        })
    }

    /// Measure the current process and optional bounded peer processes.
    pub fn collect(&mut self, pids: &[u32]) -> MuseResult<GraphBatch> {
        if pids.len() > self.config.max_processes {
            return Err(MuseError::Validation(format!(
                "process request exceeds {} entries",
                self.config.max_processes
            )));
        }
        let now = now_ns();
        let organization = OrganizationId(self.config.organization.clone());
        let org = key(
            organization.clone(),
            EntityIdentity::Organization {
                organization_id: self.config.organization.clone(),
            },
        );
        let environment = key(
            organization.clone(),
            EntityIdentity::StandaloneEnvironment {
                environment_id: self.config.environment_id.clone(),
            },
        );
        let host = key(
            organization.clone(),
            EntityIdentity::Host {
                environment_id: self.config.environment_id.clone(),
                host_id: self.config.host_id.clone(),
            },
        );
        let mut batch = GraphBatch {
            schema_version: GRAPH_SCHEMA_VERSION,
            contract_revision: GRAPH_CONTRACT_REVISION.into(),
            entities: vec![
                entity(org.clone(), 0, u64::MAX, BTreeMap::new()),
                entity(
                    environment.clone(),
                    0,
                    u64::MAX,
                    attrs([("ih.boot.id", self.boot_id.clone())]),
                ),
                entity(
                    host.clone(),
                    0,
                    u64::MAX,
                    attrs([("host.id", self.config.host_id.clone())]),
                ),
            ],
            relations: vec![relation(
                environment.clone(),
                org,
                RelationKind::Contains,
                0,
                u64::MAX,
                provenance(&self.config, now, JoinStatus::Resolved),
                BTreeMap::new(),
            )],
            observations: Vec::new(),
            events: Vec::new(),
            derivations: Vec::new(),
            availability: Vec::new(),
        };
        self.append_host_metrics(&mut batch, &host, now);
        self.append_processes(&mut batch, &host, pids, now)?;
        self.append_kubernetes_context(&mut batch, &host, now);
        self.append_rustvello_runner(&mut batch, now);
        batch
            .validate()
            .map_err(|error| MuseError::Validation(error.to_string()))?;
        Ok(batch)
    }

    fn append_rustvello_runner(&self, batch: &mut GraphBatch, now: u64) {
        let Some(context) = &self.config.rustvello_runner else {
            return;
        };
        let Some(process) = batch.entities.iter().find_map(|entity| {
            matches!(
                entity.key.identity,
                EntityIdentity::Process { pid, .. } if pid == context.pid
            )
            .then(|| entity.key.clone())
        }) else {
            return;
        };
        let runner = key(
            OrganizationId(self.config.organization.clone()),
            EntityIdentity::Runner {
                application_id: context.application_id.clone(),
                runner_id: context.runner_id.clone(),
            },
        );
        batch
            .entities
            .push(entity(runner.clone(), now, u64::MAX, BTreeMap::new()));
        batch.relations.push(relation(
            runner,
            process,
            RelationKind::ExecutesOn,
            now,
            u64::MAX,
            provenance(&self.config, now, JoinStatus::Resolved),
            BTreeMap::new(),
        ));
    }

    /// Wrap one batch for the shared authenticated Poet intake endpoint.
    pub fn intake_request(
        &self,
        delivery_id: String,
        batch: GraphBatch,
    ) -> MuseResult<GraphIntakeRequest> {
        let request = GraphIntakeRequest {
            schema_version: GRAPH_INTAKE_SCHEMA_VERSION,
            contract_revision: GRAPH_INTAKE_CONTRACT_REVISION.into(),
            delivery_id,
            organization: self.config.organization.clone(),
            owner_id: self.config.owner_id.clone(),
            batch,
        };
        request
            .validate()
            .map_err(|error| MuseError::Validation(error.to_string()))?;
        Ok(request)
    }

    /// Add a read-only database snapshot as graph observations for one service instance.
    pub fn append_database_snapshot(
        &self,
        batch: &mut GraphBatch,
        service_name: &str,
        instance_id: &str,
        snapshot: &DatabaseSnapshot,
    ) -> MuseResult<()> {
        self.append_database_snapshot_at_process(batch, service_name, instance_id, snapshot, None)
    }

    /// Add database observations and, when measured, bind the service to its process.
    pub fn append_database_snapshot_at_process(
        &self,
        batch: &mut GraphBatch,
        service_name: &str,
        instance_id: &str,
        snapshot: &DatabaseSnapshot,
        process_pid: Option<u32>,
    ) -> MuseResult<()> {
        validate_instance(service_name)?;
        validate_instance(instance_id)?;
        let now = now_ns();
        let entity_key = key(
            OrganizationId(self.config.organization.clone()),
            EntityIdentity::Service {
                service_name: service_name.into(),
                instance_id: Some(instance_id.into()),
            },
        );
        if !batch.entities.iter().any(|item| item.key == entity_key) {
            batch.entities.push(entity(
                entity_key.clone(),
                0,
                u64::MAX,
                attrs([("ih.database.role", service_name.to_owned())]),
            ));
        }
        if let Some(pid) = process_pid {
            let process = batch.entities.iter().find_map(|entity| {
                matches!(entity.key.identity, EntityIdentity::Process { pid: candidate, .. } if candidate == pid)
                    .then(|| entity.key.clone())
            });
            if let Some(process) = process {
                batch.relations.push(relation(
                    entity_key.clone(),
                    process,
                    RelationKind::ExecutesOn,
                    now,
                    u64::MAX,
                    provenance(&self.config, now, JoinStatus::Resolved),
                    BTreeMap::new(),
                ));
            }
        }
        for (name, unit, value, cumulative) in [
            (
                "database.connections",
                "{connection}",
                snapshot.connections,
                false,
            ),
            (
                "database.commands",
                "{operation}",
                snapshot.commands_or_transactions,
                true,
            ),
            ("database.waits", "{wait}", snapshot.waits, false),
            ("database.cache.hits", "{hit}", snapshot.cache_hits, true),
            (
                "database.cache.misses",
                "{miss}",
                snapshot.cache_misses,
                true,
            ),
            (
                "database.storage.bytes",
                "By",
                snapshot.storage_bytes,
                false,
            ),
            (
                "database.persistence.bytes",
                "By",
                snapshot.persistence_bytes,
                false,
            ),
            (
                "database.replication.lag.bytes",
                "By",
                snapshot.replication_lag_bytes,
                false,
            ),
        ] {
            let metric_value = value.map_or(
                MetricObservation::Missing {
                    reason: MissingReason::NotCollected,
                },
                |value| MetricObservation::Measured {
                    value: if cumulative {
                        TypedMetricValue::Sum(Number::U64(value))
                    } else {
                        TypedMetricValue::Gauge(Number::U64(value))
                    },
                },
            );
            batch.observations.push(observation(
                entity_key.clone(),
                descriptor(name, unit, cumulative),
                now,
                None,
                metric_value,
                provenance(&self.config, now, JoinStatus::Resolved),
            ));
        }
        batch.observations.push(observation(
            entity_key,
            descriptor("database.checkpoint.write_seconds_total", "s", true),
            now,
            None,
            snapshot.checkpoint_write_seconds_total.map_or(
                MetricObservation::Missing {
                    reason: MissingReason::NotCollected,
                },
                |seconds| MetricObservation::Measured {
                    value: TypedMetricValue::Sum(Number::F64(seconds)),
                },
            ),
            provenance(&self.config, now, JoinStatus::Resolved),
        ));
        Ok(())
    }

    /// Add one Spring Boot / Java OTel trace span and its placement relations to the canonical graph.
    /// Secrets, query strings, and untrusted placement are redacted; unmeasured task CPU remains unknown.
    pub fn append_service_trace(
        &self,
        batch: &mut GraphBatch,
        span: &ServiceTraceSpan<'_>,
    ) -> MuseResult<()> {
        if span.trace_id.is_empty()
            || span.trace_id.len() > 64
            || span.span_id.is_empty()
            || span.span_id.len() > 64
        {
            return Err(MuseError::Validation("invalid trace or span id".into()));
        }
        validate_instance(span.service_name)?;
        if let Some(instance) = span.instance_id {
            validate_instance(instance)?;
        }
        if span.start_time_unix_nano == 0 || span.end_time_unix_nano < span.start_time_unix_nano {
            return Err(MuseError::Validation("invalid span timestamps".into()));
        }
        let now = now_ns();
        let org = OrganizationId(self.config.organization.clone());
        let span_key = key(
            org.clone(),
            EntityIdentity::Span {
                trace_id: span.trace_id.into(),
                span_id: span.span_id.into(),
            },
        );
        let mut span_attrs = BTreeMap::new();
        span_attrs.insert(
            "otel.span.name".into(),
            AttributeValue::String(span.name.into()),
        );
        if let Some(route) = span.http_route {
            // Strip any sensitive query parameters or fragments
            let clean_route = route
                .split('?')
                .next()
                .unwrap_or(route)
                .split('#')
                .next()
                .unwrap_or(route);
            span_attrs.insert(
                "http.route".into(),
                AttributeValue::String(clean_route.into()),
            );
        }
        if let Some(status) = span.http_status_code {
            span_attrs.insert(
                "http.status_code".into(),
                AttributeValue::I64(status as i64),
            );
        }

        if !batch.entities.iter().any(|item| item.key == span_key) {
            batch.entities.push(entity(
                span_key.clone(),
                span.start_time_unix_nano,
                span.end_time_unix_nano,
                span_attrs,
            ));
        }

        if let Some(parent_id) = span.parent_span_id {
            if !parent_id.is_empty() && parent_id.len() <= 64 {
                let parent_key = key(
                    org.clone(),
                    EntityIdentity::Span {
                        trace_id: span.trace_id.into(),
                        span_id: parent_id.into(),
                    },
                );
                if !batch.entities.iter().any(|item| item.key == parent_key) {
                    batch.entities.push(entity(
                        parent_key.clone(),
                        span.start_time_unix_nano,
                        span.end_time_unix_nano,
                        BTreeMap::new(),
                    ));
                }
                batch.relations.push(relation(
                    span_key.clone(),
                    parent_key,
                    RelationKind::TraceParent,
                    span.start_time_unix_nano,
                    span.end_time_unix_nano,
                    provenance(&self.config, now, JoinStatus::Resolved),
                    BTreeMap::new(),
                ));
            }
        }

        let service_key = key(
            org.clone(),
            EntityIdentity::Service {
                service_name: span.service_name.into(),
                instance_id: span.instance_id.map(Into::into),
            },
        );
        if !batch.entities.iter().any(|item| item.key == service_key) {
            batch.entities.push(entity(
                service_key.clone(),
                0,
                u64::MAX,
                attrs([("service.name", span.service_name.to_owned())]),
            ));
        }
        batch.relations.push(relation(
            span_key.clone(),
            service_key.clone(),
            RelationKind::ExecutesOn,
            span.start_time_unix_nano,
            span.end_time_unix_nano,
            provenance(&self.config, now, JoinStatus::Resolved),
            BTreeMap::new(),
        ));

        // Downward API placement: link Service to Pod/Node
        if let Some(pod_uid) = span.k8s_pod_uid {
            if !pod_uid.is_empty() {
                let cluster_uid = span.k8s_cluster_uid.unwrap_or("k8s-unscoped");
                let pod_key = key(
                    org.clone(),
                    EntityIdentity::Kubernetes {
                        cluster_uid: cluster_uid.into(),
                        resource_kind: "pod".into(),
                        resource_uid: pod_uid.into(),
                    },
                );
                let join_status = if span.ambiguous_placement || span.k8s_cluster_uid.is_none() {
                    JoinStatus::Ambiguous
                } else {
                    JoinStatus::Resolved
                };
                if !batch.entities.iter().any(|item| item.key == pod_key) {
                    batch.entities.push(entity(
                        pod_key.clone(),
                        0,
                        u64::MAX,
                        attrs([("k8s.pod.uid", pod_uid.to_owned())]),
                    ));
                }
                batch.relations.push(relation(
                    service_key,
                    pod_key,
                    RelationKind::ScheduledOn,
                    span.start_time_unix_nano,
                    span.end_time_unix_nano,
                    provenance(&self.config, now, join_status),
                    BTreeMap::new(),
                ));
            }
        }
        Ok(())
    }

    fn append_host_metrics(&mut self, batch: &mut GraphBatch, host: &EntityKey, now: u64) {
        let cpu = read_node_cpu();
        let cpu_observation = node_cpu_observation(self.previous_node_cpu, cpu);
        self.previous_node_cpu = cpu;
        batch.observations.push(observation(
            host.clone(),
            ratio_descriptor("host.cpu.utilization", RatioBasis::SystemCapacity),
            now,
            None,
            cpu_observation,
            provenance(&self.config, now, JoinStatus::Resolved),
        ));
        let memory = read_memory();
        for (name, value) in [
            ("host.memory.total", memory.map(|value| value.0)),
            ("host.memory.used", memory.map(|value| value.1)),
        ] {
            batch.observations.push(observation(
                host.clone(),
                descriptor(name, "By", false),
                now,
                None,
                value.map_or(
                    MetricObservation::Missing {
                        reason: MissingReason::SourceUnavailable,
                    },
                    |value| MetricObservation::Measured {
                        value: TypedMetricValue::Gauge(Number::U64(value)),
                    },
                ),
                provenance(&self.config, now, JoinStatus::Resolved),
            ));
        }
        batch
            .availability
            .push(availability(host.clone(), now, &self.config));
    }

    fn append_processes(
        &self,
        batch: &mut GraphBatch,
        host: &EntityKey,
        pids: &[u32],
        now: u64,
    ) -> MuseResult<()> {
        let mut pids = pids.to_vec();
        if !pids.contains(&std::process::id()) {
            pids.push(std::process::id());
        }
        pids.sort_unstable();
        pids.dedup();
        if pids.len() > self.config.max_processes {
            return Err(MuseError::Validation(
                "process request exceeds configured bound".into(),
            ));
        }
        for pid in pids {
            let facts = process_facts(pid, now);
            let start = facts
                .as_ref()
                .map_or(now, |facts| facts.start_time_unix_nano);
            let process = key(
                OrganizationId(self.config.organization.clone()),
                EntityIdentity::Process {
                    environment_id: self.config.environment_id.clone(),
                    host_id: self.config.host_id.clone(),
                    pid,
                    start_time_unix_nano: start,
                },
            );
            batch.entities.push(entity(
                process.clone(),
                start,
                u64::MAX,
                attrs([("process.pid", pid.to_string())]),
            ));
            let status = if facts.is_some() {
                JoinStatus::Resolved
            } else {
                JoinStatus::MissingIdentity
            };
            batch.relations.push(relation(
                process.clone(),
                host.clone(),
                RelationKind::ExecutesOn,
                start,
                u64::MAX,
                provenance(&self.config, now, status.clone()),
                BTreeMap::new(),
            ));
            let cpu_value = facts.as_ref().and_then(|facts| facts.cpu_seconds);
            batch.observations.push(observation(
                process.clone(),
                descriptor("process.cpu.time", "s", true),
                now,
                Some(start),
                cpu_value.map_or(
                    MetricObservation::Missing {
                        reason: MissingReason::SourceUnavailable,
                    },
                    |value| MetricObservation::Measured {
                        value: TypedMetricValue::Sum(Number::F64(value)),
                    },
                ),
                provenance(&self.config, now, status.clone()),
            ));
            for (name, unit, value) in [
                (
                    "process.memory.rss",
                    "By",
                    facts.as_ref().and_then(|facts| facts.rss_bytes),
                ),
                (
                    "process.memory.peak_rss",
                    "By",
                    facts.as_ref().and_then(|facts| facts.peak_rss_bytes),
                ),
            ] {
                let metric_value = value.map_or(
                    MetricObservation::Missing {
                        reason: MissingReason::SourceUnavailable,
                    },
                    |value| MetricObservation::Measured {
                        value: TypedMetricValue::Gauge(Number::U64(value)),
                    },
                );
                batch.observations.push(observation(
                    process.clone(),
                    descriptor(name, unit, false),
                    now,
                    None,
                    metric_value,
                    provenance(&self.config, now, status.clone()),
                ));
            }
            batch
                .availability
                .push(availability(process, now, &self.config));
        }
        Ok(())
    }

    fn append_kubernetes_context(&self, batch: &mut GraphBatch, host: &EntityKey, now: u64) {
        let context = &self.config.kubernetes;
        let Some(cluster_uid) = context.cluster_uid.as_ref().filter(|_| context.trusted) else {
            return;
        };
        let organization = OrganizationId(self.config.organization.clone());
        let cluster = key(
            organization.clone(),
            EntityIdentity::Cluster {
                cluster_uid: cluster_uid.clone(),
            },
        );
        batch
            .entities
            .push(entity(cluster.clone(), 0, u64::MAX, BTreeMap::new()));
        let org = key(
            organization.clone(),
            EntityIdentity::Organization {
                organization_id: self.config.organization.clone(),
            },
        );
        batch.relations.push(relation(
            cluster.clone(),
            org,
            RelationKind::Contains,
            0,
            u64::MAX,
            provenance(&self.config, now, JoinStatus::Resolved),
            BTreeMap::new(),
        ));
        let namespace = context.namespace_uid.as_ref().map(|uid| {
            key(
                organization.clone(),
                EntityIdentity::Kubernetes {
                    cluster_uid: cluster_uid.clone(),
                    resource_kind: "namespace".into(),
                    resource_uid: uid.clone(),
                },
            )
        });
        if let Some(namespace) = &namespace {
            batch.entities.push(entity(
                namespace.clone(),
                0,
                u64::MAX,
                attrs([(
                    "k8s.namespace.name",
                    context.namespace_name.clone().unwrap_or_default(),
                )]),
            ));
            batch.relations.push(relation(
                namespace.clone(),
                cluster.clone(),
                RelationKind::Contains,
                0,
                u64::MAX,
                provenance(&self.config, now, JoinStatus::Resolved),
                BTreeMap::new(),
            ));
        }
        let pod = context.pod_uid.as_ref().map(|uid| {
            key(
                organization.clone(),
                EntityIdentity::Kubernetes {
                    cluster_uid: cluster_uid.clone(),
                    resource_kind: "pod".into(),
                    resource_uid: uid.clone(),
                },
            )
        });
        if let Some(pod) = &pod {
            batch.entities.push(entity(
                pod.clone(),
                now,
                u64::MAX,
                attrs([("k8s.pod.name", context.pod_name.clone().unwrap_or_default())]),
            ));
            if let Some(namespace) = &namespace {
                batch.relations.push(relation(
                    pod.clone(),
                    namespace.clone(),
                    RelationKind::Contains,
                    now,
                    u64::MAX,
                    provenance(&self.config, now, JoinStatus::Resolved),
                    BTreeMap::new(),
                ));
            }
            let node = context.node_uid.as_ref().map(|uid| {
                key(
                    organization.clone(),
                    EntityIdentity::Kubernetes {
                        cluster_uid: cluster_uid.clone(),
                        resource_kind: "node".into(),
                        resource_uid: uid.clone(),
                    },
                )
            });
            if let Some(node) = node {
                batch.entities.push(entity(
                    node.clone(),
                    0,
                    u64::MAX,
                    attrs([(
                        "k8s.node.name",
                        context.node_name.clone().unwrap_or_default(),
                    )]),
                ));
                batch.relations.push(relation(
                    node.clone(),
                    cluster.clone(),
                    RelationKind::Contains,
                    0,
                    u64::MAX,
                    provenance(&self.config, now, JoinStatus::Resolved),
                    BTreeMap::new(),
                ));
                batch.relations.push(relation(
                    pod.clone(),
                    node,
                    RelationKind::ScheduledOn,
                    now,
                    u64::MAX,
                    provenance(&self.config, now, JoinStatus::Resolved),
                    BTreeMap::new(),
                ));
            } else if let Some(name) = &context.node_name {
                let unresolved = key(
                    organization.clone(),
                    EntityIdentity::Other {
                        namespace: "kubernetes.unresolved.node".into(),
                        id: name.clone(),
                    },
                );
                batch
                    .entities
                    .push(entity(unresolved.clone(), now, u64::MAX, BTreeMap::new()));
                batch.relations.push(relation(
                    pod.clone(),
                    unresolved,
                    RelationKind::ScheduledOn,
                    now,
                    u64::MAX,
                    provenance(&self.config, now, JoinStatus::Ambiguous),
                    attrs([("k8s.node.name", name.clone())]),
                ));
            }
            if let (Some(name), Some(id)) = (&context.container_name, &context.container_id) {
                let container = key(
                    organization.clone(),
                    EntityIdentity::Container {
                        cluster_uid: cluster_uid.clone(),
                        pod_uid: context.pod_uid.clone().unwrap_or_default(),
                        container_name: name.clone(),
                        container_id: id.clone(),
                    },
                );
                batch
                    .entities
                    .push(entity(container.clone(), now, u64::MAX, BTreeMap::new()));
                batch.relations.push(relation(
                    container.clone(),
                    pod.clone(),
                    RelationKind::Contains,
                    now,
                    u64::MAX,
                    provenance(&self.config, now, JoinStatus::Resolved),
                    BTreeMap::new(),
                ));
                let process = key(
                    organization.clone(),
                    EntityIdentity::Process {
                        environment_id: self.config.environment_id.clone(),
                        host_id: self.config.host_id.clone(),
                        pid: std::process::id(),
                        start_time_unix_nano: process_facts(std::process::id(), now)
                            .map_or(now, |facts| facts.start_time_unix_nano),
                    },
                );
                if batch.entities.iter().any(|entity| entity.key == process) {
                    batch.relations.push(relation(
                        process,
                        container,
                        RelationKind::ExecutesOn,
                        now,
                        u64::MAX,
                        provenance(&self.config, now, JoinStatus::Resolved),
                        BTreeMap::new(),
                    ));
                }
            } else {
                // A trusted pod UID proves placement for processes observed via
                // a shared pod PID namespace even when no stable container ID
                // is available through Kubernetes' downward API.
                for process in batch
                    .entities
                    .iter()
                    .filter(|entity| matches!(entity.key.identity, EntityIdentity::Process { .. }))
                    .map(|entity| entity.key.clone())
                    .collect::<Vec<_>>()
                {
                    batch.relations.push(relation(
                        process,
                        pod.clone(),
                        RelationKind::ExecutesOn,
                        now,
                        u64::MAX,
                        provenance(&self.config, now, JoinStatus::Resolved),
                        BTreeMap::new(),
                    ));
                }
            }
        }
        let _ = host;
    }
}

fn key(organization: OrganizationId, identity: EntityIdentity) -> EntityKey {
    EntityKey {
        organization,
        identity,
    }
}

fn entity(
    key: EntityKey,
    from: u64,
    to: u64,
    attributes: BTreeMap<String, AttributeValue>,
) -> Entity {
    Entity {
        key,
        lifetime: TimeRange {
            from_unix_nano: from,
            to_unix_nano: to.max(from.saturating_add(1)),
        },
        attributes,
    }
}

fn relation(
    subject: EntityKey,
    object: EntityKey,
    kind: RelationKind,
    from: u64,
    to: u64,
    provenance: Provenance,
    attributes: BTreeMap<String, AttributeValue>,
) -> TemporalRelation {
    TemporalRelation {
        subject,
        object,
        kind,
        valid_time: TimeRange {
            from_unix_nano: from,
            to_unix_nano: to.max(from.saturating_add(1)),
        },
        provenance,
        attributes,
    }
}

fn provenance(
    config: &GraphMuseConfig,
    observed_at_unix_nano: u64,
    join_status: JoinStatus,
) -> Provenance {
    Provenance {
        source_id: config.source_id.clone(),
        source_revision: SOURCE_REVISION.into(),
        observed_at_unix_nano,
        join_status,
    }
}

fn availability(entity: EntityKey, now: u64, config: &GraphMuseConfig) -> Availability {
    Availability {
        entity,
        metric_name: None,
        available_time: TimeRange {
            from_unix_nano: now.saturating_sub(DEFAULT_RESOLUTION_NS),
            to_unix_nano: now.saturating_add(1),
        },
        resolution_unix_nano: DEFAULT_RESOLUTION_NS,
        coverage_fraction: 1.0,
        retained_from_unix_nano: now.saturating_sub(DEFAULT_RESOLUTION_NS),
        pressure_policy: None,
        provenance: provenance(config, now, JoinStatus::Resolved),
    }
}

fn observation(
    entity: EntityKey,
    descriptor: MetricDescriptor,
    now: u64,
    start: Option<u64>,
    value: MetricObservation,
    provenance: Provenance,
) -> Observation {
    Observation {
        entity,
        scope: InstrumentationScope {
            name: "ih-muse-native".into(),
            version: Some(SOURCE_REVISION.into()),
            schema_url: None,
            attributes: BTreeMap::new(),
        },
        descriptor,
        ratio_denominator: None,
        attributes: BTreeMap::new(),
        start_time_unix_nano: start,
        time_unix_nano: now,
        value,
        provenance,
    }
}

fn descriptor(name: &str, unit: &str, cumulative: bool) -> MetricDescriptor {
    MetricDescriptor {
        name: name.into(),
        description: format!("Native bounded observation for {name}"),
        unit: MetricUnit {
            ucum: unit.into(),
            display: if unit == "By" {
                UnitDisplay::Bytes
            } else {
                UnitDisplay::Number
            },
        },
        value_domain: ValueDomain::Unbounded,
        instrument: if cumulative {
            MetricInstrument::Sum {
                monotonic: true,
                temporality: ih_muse_proto::AggregationTemporality::Cumulative,
            }
        } else {
            MetricInstrument::Gauge
        },
        spatial_aggregation: SpatialAggregation::Last,
    }
}

fn ratio_descriptor(name: &str, basis: RatioBasis) -> MetricDescriptor {
    MetricDescriptor {
        name: name.into(),
        description: "Measured busy CPU ratio against the declared capacity denominator".into(),
        unit: MetricUnit {
            ucum: "1".into(),
            display: UnitDisplay::Percent,
        },
        value_domain: ValueDomain::Ratio { basis },
        instrument: MetricInstrument::Gauge,
        spatial_aggregation: SpatialAggregation::Mean,
    }
}

fn node_cpu_observation(
    previous: Option<CpuTicks>,
    current: Option<CpuTicks>,
) -> MetricObservation {
    match (previous, current) {
        (Some(previous), Some(current)) if current.total > previous.total => {
            let busy = current.busy.saturating_sub(previous.busy) as f64;
            let total = current.total.saturating_sub(previous.total) as f64;
            MetricObservation::Measured {
                value: TypedMetricValue::Gauge(Number::F64((busy / total).clamp(0.0, 1.0))),
            }
        }
        _ => MetricObservation::Missing {
            reason: MissingReason::NotYetAvailable,
        },
    }
}

fn attrs<const N: usize>(items: [(&str, String); N]) -> BTreeMap<String, AttributeValue> {
    items
        .into_iter()
        .map(|(key, value)| (key.into(), AttributeValue::String(value)))
        .collect()
}

fn validate_instance(value: &str) -> MuseResult<()> {
    if value.is_empty() || value.len() > 256 || value.contains(['@', '\r', '\n', '\0']) {
        return Err(MuseError::Validation("database identity is invalid".into()));
    }
    Ok(())
}

fn now_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_nanos()
        .min(u64::MAX as u128) as u64
}

#[cfg(target_os = "linux")]
fn boot_id() -> String {
    fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "linux-boot-id-unavailable".into())
}

#[cfg(not(target_os = "linux"))]
fn boot_id() -> String {
    "boot-id-unavailable".into()
}

#[cfg(target_os = "linux")]
fn read_node_cpu() -> Option<CpuTicks> {
    let line = fs::read_to_string("/proc/stat")
        .ok()?
        .lines()
        .next()?
        .to_owned();
    let values = line
        .split_whitespace()
        .skip(1)
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if values.len() < 4 {
        return None;
    }
    let total = values.iter().copied().sum::<u64>();
    let idle = values[3].saturating_add(*values.get(4).unwrap_or(&0));
    Some(CpuTicks {
        busy: total.saturating_sub(idle),
        total,
    })
}

#[cfg(not(target_os = "linux"))]
fn read_node_cpu() -> Option<CpuTicks> {
    None
}

#[cfg(target_os = "linux")]
fn read_memory() -> Option<(u64, u64)> {
    parse_linux_memory(&fs::read_to_string("/proc/meminfo").ok()?)
}

#[cfg(target_os = "linux")]
fn parse_linux_memory(contents: &str) -> Option<(u64, u64)> {
    let mut fields = BTreeMap::new();
    for line in contents.lines() {
        let (key, value) = line.split_once(':')?;
        let kilobytes = value.split_whitespace().next()?.parse::<u64>().ok()?;
        fields.insert(key, kilobytes.saturating_mul(1024));
    }
    let total = *fields.get("MemTotal")?;
    let available = fields
        .get("MemAvailable")
        .copied()
        .or_else(|| fields.get("MemFree").copied())?;
    Some((total, total.saturating_sub(available)))
}

#[cfg(not(target_os = "linux"))]
fn read_memory() -> Option<(u64, u64)> {
    None
}

#[cfg(target_os = "linux")]
fn process_facts(pid: u32, _now: u64) -> Option<ProcessFacts> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let close = stat.rfind(')')?;
    let values = stat
        .get(close + 2..)?
        .split_whitespace()
        .collect::<Vec<_>>();
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if ticks <= 0 || page_size <= 0 || values.len() <= 21 {
        return None;
    }
    let user = values[11].parse::<u64>().ok()?;
    let system = values[12].parse::<u64>().ok()?;
    let start_ticks = values[19].parse::<u64>().ok()?;
    let rss_pages = values[21].parse::<u64>().ok()?;
    let boot_seconds = fs::read_to_string("/proc/stat")
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("btime "))?
        .parse::<u64>()
        .ok()?;
    let peak = fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find_map(|line| line.strip_prefix("VmHWM:"))
                .and_then(|value| value.split_whitespace().next()?.parse::<u64>().ok())
                .map(|value| value.saturating_mul(1024))
        });
    Some(ProcessFacts {
        start_time_unix_nano: (boot_seconds.saturating_mul(1_000_000_000))
            .saturating_add(start_ticks.saturating_mul(1_000_000_000) / ticks as u64),
        cpu_seconds: Some(user.saturating_add(system) as f64 / ticks as f64),
        rss_bytes: Some(rss_pages.saturating_mul(page_size as u64)),
        peak_rss_bytes: peak,
    })
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn process_facts(pid: u32, _now: u64) -> Option<ProcessFacts> {
    // Darwin's `RUSAGE_INFO_V4` flavor; libc does not export the constant.
    const RUSAGE_INFO_V4: libc::c_int = 4;
    let mut process = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_bsdinfo>();
    // Darwin initializes this fixed-size buffer only when it returns its exact size.
    let written = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            process.as_mut_ptr().cast(),
            size as libc::c_int,
        )
    };
    if written != size as libc::c_int {
        return None;
    }
    let process = unsafe { process.assume_init() };
    // Resource usage of the observed `pid`, never of this collector
    // (`getrusage(RUSAGE_SELF)` would report the Muse itself).
    let mut usage = std::mem::MaybeUninit::<libc::rusage_info_v4>::zeroed();
    let usage = (unsafe {
        libc::proc_pid_rusage(
            pid as libc::c_int,
            RUSAGE_INFO_V4,
            usage.as_mut_ptr().cast::<libc::rusage_info_t>(),
        )
    } == 0)
        .then(|| unsafe { usage.assume_init() });
    Some(ProcessFacts {
        start_time_unix_nano: (process.pbi_start_tvsec as u64)
            .saturating_mul(1_000_000_000)
            .saturating_add((process.pbi_start_tvusec as u64).saturating_mul(1_000)),
        cpu_seconds: usage.as_ref().map(|usage| {
            mach_ticks_to_seconds(usage.ri_user_time.saturating_add(usage.ri_system_time))
        }),
        rss_bytes: usage.as_ref().map(|usage| usage.ri_resident_size),
        peak_rss_bytes: usage.as_ref().map(|usage| usage.ri_lifetime_max_phys_footprint),
    })
}

/// Convert Mach absolute-time ticks (the unit of `rusage_info` CPU times) to seconds.
#[cfg(target_os = "macos")]
#[allow(unsafe_code, deprecated)] // libc's Mach timebase items point to the `mach2` crate
fn mach_ticks_to_seconds(ticks: u64) -> f64 {
    let mut timebase = libc::mach_timebase_info { numer: 0, denom: 0 };
    // `mach_timebase_info` fills both fields on success; fall back to 1:1 (Intel).
    let ok = unsafe { libc::mach_timebase_info(&mut timebase) } == 0 && timebase.denom != 0;
    let (numer, denom) = if ok { (timebase.numer, timebase.denom) } else { (1, 1) };
    ticks as f64 * f64::from(numer) / f64::from(denom) / 1e9
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn process_facts(pid: u32, now: u64) -> Option<ProcessFacts> {
    (pid == std::process::id()).then(|| ProcessFacts {
        start_time_unix_nano: now,
        cpu_seconds: None,
        rss_bytes: None,
        peak_rss_bytes: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_process_facts_describe_the_observed_process_not_the_collector() {
        // A child that burns CPU while this test process stays idle.
        let mut child = std::process::Command::new("/bin/sh")
            .args(["-c", "i=0; while [ $i -lt 3000000 ]; do i=$((i+1)); done; sleep 5"])
            .spawn()
            .expect("spawn busy child");
        let pid = child.id();
        let mut child_cpu = 0.0;
        for _ in 0..100 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            child_cpu = process_facts(pid, 0).and_then(|facts| facts.cpu_seconds).unwrap_or(0.0);
            if child_cpu > 0.5 {
                break;
            }
        }
        let own = process_facts(std::process::id(), 0).expect("own facts");
        let child_facts = process_facts(pid, 0).expect("child facts");
        let _ = child.kill();
        let _ = child.wait();
        assert!(child_cpu > 0.5, "child CPU was {child_cpu}s");
        assert!(own.cpu_seconds.unwrap() < child_cpu, "facts must not be the collector's own");
        assert!(child_facts.rss_bytes.unwrap() > 0);
        assert!(child_facts.peak_rss_bytes.unwrap() >= child_facts.rss_bytes.unwrap() / 2);
    }

    fn config() -> GraphMuseConfig {
        GraphMuseConfig {
            organization: "org-a".into(),
            owner_id: "org-a".into(),
            source_id: "laptop-muse".into(),
            environment_id: "laptop-a".into(),
            host_id: "host-a".into(),
            kubernetes: KubernetesContext::default(),
            rustvello_runner: None,
            max_processes: 4,
        }
    }

    #[test]
    fn real_process_measurement_has_lifetime_identity_and_no_synthetic_root() {
        let mut muse = GraphMuse::new(config()).unwrap();
        let batch = muse.collect(&[]).unwrap();
        batch.validate().unwrap();
        assert!(batch.entities.iter().any(|entity| matches!(entity.key.identity, EntityIdentity::Process { pid, start_time_unix_nano, .. } if pid == std::process::id() && start_time_unix_nano > 0)));
        assert!(batch
            .observations
            .iter()
            .any(|observation| observation.descriptor.name == "process.memory.rss"));
        assert!(!batch.entities.iter().any(|entity| matches!(&entity.key.identity, EntityIdentity::Other { namespace, .. } if namespace == "target_muse")));
    }

    #[test]
    fn node_cpu_ratio_requires_a_real_interval() {
        assert!(matches!(
            node_cpu_observation(None, Some(CpuTicks { busy: 5, total: 10 })),
            MetricObservation::Missing {
                reason: MissingReason::NotYetAvailable
            }
        ));
        assert!(
            matches!(node_cpu_observation(Some(CpuTicks { busy: 10, total: 20 }), Some(CpuTicks { busy: 25, total: 50 })), MetricObservation::Measured { value: TypedMetricValue::Gauge(Number::F64(value)) } if (value - 0.5).abs() < f64::EPSILON)
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_memory_parser_owns_no_borrowed_input() {
        let input = String::from(
            "MemTotal:       1024 kB\nMemFree:         128 kB\nMemAvailable:    256 kB\n",
        );
        assert_eq!(parse_linux_memory(&input), Some((1_048_576, 786_432)));
        drop(input);
    }

    #[test]
    fn incomplete_node_identity_is_retained_as_ambiguous_not_joined() {
        let mut config = config();
        config.kubernetes = KubernetesContext {
            trusted: true,
            cluster_uid: Some("cluster-uid".into()),
            namespace_uid: Some("namespace-uid".into()),
            pod_uid: Some("pod-uid".into()),
            node_name: Some("node-name".into()),
            ..KubernetesContext::default()
        };
        let batch = GraphMuse::new(config).unwrap().collect(&[]).unwrap();
        assert!(batch
            .relations
            .iter()
            .any(|relation| relation.kind == RelationKind::ScheduledOn
                && relation.provenance.join_status == JoinStatus::Ambiguous));
    }

    #[test]
    fn trusted_pod_and_runner_context_bind_the_measured_process() {
        let mut config = config();
        config.kubernetes = KubernetesContext {
            trusted: true,
            cluster_uid: Some("cluster-uid".into()),
            namespace_uid: Some("namespace-uid".into()),
            pod_uid: Some("pod-uid".into()),
            pod_name: Some("worker-pod".into()),
            node_uid: Some("node-uid".into()),
            node_name: Some("node-a".into()),
            ..KubernetesContext::default()
        };
        config.rustvello_runner = Some(RustvelloRunnerContext {
            application_id: "shibuya".into(),
            runner_id: "runner-a".into(),
            pid: std::process::id(),
        });
        let batch = GraphMuse::new(config).unwrap().collect(&[]).unwrap();
        let process = batch
            .entities
            .iter()
            .find(|entity| matches!(entity.key.identity, EntityIdentity::Process { pid, .. } if pid == std::process::id()))
            .unwrap()
            .key
            .clone();
        let runner = batch
            .entities
            .iter()
            .find(|entity| matches!(&entity.key.identity, EntityIdentity::Runner { runner_id, .. } if runner_id == "runner-a"))
            .unwrap()
            .key
            .clone();
        let pod = batch
            .entities
            .iter()
            .find(|entity| matches!(&entity.key.identity, EntityIdentity::Kubernetes { resource_kind, resource_uid, .. } if resource_kind == "pod" && resource_uid == "pod-uid"))
            .unwrap()
            .key
            .clone();
        assert!(batch.relations.iter().any(|relation| {
            relation.subject == runner
                && relation.object == process
                && relation.kind == RelationKind::ExecutesOn
        }));
        assert!(batch.relations.iter().any(|relation| {
            relation.subject == process
                && relation.object == pod
                && relation.kind == RelationKind::ExecutesOn
        }));
    }

    #[test]
    fn database_adapters_are_allow_list_only_and_secret_free() {
        let redis = RedisInfoAdapter::parse(b"connected_clients:2\r\ntotal_commands_processed:7\r\nused_memory:42\r\nkey:secret\r\n").unwrap();
        assert_eq!(redis.connections, Some(2));
        assert_eq!(redis.commands_or_transactions, Some(7));
        assert_eq!(redis.storage_bytes, Some(42));
        let point = |name, value, unit, kind, attributes| PostgresOtelPoint {
            name,
            value,
            unit,
            kind,
            attributes,
        };
        let sum = PostgresOtelKind::CumulativeSum { monotonic: true };
        let qualified = PostgresOtelAdapter::from_metrics(&[
            point(
                "postgresql.bgwriter.duration",
                1250.0,
                "ms",
                sum,
                &[("type", "write")],
            ),
            point(
                "postgresql.bgwriter.duration",
                25.0,
                "ms",
                sum,
                &[("type", "sync")],
            ),
            point(
                "postgresql.replication.data_delay",
                2048.0,
                "By",
                PostgresOtelKind::Gauge,
                &[],
            ),
            point("postgresql.commits", 5.0, "1", sum, &[]),
            point("postgresql.rollbacks", 2.0, "1", sum, &[]),
            point("postgresql.wal.lag", 8.0, "s", PostgresOtelKind::Gauge, &[]),
            point("db.statement", 9.0, "1", sum, &[]),
        ]);
        assert_eq!(qualified.checkpoint_write_seconds_total, Some(1.25));
        assert_eq!(qualified.replication_lag_bytes, Some(2048));
        assert_eq!(qualified.commands_or_transactions, Some(7));
        assert_eq!(qualified.persistence_bytes, None);
        let wrong_units = PostgresOtelAdapter::from_metrics(&[
            point(
                "postgresql.bgwriter.duration",
                1250.0,
                "By",
                sum,
                &[("type", "write")],
            ),
            point(
                "postgresql.replication.data_delay",
                2048.0,
                "s",
                PostgresOtelKind::Gauge,
                &[],
            ),
        ]);
        assert_eq!(wrong_units.checkpoint_write_seconds_total, None);
        assert_eq!(wrong_units.replication_lag_bytes, None);
        let ambiguous_replica = PostgresOtelAdapter::from_metrics(&[
            point(
                "postgresql.replication.data_delay",
                4.0,
                "By",
                PostgresOtelKind::Gauge,
                &[("replication_client", "a")],
            ),
            point(
                "postgresql.replication.data_delay",
                8.0,
                "By",
                PostgresOtelKind::Gauge,
                &[("replication_client", "b")],
            ),
        ]);
        assert_eq!(ambiguous_replica.replication_lag_bytes, None);
        let ambiguous_database = PostgresOtelAdapter::from_metrics(&[
            point(
                "postgresql.commits",
                5.0,
                "1",
                sum,
                &[("db.namespace", "a")],
            ),
            point(
                "postgresql.commits",
                8.0,
                "1",
                sum,
                &[("db.namespace", "b")],
            ),
            point(
                "postgresql.rollbacks",
                2.0,
                "1",
                sum,
                &[("db.namespace", "a")],
            ),
        ]);
        assert_eq!(ambiguous_database.commands_or_transactions, None);
        let fractional_bytes = PostgresOtelAdapter::from_metrics(&[point(
            "postgresql.replication.data_delay",
            1.5,
            "By",
            PostgresOtelKind::Gauge,
            &[],
        )]);
        assert_eq!(fractional_bytes.replication_lag_bytes, None);
        assert_eq!(redis.replication_lag_bytes, None);
        let mut muse = GraphMuse::new(config()).unwrap();
        let mut graph = muse.collect(&[]).unwrap();
        muse.append_database_snapshot(&mut graph, "postgresql", "db-1", &qualified)
            .unwrap();
        let checkpoint = graph
            .observations
            .iter()
            .find(|item| item.descriptor.name == "database.checkpoint.write_seconds_total")
            .unwrap();
        assert_eq!(checkpoint.descriptor.unit.ucum, "s");
        assert!(matches!(
            checkpoint.value,
            MetricObservation::Measured {
                value: TypedMetricValue::Sum(Number::F64(1.25))
            }
        ));
        graph.validate().unwrap();
    }

    #[test]
    fn database_service_is_bound_only_to_a_measured_process() {
        let mut muse = GraphMuse::new(config()).unwrap();
        let mut batch = muse.collect(&[std::process::id()]).unwrap();
        muse.append_database_snapshot_at_process(
            &mut batch,
            "redis",
            "redis-pod-uid",
            &DatabaseSnapshot {
                connections: Some(2),
                ..DatabaseSnapshot::default()
            },
            Some(std::process::id()),
        )
        .unwrap();
        let service = batch
            .entities
            .iter()
            .find(|entity| matches!(&entity.key.identity, EntityIdentity::Service { service_name, .. } if service_name == "redis"))
            .unwrap();
        assert!(batch.relations.iter().any(|relation| {
            relation.subject == service.key
                && matches!(relation.object.identity, EntityIdentity::Process { pid, .. } if pid == std::process::id())
                && relation.kind == RelationKind::ExecutesOn
        }));
    }

    #[test]
    fn mongo_otel_adapter_maps_metrics_and_redacts_queries() {
        let point = |name, value, unit, kind, attributes| MongoOtelPoint {
            name,
            value,
            unit,
            kind,
            attributes,
        };
        let sum = MongoOtelKind::CumulativeSum { monotonic: true };
        let qualified = MongoOtelAdapter::from_metrics(&[
            point("mongodb.connections.current", 12.0, "1", MongoOtelKind::Gauge, &[]),
            point("mongodb.operations", 150.0, "{operation}", sum, &[]),
            point("mongodb.data_size", 4096.0, "By", MongoOtelKind::Gauge, &[]),
            point("mongodb.cache.operations", 80.0, "1", sum, &[("type", "hit")]),
            point("mongodb.cache.operations", 20.0, "1", sum, &[("type", "miss")]),
            point("mongodb.replication.lag.bytes", 512.0, "By", MongoOtelKind::Gauge, &[]),
            // Unsafe query string and credentials must be completely ignored/redacted
            point("db.statement", 1.0, "1", sum, &[("statement", "SELECT * FROM users WHERE password = 'secret'")]),
        ]);
        assert_eq!(qualified.connections, Some(12));
        assert_eq!(qualified.commands_or_transactions, Some(150));
        assert_eq!(qualified.storage_bytes, Some(4096));
        assert_eq!(qualified.cache_hits, Some(80));
        assert_eq!(qualified.cache_misses, Some(20));
        assert_eq!(qualified.replication_lag_bytes, Some(512));

        // Wrong units stay absent
        let wrong_units = MongoOtelAdapter::from_metrics(&[
            point("mongodb.connections.current", 12.0, "By", MongoOtelKind::Gauge, &[]),
            point("mongodb.data_size", 4096.0, "s", MongoOtelKind::Gauge, &[]),
        ]);
        assert_eq!(wrong_units.connections, None);
        assert_eq!(wrong_units.storage_bytes, None);

        // Fractional values stay absent
        let fractional = MongoOtelAdapter::from_metrics(&[
            point("mongodb.connections.current", 12.5, "1", MongoOtelKind::Gauge, &[]),
        ]);
        assert_eq!(fractional.connections, None);

        // Ambiguous duplicate series stay absent
        let ambiguous = MongoOtelAdapter::from_metrics(&[
            point("mongodb.connections.current", 12.0, "1", MongoOtelKind::Gauge, &[]),
            point("mongodb.connections.current", 15.0, "1", MongoOtelKind::Gauge, &[]),
        ]);
        assert_eq!(ambiguous.connections, None);
    }

    #[test]
    fn spring_boot_trace_spans_link_to_workload_and_placement() {
        let muse = GraphMuse::new(config()).unwrap();
        let mut graph = GraphBatch {
            schema_version: ih_muse_proto::GRAPH_SCHEMA_VERSION,
            contract_revision: ih_muse_proto::GRAPH_CONTRACT_REVISION.into(),
            entities: Vec::new(),
            relations: Vec::new(),
            observations: Vec::new(),
            events: Vec::new(),
            derivations: Vec::new(),
            availability: Vec::new(),
        };

        let span = ServiceTraceSpan {
            trace_id: "4bf92f3577b34da6a3ce929d0e0e4736",
            span_id: "00f067aa0ba902b7",
            parent_span_id: Some("5fb397be34d23b0f"),
            service_name: "spring-boot-order-service",
            instance_id: Some("order-service-pod-0"),
            name: "GET /orders/{id}",
            start_time_unix_nano: 1_000_000,
            end_time_unix_nano: 1_050_000,
            http_route: Some("/orders/123?token=secret#frag"),
            http_status_code: Some(200),
            k8s_pod_uid: Some("pod-order-service-uid"),
            k8s_cluster_uid: Some("k-lab-p2"),
            k8s_node_name: Some("pi00"),
            ambiguous_placement: false,
        };

        muse.append_service_trace(&mut graph, &span).unwrap();

        // 1. Span entity created with stripped query string
        let span_entity = graph
            .entities
            .iter()
            .find(|e| matches!(&e.key.identity, EntityIdentity::Span { span_id, .. } if span_id == "00f067aa0ba902b7"))
            .expect("span entity exists");
        assert_eq!(
            span_entity.attributes.get("http.route"),
            Some(&AttributeValue::String("/orders/123".into()))
        );

        // 2. TraceParent relation between span and parent
        assert!(graph.relations.iter().any(|r| {
            r.kind == RelationKind::TraceParent
                && matches!(&r.subject.identity, EntityIdentity::Span { span_id, .. } if span_id == "00f067aa0ba902b7")
                && matches!(&r.object.identity, EntityIdentity::Span { span_id, .. } if span_id == "5fb397be34d23b0f")
        }));

        // 3. ExecutesOn relation between span and service
        assert!(graph.relations.iter().any(|r| {
            r.kind == RelationKind::ExecutesOn
                && matches!(&r.subject.identity, EntityIdentity::Span { span_id, .. } if span_id == "00f067aa0ba902b7")
                && matches!(&r.object.identity, EntityIdentity::Service { service_name, .. } if service_name == "spring-boot-order-service")
        }));

        // 4. Downward API placement: ScheduledOn relation between service and pod
        let placement = graph
            .relations
            .iter()
            .find(|r| {
                r.kind == RelationKind::ScheduledOn
                    && matches!(&r.subject.identity, EntityIdentity::Service { service_name, .. } if service_name == "spring-boot-order-service")
                    && matches!(&r.object.identity, EntityIdentity::Kubernetes { resource_uid, .. } if resource_uid == "pod-order-service-uid")
            })
            .expect("placement relation exists");
        assert_eq!(placement.provenance.join_status, JoinStatus::Resolved);

        // 5. Ambiguous placement marks JoinStatus::Ambiguous
        let ambiguous_span = ServiceTraceSpan {
            trace_id: "4bf92f3577b34da6a3ce929d0e0e4736",
            span_id: "11f067aa0ba902b8",
            parent_span_id: None,
            service_name: "unplaced-service",
            instance_id: None,
            name: "GET /status",
            start_time_unix_nano: 2_000_000,
            end_time_unix_nano: 2_010_000,
            http_route: Some("/status"),
            http_status_code: Some(200),
            k8s_pod_uid: Some("pod-unplaced-uid"),
            k8s_cluster_uid: None, // Missing cluster makes placement ambiguous
            k8s_node_name: None,
            ambiguous_placement: true,
        };
        muse.append_service_trace(&mut graph, &ambiguous_span).unwrap();
        let unplaced_relation = graph
            .relations
            .iter()
            .find(|r| {
                r.kind == RelationKind::ScheduledOn
                    && matches!(&r.subject.identity, EntityIdentity::Service { service_name, .. } if service_name == "unplaced-service")
            })
            .expect("ambiguous placement relation exists");
        assert_eq!(unplaced_relation.provenance.join_status, JoinStatus::Ambiguous);

        // Canonical graph validation succeeds
        graph.validate().unwrap();
    }
}
