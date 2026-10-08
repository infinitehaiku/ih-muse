// crates/ih-muse-proto/src/lib.rs

mod cluster_state;
pub mod dashboard;
pub mod deployment;
mod element;
pub mod event_mapping;
mod element_kind;
mod graph;
mod metric;
mod node_elem_ranges;
pub mod prelude;
mod producer;
mod telemetry;
mod timestamp_resolution;
pub mod trace;
pub mod types;
mod utils;

pub use cluster_state::{NodeInfo, NodeState, NodeStatus};
pub use dashboard::{
    DashboardAppliesTo, DashboardBlock, DashboardDefinition, DashboardDefinitionError,
};
pub use element::{generate_local_element_id, ElementRegistration, NewElementsResponse};
pub use element_kind::ElementKindRegistration;
pub use graph::{
    AdditiveProjection, Availability, Derivation, EdgeDirection, Entity, EntityIdentity, EntityKey,
    Event, EventKind, GraphBatch, GraphImportError, GraphIntakeAnswer, GraphIntakeError,
    GraphIntakeRequest, GraphValidationError, JoinStatus, Observation, OrganizationId,
    ProjectionEdge, ProjectionError, ProjectionSpec, Provenance, RelationCardinality, RelationKind,
    RelationSemantics, ResidualPolicy, TemporalRelation, GRAPH_CONTRACT_REVISION,
    GRAPH_INTAKE_CONTRACT_REVISION, GRAPH_INTAKE_SCHEMA_VERSION, GRAPH_SCHEMA_VERSION,
};
pub use metric::{
    metric_id_from_code, MetricDefinition, MetricDisplay, MetricPayload, MetricQuery,
};
pub use node_elem_ranges::{GetRangesRequest, NodeElementRange, OrdRangeInc};
pub use producer::{
    ProducerBatch, ProducerEnvelope, ProducerValidationError, MAX_BATCH_MEASUREMENTS,
    PRODUCER_PROTOCOL_VERSION,
};
pub use telemetry::{
    AggregationTemporality, AttributeValue, Exemplar, ExponentialBuckets,
    ExponentialHistogramValue, HistogramValue, InstrumentationScope, KubernetesResourceKind,
    LogRecord, LogSeverity, MetricDescriptor, MetricInstrument, MetricObservation,
    MetricPersistenceRecord, MetricPoint, MetricSeriesKey, MetricUnit,
    MetricValue as TypedMetricValue, MissingReason, Number, PersistenceProjection, RatioBasis,
    RelationshipKind, Resource, ResourceKey, ResourceRelationship, SignalKind, SpanEvent, SpanKind,
    SpanLink, SpanRecord, SpanStatus, SpatialAggregation, TelemetryEnvelope, TelemetryQuery,
    TelemetryQueryPage, TelemetryQueryRecord, TelemetryQueryValidationError,
    TelemetryValidationError, TimeRange, UnitDisplay, UnixNanos, ValueDomain,
    MAX_TELEMETRY_QUERY_RECORDS, MAX_TELEMETRY_SIGNALS, TELEMETRY_CONTRACT_REVISION,
    TELEMETRY_SCHEMA_VERSION,
};
pub use timestamp_resolution::TimestampResolution;
pub use types::{ElementId, ElementKindId, LocalElementId, MetricId, MetricValue, Timestamp};
