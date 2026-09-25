//! Versioned normalized telemetry exchanged after an adapter decodes its source protocol.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// Schema version for the OTLP-qualified metrics, traces, logs, and identity contract.
pub const TELEMETRY_SCHEMA_VERSION: u16 = 3;
/// Stable contract revision consumed by independently versioned repositories.
pub const TELEMETRY_CONTRACT_REVISION: &str = "ih.telemetry.v3";
/// Maximum normalized signal records accepted in one batch before transport limits apply.
pub const MAX_TELEMETRY_SIGNALS: usize = 16_384;
/// Maximum records returned by one typed query page.
pub const MAX_TELEMETRY_QUERY_RECORDS: usize = 10_000;

/// Nanoseconds since the Unix epoch, preserving OTLP event-time precision.
pub type UnixNanos = u64;

/// A half-open time interval used for resource validity and query boundaries.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TimeRange {
    pub from_unix_nano: UnixNanos,
    pub to_unix_nano: UnixNanos,
}

/// A stable resource identity that does not depend on a display name.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResourceKey {
    Host {
        host_id: String,
    },
    Kubernetes {
        cluster_uid: String,
        resource_kind: KubernetesResourceKind,
        resource_uid: String,
    },
    Process {
        host_id: String,
        pid: u32,
        start_time_unix_nano: UnixNanos,
    },
    Application {
        application_id: String,
    },
    Service {
        service_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        service_instance_id: Option<String>,
    },
    Container {
        cluster_uid: String,
        pod_uid: String,
        container_name: String,
        container_id: String,
    },
    Queue {
        application_id: String,
        queue_id: String,
    },
    Workflow {
        application_id: String,
        workflow_id: String,
    },
    Worker {
        application_id: String,
        worker_id: String,
    },
    Task {
        application_id: String,
        task_id: String,
    },
    TaskAttempt {
        application_id: String,
        task_id: String,
        invocation_id: String,
        attempt: u32,
    },
    Other {
        namespace: String,
        id: String,
    },
}

/// Kubernetes object kinds whose UID supplies identity across rename or relocation.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KubernetesResourceKind {
    Cluster,
    Node,
    Namespace,
    Workload,
    Pod,
}

/// Recursive OTLP-compatible values permitted in resources, signals, and log bodies.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum AttributeValue {
    String(String),
    Bool(bool),
    I64(i64),
    U64(u64),
    F64(f64),
    Strings(Vec<String>),
    Bools(Vec<bool>),
    I64s(Vec<i64>),
    U64s(Vec<u64>),
    F64s(Vec<f64>),
    Bytes(Vec<u8>),
    Array(Vec<AttributeValue>),
    KeyValueList(BTreeMap<String, AttributeValue>),
}

/// Descriptive resource data attached to a stable resource key.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Resource {
    pub key: ResourceKey,
    #[serde(default)]
    pub attributes: BTreeMap<String, AttributeValue>,
}

/// A relationship with explicit validity so recycled processes and pods cannot inherit edges.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ResourceRelationship {
    pub source: ResourceKey,
    pub target: ResourceKey,
    pub relationship: RelationshipKind,
    pub valid_time: TimeRange,
    #[serde(default)]
    pub attributes: BTreeMap<String, AttributeValue>,
}

/// Meaning of a directional relationship from `source` to `target`.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationshipKind {
    Contains,
    RunsOn,
    Represents,
    Executes,
    AttemptOf,
    MemberOf,
    ObservedBy,
}

/// Instrumentation library identity included in metric series and signal records.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct InstrumentationScope {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_url: Option<String>,
    #[serde(default)]
    pub attributes: BTreeMap<String, AttributeValue>,
}

/// Display semantics for a UCUM unit without converting the stored value.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MetricUnit {
    pub ucum: String,
    pub display: UnitDisplay,
}

/// Unit families needed to format values and reject unsafe default aggregation.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitDisplay {
    Number,
    Ratio,
    Percent,
    Bytes,
    Seconds,
    Celsius,
    Boolean,
    Custom,
}

/// Denominator used by ratio and percentage values.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RatioBasis {
    SystemCapacity,
    SingleLogicalCore,
    ResourceLimit,
    Parent,
}

/// Numeric domain needed to interpret bounds independently from display units.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ValueDomain {
    Unbounded,
    Boolean,
    Ratio { basis: RatioBasis },
}

/// OTLP metric instrument and its required aggregation temporality.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MetricInstrument {
    Gauge,
    Sum {
        monotonic: bool,
        temporality: AggregationTemporality,
    },
    Histogram {
        temporality: AggregationTemporality,
    },
    ExponentialHistogram {
        temporality: AggregationTemporality,
    },
}

/// Whether an aggregate covers only its interval or all observations since its start.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregationTemporality {
    Delta,
    Cumulative,
}

/// Spatial operation allowed when combining compatible resources at one timestamp.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SpatialAggregation {
    None,
    Sum,
    Mean,
    WeightedMean { weight_metric: String },
    Min,
    Max,
    Last,
}

/// Immutable metric semantics that participate in series identity.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MetricDescriptor {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub unit: MetricUnit,
    pub value_domain: ValueDomain,
    pub instrument: MetricInstrument,
    pub spatial_aggregation: SpatialAggregation,
}

/// An integer or finite floating-point measurement without early `f32` conversion.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Number {
    I64(i64),
    U64(u64),
    F64(f64),
}

/// Explicit histogram buckets in increasing-bound order.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HistogramValue {
    pub count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sum: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    pub explicit_bounds: Vec<f64>,
    pub bucket_counts: Vec<u64>,
}

/// One side of an exponential histogram around the zero bucket.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ExponentialBuckets {
    pub offset: i32,
    pub bucket_counts: Vec<u64>,
}

/// Exponential histogram data retained without conversion to explicit buckets.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ExponentialHistogramValue {
    pub count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sum: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    pub scale: i32,
    pub zero_count: u64,
    pub zero_threshold: f64,
    pub positive: ExponentialBuckets,
    pub negative: ExponentialBuckets,
}

/// Typed value whose variant must agree with the metric descriptor instrument.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum MetricValue {
    Gauge(Number),
    Sum(Number),
    Histogram(HistogramValue),
    ExponentialHistogram(ExponentialHistogramValue),
}

/// Why a source did not produce a measurement; this is never converted to zero.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MissingReason {
    NotCollected,
    Unsupported,
    PermissionDenied,
    SourceUnavailable,
    NotYetAvailable,
    Unknown,
}

/// A measured value or an explicit gap in coverage.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum MetricObservation {
    Measured { value: MetricValue },
    Missing { reason: MissingReason },
}

/// Trace correlation attached to a representative metric sample.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Exemplar {
    pub time_unix_nano: UnixNanos,
    pub value: Number,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span_id: Option<String>,
    #[serde(default)]
    pub attributes: BTreeMap<String, AttributeValue>,
}

/// One normalized metric point, including interval provenance where required.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MetricPoint {
    pub resource: ResourceKey,
    pub scope: InstrumentationScope,
    pub descriptor: MetricDescriptor,
    #[serde(default)]
    pub attributes: BTreeMap<String, AttributeValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_time_unix_nano: Option<UnixNanos>,
    pub time_unix_nano: UnixNanos,
    pub observation: MetricObservation,
    #[serde(default)]
    pub exemplars: Vec<Exemplar>,
}

/// Trace status retained independently from task-ledger state.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpanStatus {
    Unset,
    Ok,
    Error,
}

/// OpenTelemetry span kind used to distinguish local and remote work boundaries.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpanKind {
    Internal,
    Server,
    Client,
    Producer,
    Consumer,
}

/// A causal or associative link to another trace span.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SpanLink {
    pub trace_id: String,
    pub span_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_state: Option<String>,
    #[serde(default)]
    pub flags: u32,
    #[serde(default)]
    pub attributes: BTreeMap<String, AttributeValue>,
}

/// A timestamped event retained inside its owning span.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SpanEvent {
    pub name: String,
    pub time_unix_nano: UnixNanos,
    #[serde(default)]
    pub attributes: BTreeMap<String, AttributeValue>,
}

/// A normalized span with resource, task, and retry correlation attributes.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SpanRecord {
    pub resource: ResourceKey,
    pub scope: InstrumentationScope,
    pub trace_id: String,
    pub span_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_state: Option<String>,
    #[serde(default)]
    pub flags: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_span_id: Option<String>,
    pub name: String,
    pub kind: SpanKind,
    pub status: SpanStatus,
    pub start_time_unix_nano: UnixNanos,
    pub end_time_unix_nano: UnixNanos,
    #[serde(default)]
    pub attributes: BTreeMap<String, AttributeValue>,
    #[serde(default)]
    pub events: Vec<SpanEvent>,
    #[serde(default)]
    pub links: Vec<SpanLink>,
}

/// Structured log severity following the OTLP numeric range when available.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct LogSeverity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// A bounded structured log with optional trace/span and producer event identity.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct LogRecord {
    pub resource: ResourceKey,
    pub scope: InstrumentationScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_time_unix_nano: Option<UnixNanos>,
    pub observed_time_unix_nano: UnixNanos,
    pub severity: LogSeverity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<AttributeValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span_id: Option<String>,
    #[serde(default)]
    pub trace_flags: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    #[serde(default)]
    pub attributes: BTreeMap<String, AttributeValue>,
}

/// One normalized producer delivery; authenticated tenant identity is intentionally absent.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TelemetryEnvelope {
    pub schema_version: u16,
    pub producer_id: Uuid,
    pub batch_id: Uuid,
    pub sequence: u64,
    pub exported_at_unix_nano: UnixNanos,
    pub resources: Vec<Resource>,
    #[serde(default)]
    pub relationships: Vec<ResourceRelationship>,
    #[serde(default)]
    pub metrics: Vec<MetricPoint>,
    #[serde(default)]
    pub spans: Vec<SpanRecord>,
    #[serde(default)]
    pub logs: Vec<LogRecord>,
}

/// Canonical metric identity used by storage and duplicate detection.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MetricSeriesKey {
    pub resource: ResourceKey,
    pub scope: InstrumentationScope,
    pub descriptor: MetricDescriptor,
    pub attributes: BTreeMap<String, AttributeValue>,
}

/// One canonical metric record suitable for a durable typed-signal store.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MetricPersistenceRecord {
    pub series: MetricSeriesKey,
    pub start_time_unix_nano: Option<UnixNanos>,
    pub time_unix_nano: UnixNanos,
    pub observation: MetricObservation,
    pub exemplars: Vec<Exemplar>,
}

/// Deterministically ordered records expected after validation and normalization.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PersistenceProjection {
    pub schema_version: u16,
    pub producer_id: Uuid,
    pub batch_id: Uuid,
    pub sequence: u64,
    pub resources: Vec<Resource>,
    pub relationships: Vec<ResourceRelationship>,
    pub metrics: Vec<MetricPersistenceRecord>,
    pub spans: Vec<SpanRecord>,
    pub logs: Vec<LogRecord>,
}

/// Signal families selectable through the bounded typed query contract.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalKind {
    Resource,
    Relationship,
    Metric,
    Span,
    Log,
}

/// A bounded signal query; the authenticated tenant remains transport-owned.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TelemetryQuery {
    pub time_range: TimeRange,
    pub signal_kinds: BTreeSet<SignalKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<ResourceKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metric_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_name: Option<String>,
    #[serde(default)]
    pub attribute_filters: BTreeMap<String, AttributeValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_log_severity: Option<u8>,
    pub limit: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// A typed query record without flattening signal-specific fields.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "signal", content = "record", rename_all = "snake_case")]
pub enum TelemetryQueryRecord {
    Resource(Resource),
    Relationship(ResourceRelationship),
    Metric(MetricPersistenceRecord),
    Span(SpanRecord),
    Log(LogRecord),
}

/// One query page with authoritative retained bounds and explicit incompleteness.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TelemetryQueryPage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authoritative_bounds: Option<TimeRange>,
    pub records: Vec<TelemetryQueryRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub incomplete: bool,
}

/// Validation failures surfaced before a batch can be acknowledged or persisted.
#[derive(Clone, Debug, Error, PartialEq)]
pub enum TelemetryValidationError {
    #[error("unsupported telemetry schema version {0}")]
    UnsupportedVersion(u16),
    #[error("producer, batch, sequence, and export time must be present")]
    MissingDeliveryIdentity,
    #[error("telemetry batch must contain resources and stay within the signal limit")]
    InvalidSignalCount,
    #[error("resource identity is invalid: {0}")]
    InvalidResource(String),
    #[error("duplicate resource identity")]
    DuplicateResource,
    #[error("signal or relationship references an unregistered resource")]
    UnknownResource,
    #[error("tenant identity must come from authenticated transport")]
    PayloadTenantIdentity,
    #[error("invalid relationship validity interval")]
    InvalidRelationshipTime,
    #[error("invalid metric descriptor or value: {0}")]
    InvalidMetric(String),
    #[error("invalid trace identity or interval")]
    InvalidSpan,
    #[error("invalid log identity, severity, or timestamp")]
    InvalidLog,
    #[error("canonical serialization failed")]
    Serialization,
}

/// Invalid bounds, filters, or page size in a typed telemetry query.
#[derive(Clone, Debug, Error, PartialEq)]
pub enum TelemetryQueryValidationError {
    #[error("query must use a non-empty half-open time range")]
    InvalidTimeRange,
    #[error("query must select at least one signal kind")]
    MissingSignalKind,
    #[error("query page limit must be between 1 and {MAX_TELEMETRY_QUERY_RECORDS}")]
    InvalidLimit,
    #[error("query filter identity, severity, or attributes are invalid")]
    InvalidFilter,
}

impl TelemetryQuery {
    /// Validate a query before storage work or cursor lookup is scheduled.
    pub fn validate(&self) -> Result<(), TelemetryQueryValidationError> {
        if self.time_range.from_unix_nano >= self.time_range.to_unix_nano {
            return Err(TelemetryQueryValidationError::InvalidTimeRange);
        }
        if self.signal_kinds.is_empty() {
            return Err(TelemetryQueryValidationError::MissingSignalKind);
        }
        if self.limit == 0 || self.limit > MAX_TELEMETRY_QUERY_RECORDS {
            return Err(TelemetryQueryValidationError::InvalidLimit);
        }
        if self
            .metric_name
            .as_ref()
            .is_some_and(|name| name.is_empty())
            || self
                .trace_id
                .as_ref()
                .is_some_and(|id| !valid_hex_id(id, 32))
            || self.cursor.as_ref().is_some_and(|cursor| cursor.is_empty())
            || self
                .scope_name
                .as_ref()
                .is_some_and(|scope| scope.is_empty())
            || self
                .minimum_log_severity
                .is_some_and(|severity| severity > 24)
            || self
                .resource
                .as_ref()
                .is_some_and(|resource| validate_resource_key(resource).is_err())
        {
            return Err(TelemetryQueryValidationError::InvalidFilter);
        }
        if validate_attributes(&self.attribute_filters).is_err() {
            return Err(TelemetryQueryValidationError::InvalidFilter);
        }
        Ok(())
    }
}

impl TelemetryEnvelope {
    /// Validate semantic invariants shared by native Muse and future OTLP adapters.
    pub fn validate(&self) -> Result<(), TelemetryValidationError> {
        if self.schema_version != TELEMETRY_SCHEMA_VERSION {
            return Err(TelemetryValidationError::UnsupportedVersion(
                self.schema_version,
            ));
        }
        if self.producer_id.is_nil()
            || self.batch_id.is_nil()
            || self.sequence == 0
            || self.exported_at_unix_nano == 0
        {
            return Err(TelemetryValidationError::MissingDeliveryIdentity);
        }
        let signal_count = self
            .metrics
            .len()
            .checked_add(self.spans.len())
            .and_then(|count| count.checked_add(self.logs.len()))
            .and_then(|count| count.checked_add(self.relationships.len()))
            .ok_or(TelemetryValidationError::InvalidSignalCount)?;
        if self.resources.is_empty() || signal_count > MAX_TELEMETRY_SIGNALS {
            return Err(TelemetryValidationError::InvalidSignalCount);
        }

        let mut resource_keys = BTreeSet::new();
        for resource in &self.resources {
            validate_resource_key(&resource.key)?;
            validate_attributes(&resource.attributes)?;
            if !resource_keys.insert(resource.key.clone()) {
                return Err(TelemetryValidationError::DuplicateResource);
            }
        }

        for relationship in &self.relationships {
            require_resource(&resource_keys, &relationship.source)?;
            require_resource(&resource_keys, &relationship.target)?;
            if relationship.valid_time.from_unix_nano >= relationship.valid_time.to_unix_nano {
                return Err(TelemetryValidationError::InvalidRelationshipTime);
            }
            validate_attributes(&relationship.attributes)?;
        }
        let mut descriptors = BTreeMap::<String, MetricDescriptor>::new();
        for metric in &self.metrics {
            require_resource(&resource_keys, &metric.resource)?;
            validate_scope(&metric.scope)?;
            validate_attributes(&metric.attributes)?;
            validate_metric(metric)?;
            let descriptor_key = serde_json::to_string(&(
                &metric.resource,
                &metric.scope,
                &metric.descriptor.name,
                &metric.attributes,
            ))
            .map_err(|_| TelemetryValidationError::Serialization)?;
            if let Some(existing) = descriptors.insert(descriptor_key, metric.descriptor.clone()) {
                if !same_metric_identity(&existing, &metric.descriptor) {
                    return Err(TelemetryValidationError::InvalidMetric(
                        "one series has conflicting metric descriptors".into(),
                    ));
                }
            }
        }
        for span in &self.spans {
            require_resource(&resource_keys, &span.resource)?;
            validate_scope(&span.scope)?;
            validate_attributes(&span.attributes)?;
            if !valid_hex_id(&span.trace_id, 32)
                || !valid_hex_id(&span.span_id, 16)
                || span
                    .parent_span_id
                    .as_ref()
                    .is_some_and(|id| !valid_hex_id(id, 16))
                || span.name.is_empty()
                || span.start_time_unix_nano == 0
                || span.start_time_unix_nano > span.end_time_unix_nano
                || span.links.iter().any(|link| {
                    !valid_hex_id(&link.trace_id, 32) || !valid_hex_id(&link.span_id, 16)
                })
            {
                return Err(TelemetryValidationError::InvalidSpan);
            }
            for event in &span.events {
                validate_attributes(&event.attributes)?;
                if event.name.is_empty() || event.time_unix_nano == 0 {
                    return Err(TelemetryValidationError::InvalidSpan);
                }
            }
            for link in &span.links {
                validate_attributes(&link.attributes)?;
            }
        }
        for log in &self.logs {
            require_resource(&resource_keys, &log.resource)?;
            validate_scope(&log.scope)?;
            validate_attributes(&log.attributes)?;
            if log.severity.number.is_some_and(|number| number > 24)
                || log
                    .trace_id
                    .as_ref()
                    .is_some_and(|id| !valid_hex_id(id, 32))
                || log.span_id.as_ref().is_some_and(|id| !valid_hex_id(id, 16))
                || (log.span_id.is_some() && log.trace_id.is_none())
            {
                return Err(TelemetryValidationError::InvalidLog);
            }
            if let Some(body) = &log.body {
                validate_attribute_value(body, 0)?;
            }
        }
        Ok(())
    }

    /// Produce deterministic typed records; actual durable storage is implemented in LC-03.
    pub fn persistence_projection(
        &self,
    ) -> Result<PersistenceProjection, TelemetryValidationError> {
        self.validate()?;
        let mut resources = self.resources.clone();
        let mut relationships = self.relationships.clone();
        let mut descriptions = BTreeMap::<String, String>::new();
        for point in &self.metrics {
            let key = metric_stream_identity(point)?;
            let candidate = point.descriptor.description.clone();
            descriptions
                .entry(key)
                .and_modify(|current| {
                    if candidate.len() > current.len() {
                        *current = candidate.clone();
                    }
                })
                .or_insert(candidate);
        }
        let mut metrics = self
            .metrics
            .iter()
            .map(|point| {
                let mut descriptor = point.descriptor.clone();
                let identity = metric_stream_identity(point)?;
                descriptor.description = descriptions[&identity].clone();
                Ok(MetricPersistenceRecord {
                    series: MetricSeriesKey {
                        resource: point.resource.clone(),
                        scope: point.scope.clone(),
                        descriptor,
                        attributes: point.attributes.clone(),
                    },
                    start_time_unix_nano: point.start_time_unix_nano,
                    time_unix_nano: point.time_unix_nano,
                    observation: point.observation.clone(),
                    exemplars: point.exemplars.clone(),
                })
            })
            .collect::<Result<Vec<_>, TelemetryValidationError>>()?;
        let mut spans = self.spans.clone();
        let mut logs = self.logs.clone();

        sort_by_json(&mut resources)?;
        sort_by_json(&mut relationships)?;
        sort_by_json(&mut metrics)?;
        sort_by_json(&mut spans)?;
        sort_by_json(&mut logs)?;

        Ok(PersistenceProjection {
            schema_version: self.schema_version,
            producer_id: self.producer_id,
            batch_id: self.batch_id,
            sequence: self.sequence,
            resources,
            relationships,
            metrics,
            spans,
            logs,
        })
    }
}

fn require_resource(
    resources: &BTreeSet<ResourceKey>,
    key: &ResourceKey,
) -> Result<(), TelemetryValidationError> {
    if resources.contains(key) {
        Ok(())
    } else {
        Err(TelemetryValidationError::UnknownResource)
    }
}

fn validate_resource_key(key: &ResourceKey) -> Result<(), TelemetryValidationError> {
    let valid = match key {
        ResourceKey::Host { host_id } => !host_id.is_empty(),
        ResourceKey::Kubernetes {
            cluster_uid,
            resource_uid,
            ..
        } => !cluster_uid.is_empty() && !resource_uid.is_empty(),
        ResourceKey::Process {
            host_id,
            pid,
            start_time_unix_nano,
        } => !host_id.is_empty() && *pid > 0 && *start_time_unix_nano > 0,
        ResourceKey::Application { application_id } => !application_id.is_empty(),
        ResourceKey::Service {
            service_name,
            service_instance_id,
        } => {
            !service_name.is_empty()
                && service_instance_id
                    .as_ref()
                    .map_or(true, |instance| !instance.is_empty())
        }
        ResourceKey::Container {
            cluster_uid,
            pod_uid,
            container_name,
            container_id,
        } => {
            !cluster_uid.is_empty()
                && !pod_uid.is_empty()
                && !container_name.is_empty()
                && !container_id.is_empty()
        }
        ResourceKey::Queue {
            application_id,
            queue_id,
        } => !application_id.is_empty() && !queue_id.is_empty(),
        ResourceKey::Workflow {
            application_id,
            workflow_id,
        } => !application_id.is_empty() && !workflow_id.is_empty(),
        ResourceKey::Worker {
            application_id,
            worker_id,
        } => !application_id.is_empty() && !worker_id.is_empty(),
        ResourceKey::Task {
            application_id,
            task_id,
        } => !application_id.is_empty() && !task_id.is_empty(),
        ResourceKey::TaskAttempt {
            application_id,
            task_id,
            invocation_id,
            attempt: _,
        } => !application_id.is_empty() && !task_id.is_empty() && !invocation_id.is_empty(),
        ResourceKey::Other { namespace, id } => !namespace.is_empty() && !id.is_empty(),
    };
    if valid {
        Ok(())
    } else {
        Err(TelemetryValidationError::InvalidResource(format!(
            "{key:?}"
        )))
    }
}

fn validate_scope(scope: &InstrumentationScope) -> Result<(), TelemetryValidationError> {
    validate_attributes(&scope.attributes)
}

fn validate_attributes(
    attributes: &BTreeMap<String, AttributeValue>,
) -> Result<(), TelemetryValidationError> {
    if attributes
        .keys()
        .any(|key| key == "ih.tenant.id" || key == "tenant.id" || key == "service.tenant.id")
    {
        return Err(TelemetryValidationError::PayloadTenantIdentity);
    }
    if attributes
        .iter()
        .any(|(key, value)| key.is_empty() || validate_attribute_value(value, 0).is_err())
    {
        return Err(TelemetryValidationError::InvalidMetric(
            "attribute key or value is invalid".into(),
        ));
    }
    Ok(())
}

fn validate_attribute_value(
    value: &AttributeValue,
    depth: usize,
) -> Result<(), TelemetryValidationError> {
    if depth > 16 {
        return Err(TelemetryValidationError::InvalidMetric(
            "attribute nesting exceeds 16 levels".into(),
        ));
    }
    let invalid = match value {
        AttributeValue::F64(value) => !value.is_finite(),
        AttributeValue::F64s(values) => values.iter().any(|value| !value.is_finite()),
        AttributeValue::Array(values) => values
            .iter()
            .any(|value| validate_attribute_value(value, depth + 1).is_err()),
        AttributeValue::KeyValueList(values) => values.iter().any(|(key, value)| {
            key.is_empty()
                || key == "ih.tenant.id"
                || key == "tenant.id"
                || key == "service.tenant.id"
                || validate_attribute_value(value, depth + 1).is_err()
        }),
        _ => false,
    };
    if invalid {
        Err(TelemetryValidationError::InvalidMetric(
            "attribute value is invalid".into(),
        ))
    } else {
        Ok(())
    }
}

fn validate_metric(point: &MetricPoint) -> Result<(), TelemetryValidationError> {
    if point.descriptor.name.is_empty() || point.descriptor.unit.ucum.is_empty() {
        return Err(TelemetryValidationError::InvalidMetric(
            "metric name and UCUM unit are required".into(),
        ));
    }
    if point.time_unix_nano == 0
        || point
            .start_time_unix_nano
            .is_some_and(|start| start == 0 || start > point.time_unix_nano)
    {
        return Err(TelemetryValidationError::InvalidMetric(
            "metric timestamp interval is invalid".into(),
        ));
    }
    let ratio_unit = matches!(
        point.descriptor.unit.display,
        UnitDisplay::Ratio | UnitDisplay::Percent
    );
    if ratio_unit != matches!(point.descriptor.value_domain, ValueDomain::Ratio { .. })
        || (ratio_unit && point.descriptor.unit.ucum != "1")
    {
        return Err(TelemetryValidationError::InvalidMetric(
            "ratio units require an explicit ratio domain and UCUM unit 1".into(),
        ));
    }
    let boolean_unit = matches!(point.descriptor.unit.display, UnitDisplay::Boolean);
    if boolean_unit != matches!(point.descriptor.value_domain, ValueDomain::Boolean) {
        return Err(TelemetryValidationError::InvalidMetric(
            "boolean display requires a boolean value domain".into(),
        ));
    }
    if matches!(point.descriptor.unit.display, UnitDisplay::Celsius)
        && matches!(
            point.descriptor.spatial_aggregation,
            SpatialAggregation::Sum
        )
    {
        return Err(TelemetryValidationError::InvalidMetric(
            "temperature metrics cannot use spatial sum".into(),
        ));
    }
    if ratio_unit
        && matches!(
            point.descriptor.spatial_aggregation,
            SpatialAggregation::Sum
        )
        && !matches!(
            point.descriptor.value_domain,
            ValueDomain::Ratio {
                basis: RatioBasis::SingleLogicalCore
            }
        )
    {
        return Err(TelemetryValidationError::InvalidMetric(
            "only single-core ratios may be spatially summed".into(),
        ));
    }

    if let MetricObservation::Measured { value } = &point.observation {
        let compatible = matches!(
            (&point.descriptor.instrument, value),
            (MetricInstrument::Gauge, MetricValue::Gauge(_))
                | (MetricInstrument::Sum { .. }, MetricValue::Sum(_))
                | (
                    MetricInstrument::Histogram { .. },
                    MetricValue::Histogram(_)
                )
                | (
                    MetricInstrument::ExponentialHistogram { .. },
                    MetricValue::ExponentialHistogram(_)
                )
        );
        if !compatible {
            return Err(TelemetryValidationError::InvalidMetric(
                "instrument and value kinds differ".into(),
            ));
        }
        validate_metric_value(value, &point.descriptor.instrument)?;
        validate_value_domain(value, &point.descriptor.value_domain)?;
    }
    for exemplar in &point.exemplars {
        validate_number(exemplar.value)?;
        validate_attributes(&exemplar.attributes)?;
        if exemplar.time_unix_nano == 0
            || exemplar
                .trace_id
                .as_ref()
                .is_some_and(|id| !valid_hex_id(id, 32))
            || exemplar
                .span_id
                .as_ref()
                .is_some_and(|id| !valid_hex_id(id, 16))
            || (exemplar.span_id.is_some() && exemplar.trace_id.is_none())
        {
            return Err(TelemetryValidationError::InvalidMetric(
                "exemplar identity or timestamp is invalid".into(),
            ));
        }
    }
    Ok(())
}

fn metric_stream_identity(point: &MetricPoint) -> Result<String, TelemetryValidationError> {
    serde_json::to_string(&(
        &point.resource,
        &point.scope,
        &point.descriptor.name,
        &point.attributes,
    ))
    .map_err(|_| TelemetryValidationError::Serialization)
}

fn same_metric_identity(left: &MetricDescriptor, right: &MetricDescriptor) -> bool {
    left.name == right.name
        && left.unit == right.unit
        && left.value_domain == right.value_domain
        && left.instrument == right.instrument
        && left.spatial_aggregation == right.spatial_aggregation
}

fn validate_metric_value(
    value: &MetricValue,
    instrument: &MetricInstrument,
) -> Result<(), TelemetryValidationError> {
    match value {
        MetricValue::Gauge(number) => validate_number(*number),
        MetricValue::Sum(number) => {
            validate_number(*number)?;
            if matches!(
                instrument,
                MetricInstrument::Sum {
                    monotonic: true,
                    ..
                }
            ) && number_is_negative(*number)
            {
                return Err(TelemetryValidationError::InvalidMetric(
                    "monotonic sum cannot be negative".into(),
                ));
            }
            Ok(())
        }
        MetricValue::Histogram(histogram) => validate_histogram(histogram),
        MetricValue::ExponentialHistogram(histogram) => {
            if histogram.scale < -10 || histogram.scale > 20 {
                return Err(TelemetryValidationError::InvalidMetric(
                    "exponential histogram scale is outside OTLP bounds".into(),
                ));
            }
            let bucket_count = histogram
                .positive
                .bucket_counts
                .iter()
                .chain(&histogram.negative.bucket_counts)
                .try_fold(histogram.zero_count, |total, count| {
                    total.checked_add(*count)
                });
            let extrema_valid = match (histogram.min, histogram.max) {
                (Some(min), Some(max)) => min.is_finite() && max.is_finite() && min <= max,
                (None, None) => true,
                _ => false,
            };
            if bucket_count != Some(histogram.count)
                || histogram.sum.is_some_and(|value| !value.is_finite())
                || !histogram.zero_threshold.is_finite()
                || histogram.zero_threshold < 0.0
                || !extrema_valid
            {
                return Err(TelemetryValidationError::InvalidMetric(
                    "exponential histogram count or sum is invalid".into(),
                ));
            }
            Ok(())
        }
    }
}

fn validate_value_domain(
    value: &MetricValue,
    domain: &ValueDomain,
) -> Result<(), TelemetryValidationError> {
    match domain {
        ValueDomain::Unbounded => Ok(()),
        ValueDomain::Ratio { basis } => {
            let number = numeric_value(value, "ratio")?;
            let nonnegative = !number_is_negative(number);
            let at_most_one = match number {
                Number::I64(value) => value <= 1,
                Number::U64(value) => value <= 1,
                Number::F64(value) => value <= 1.0,
            };
            if !nonnegative || (!matches!(basis, RatioBasis::SingleLogicalCore) && !at_most_one) {
                return Err(TelemetryValidationError::InvalidMetric(
                    "ratio value is outside the declared denominator domain".into(),
                ));
            }
            Ok(())
        }
        ValueDomain::Boolean => {
            let number = numeric_value(value, "boolean")?;
            let valid = matches!(number, Number::I64(0 | 1) | Number::U64(0 | 1))
                || matches!(number, Number::F64(value) if value == 0.0 || value == 1.0);
            if valid {
                Ok(())
            } else {
                Err(TelemetryValidationError::InvalidMetric(
                    "boolean metric value must be zero or one".into(),
                ))
            }
        }
    }
}

fn numeric_value(value: &MetricValue, domain: &str) -> Result<Number, TelemetryValidationError> {
    match value {
        MetricValue::Gauge(number) | MetricValue::Sum(number) => Ok(*number),
        _ => Err(TelemetryValidationError::InvalidMetric(format!(
            "{domain} domains require a numeric gauge or sum"
        ))),
    }
}

fn validate_number(number: Number) -> Result<(), TelemetryValidationError> {
    if matches!(number, Number::F64(value) if !value.is_finite()) {
        Err(TelemetryValidationError::InvalidMetric(
            "floating-point values must be finite".into(),
        ))
    } else {
        Ok(())
    }
}

fn number_is_negative(number: Number) -> bool {
    matches!(number, Number::I64(value) if value < 0)
        || matches!(number, Number::F64(value) if value < 0.0)
}

fn validate_histogram(histogram: &HistogramValue) -> Result<(), TelemetryValidationError> {
    let ordered_bounds = histogram
        .explicit_bounds
        .iter()
        .all(|bound| bound.is_finite())
        && histogram
            .explicit_bounds
            .windows(2)
            .all(|window| window[0] < window[1]);
    let extrema_valid = match (histogram.min, histogram.max) {
        (Some(min), Some(max)) => min.is_finite() && max.is_finite() && min <= max,
        (None, None) => true,
        _ => false,
    };
    if histogram.bucket_counts.len() != histogram.explicit_bounds.len() + 1
        || histogram
            .bucket_counts
            .iter()
            .try_fold(0_u64, |total, count| total.checked_add(*count))
            != Some(histogram.count)
        || histogram.sum.is_some_and(|value| !value.is_finite())
        || !ordered_bounds
        || !extrema_valid
    {
        return Err(TelemetryValidationError::InvalidMetric(
            "explicit histogram shape is invalid".into(),
        ));
    }
    Ok(())
}

fn valid_hex_id(value: &str, length: usize) -> bool {
    value.len() == length
        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        && value == value.to_ascii_lowercase()
        && value.bytes().any(|byte| byte != b'0')
}

fn sort_by_json<T: Clone + Serialize>(values: &mut [T]) -> Result<(), TelemetryValidationError> {
    let mut keyed = values
        .iter()
        .map(|value| {
            serde_json::to_string(value)
                .map(|key| (key, value.clone()))
                .map_err(|_| TelemetryValidationError::Serialization)
        })
        .collect::<Result<Vec<_>, _>>()?;
    keyed.sort_by(|left, right| left.0.cmp(&right.0));
    for (slot, (_, value)) in values.iter_mut().zip(keyed) {
        *slot = value;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_identity_rejects_zero_uppercase_and_wrong_width() {
        assert!(valid_hex_id("00112233445566778899aabbccddeeff", 32));
        assert!(!valid_hex_id("0000000000000000", 16));
        assert!(!valid_hex_id("00112233445566AA", 16));
        assert!(!valid_hex_id("0011", 16));
    }
}
