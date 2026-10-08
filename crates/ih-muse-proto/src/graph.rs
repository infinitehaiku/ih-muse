//! Canonical temporal graph shared by native Muses and decoded telemetry sources.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::dashboard::{DashboardDefinition, DashboardDefinitionError, MAX_BATCH_DASHBOARDS};
use crate::{
    AttributeValue, InstrumentationScope, LogRecord, MetricDescriptor, MetricObservation,
    MetricPoint, Number, RelationshipKind as TelemetryRelationshipKind, ResourceKey, SpanRecord,
    TelemetryEnvelope, TimeRange, TypedMetricValue,
};

/// First graph contract replacing the legacy single-parent element shape.
pub const GRAPH_CONTRACT_REVISION: &str = "ih.graph.v1";
/// On-disk/interchange schema version for [`GraphBatch`].
pub const GRAPH_SCHEMA_VERSION: u16 = 1;
/// Revision for authenticated delivery of canonical graph batches to Poet.
pub const GRAPH_INTAKE_CONTRACT_REVISION: &str = "ih.graph.intake.v1";
/// Schema version for [`GraphIntakeRequest`].
pub const GRAPH_INTAKE_SCHEMA_VERSION: u16 = 1;

/// Authenticated ownership scope; payload attributes cannot supply this value.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct OrganizationId(pub String);

/// Stable identity plus lifetime discriminators needed to prevent name/PID reuse joins.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EntityIdentity {
    Organization {
        organization_id: String,
    },
    Cluster {
        cluster_uid: String,
    },
    StandaloneEnvironment {
        environment_id: String,
    },
    Host {
        environment_id: String,
        host_id: String,
    },
    Kubernetes {
        cluster_uid: String,
        resource_kind: String,
        resource_uid: String,
    },
    Container {
        cluster_uid: String,
        pod_uid: String,
        container_name: String,
        container_id: String,
    },
    Process {
        environment_id: String,
        host_id: String,
        pid: u32,
        start_time_unix_nano: u64,
    },
    Application {
        application_id: String,
    },
    Service {
        service_name: String,
        instance_id: Option<String>,
    },
    Queue {
        application_id: String,
        queue_id: String,
    },
    Workflow {
        application_id: String,
        workflow_id: String,
    },
    Request {
        request_id: String,
    },
    Invocation {
        application_id: String,
        invocation_id: String,
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
    Worker {
        application_id: String,
        worker_id: String,
    },
    Runner {
        application_id: String,
        runner_id: String,
    },
    Span {
        trace_id: String,
        span_id: String,
    },
    Other {
        namespace: String,
        id: String,
    },
}

/// Canonical key for every entity; organization scope is inseparable from identity.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct EntityKey {
    pub organization: OrganizationId,
    pub identity: EntityIdentity,
}

/// One entity lifetime and its non-authoritative discovery attributes.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Entity {
    pub key: EntityKey,
    pub lifetime: TimeRange,
    #[serde(default)]
    pub attributes: BTreeMap<String, AttributeValue>,
}

/// Why a possible join is not authoritative enough to become a relation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JoinStatus {
    Resolved,
    MissingIdentity,
    Ambiguous,
    OutsideLifetime,
    UntrustedScope,
}

/// Source and observation facts retained for a relation or derivation.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Provenance {
    pub source_id: String,
    pub source_revision: String,
    pub observed_at_unix_nano: u64,
    pub join_status: JoinStatus,
}

/// Semantic edge type. Direction is always subject to object, not implied parenthood.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    Contains,
    ScheduledOn,
    ExecutesOn,
    AttemptOf,
    InvocationOf,
    ChildInvocationOf,
    TriggeredBy,
    SelectedBy,
    DependsOn,
    Represents,
    MemberOf,
    ObservedBy,
    TraceParent,
    TraceLink,
}

/// Cardinality asserted by a relation source, independent from a selected view.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationCardinality {
    OneToOne,
    ManyToOne,
    ManyToMany,
}

/// Static safety metadata for one relation kind.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RelationSemantics {
    pub cardinality: RelationCardinality,
    pub additive_candidate: bool,
}

impl RelationKind {
    /// Describe whether an edge may be selected into an additive ownership view.
    pub fn semantics(&self) -> RelationSemantics {
        use RelationCardinality::{ManyToMany, ManyToOne, OneToOne};
        let (cardinality, additive_candidate) = match self {
            Self::Contains | Self::ScheduledOn | Self::ExecutesOn => (ManyToOne, true),
            Self::Represents | Self::AttemptOf | Self::InvocationOf => (OneToOne, false),
            Self::ChildInvocationOf
            | Self::TriggeredBy
            | Self::SelectedBy
            | Self::DependsOn
            | Self::MemberOf
            | Self::ObservedBy
            | Self::TraceParent
            | Self::TraceLink => (ManyToMany, false),
        };
        RelationSemantics {
            cardinality,
            additive_candidate,
        }
    }
}

/// Time-bounded relationship. Only resolved edges may participate in projections.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TemporalRelation {
    pub subject: EntityKey,
    pub object: EntityKey,
    pub kind: RelationKind,
    pub valid_time: TimeRange,
    pub provenance: Provenance,
    #[serde(default)]
    pub attributes: BTreeMap<String, AttributeValue>,
}

/// Metric sample normalized into the graph without changing v3 numeric semantics.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Observation {
    pub entity: EntityKey,
    pub scope: InstrumentationScope,
    pub descriptor: MetricDescriptor,
    /// Concrete denominator for entity-relative ratios; one-core ratios need none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ratio_denominator: Option<EntityKey>,
    #[serde(default)]
    pub attributes: BTreeMap<String, AttributeValue>,
    pub start_time_unix_nano: Option<u64>,
    pub time_unix_nano: u64,
    pub value: MetricObservation,
    pub provenance: Provenance,
}

/// Event kind retained without pretending durations or logs are resource consumption.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Span,
    SpanEvent,
    Log,
    Domain,
}

/// Timestamped non-metric record associated with an entity.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Event {
    pub entity: EntityKey,
    pub kind: EventKind,
    pub event_id: String,
    pub time: TimeRange,
    pub scope: InstrumentationScope,
    #[serde(default)]
    pub attributes: BTreeMap<String, AttributeValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<AttributeValue>,
    pub provenance: Provenance,
}

/// Exact input partitions and watermarks behind one derived record set.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Derivation {
    pub derivation_id: String,
    pub definition: String,
    pub definition_version: u32,
    pub source_partitions: Vec<String>,
    pub source_watermark_unix_nano: u64,
    pub output_entities: Vec<EntityKey>,
    #[serde(default)]
    pub sufficient_statistics: BTreeMap<String, AttributeValue>,
    pub provenance: Provenance,
}

/// Queryable coverage and precision; unavailable history is not a measured zero.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Availability {
    pub entity: EntityKey,
    pub metric_name: Option<String>,
    pub available_time: TimeRange,
    pub resolution_unix_nano: u64,
    pub coverage_fraction: f64,
    pub retained_from_unix_nano: u64,
    pub pressure_policy: Option<String>,
    pub provenance: Provenance,
}

/// Canonical source-independent records accepted by future native storage.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GraphBatch {
    pub schema_version: u16,
    pub contract_revision: String,
    pub entities: Vec<Entity>,
    pub relations: Vec<TemporalRelation>,
    pub observations: Vec<Observation>,
    pub events: Vec<Event>,
    pub derivations: Vec<Derivation>,
    pub availability: Vec<Availability>,
    /// Default dashboards the sending Muse defines. A Muse sends its
    /// definitions in its first batch after start, again whenever one
    /// changes, and again when the acknowledging Poet's
    /// [`GraphIntakeAnswer::definitions_epoch`] changes; receivers
    /// deduplicate by `(id, revision)`. Absent from the
    /// JSON when empty, so senders and receivers that predate the field are
    /// unaffected.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dashboards: Vec<DashboardDefinition>,
}

/// One idempotent, owner-scoped delivery of a canonical graph batch.
///
/// Authentication establishes the organization and owner. The envelope repeats
/// both values so a receiver can reject a confused or cross-tenant batch before
/// durable persistence.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GraphIntakeRequest {
    pub schema_version: u16,
    pub contract_revision: String,
    pub delivery_id: String,
    pub organization: String,
    pub owner_id: String,
    pub batch: GraphBatch,
}

/// What a Poet answers when it accepted a [`GraphIntakeRequest`] (HTTP 201).
///
/// `definitions_epoch` names the answering Poet's current store lifetime:
/// it changes whenever that Poet may no longer hold the dashboard
/// definitions it acknowledged before (a restart, a wiped store). A Muse
/// remembers the epoch of the answer that acknowledged its definitions and
/// sends them again, once, when a later answer names another epoch. Empty
/// when the Poet does not say (an empty body), which never triggers a resend.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub struct GraphIntakeAnswer {
    #[serde(default)]
    pub definitions_epoch: String,
}

/// Fail-closed reasons a graph batch cannot enter the canonical store.
#[derive(Clone, Debug, Error, PartialEq)]
pub enum GraphIntakeError {
    #[error("unsupported graph intake contract")]
    UnsupportedVersion,
    #[error("invalid graph delivery or ownership identity")]
    InvalidIdentity,
    #[error("graph delivery crosses authenticated organization scope")]
    CrossOrganization,
    #[error("invalid canonical graph: {0}")]
    InvalidGraph(String),
}

impl GraphIntakeRequest {
    /// Validate the portable envelope before it is sent or durably accepted.
    pub fn validate(&self) -> Result<(), GraphIntakeError> {
        if self.schema_version != GRAPH_INTAKE_SCHEMA_VERSION
            || self.contract_revision != GRAPH_INTAKE_CONTRACT_REVISION
        {
            return Err(GraphIntakeError::UnsupportedVersion);
        }
        if self.delivery_id.is_empty()
            || self.delivery_id.len() > 512
            || self.organization.is_empty()
            || self.organization.len() > 256
            || self.owner_id.is_empty()
            || self.owner_id.len() > 256
        {
            return Err(GraphIntakeError::InvalidIdentity);
        }
        self.batch
            .validate()
            .map_err(|error| GraphIntakeError::InvalidGraph(error.to_string()))?;
        if self
            .batch
            .entities
            .iter()
            .any(|entity| entity.key.organization.0 != self.organization)
        {
            return Err(GraphIntakeError::CrossOrganization);
        }
        Ok(())
    }
}

/// Direction used when a view promotes a semantic relation into additive parenthood.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeDirection {
    SubjectToObject,
    ObjectToSubject,
}

/// One explicit relation rule in an additive hierarchy view.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProjectionEdge {
    pub kind: RelationKind,
    pub direction: EdgeDirection,
}

/// A named additive view over a graph that may contain unrelated cycles.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProjectionSpec {
    pub name: String,
    pub root: EntityKey,
    pub edges: Vec<ProjectionEdge>,
    pub residual_policy: ResidualPolicy,
}

/// Parent-minus-children values are never fabricated by default.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResidualPolicy {
    /// A residual must arrive as its own observation with provenance.
    #[default]
    ExplicitOnly,
    /// Reject a view that would need an unobserved residual to match a parent total.
    RejectImplicit,
}

/// A validated parent-to-child projection with no cycles or additive fan-in.
#[derive(Clone, Debug, PartialEq)]
pub struct AdditiveProjection {
    pub root: EntityKey,
    pub children: BTreeMap<EntityKey, BTreeSet<EntityKey>>,
}

/// Fail-closed projection or arithmetic error.
#[derive(Clone, Debug, Error, PartialEq)]
pub enum ProjectionError {
    #[error("projection relation has unresolved or ambiguous provenance")]
    UnresolvedRelation,
    #[error("projection contains a cycle")]
    Cycle,
    #[error("one entity has multiple additive parents")]
    AdditiveFanIn,
    #[error("metric does not permit spatial summation")]
    NotSummable,
    #[error("metric descriptors or timestamps are incompatible")]
    IncompatibleObservation,
    #[error("numeric sum overflow")]
    Overflow,
    #[error("relation kind is not eligible for additive projection")]
    UnsafeRelationKind,
    #[error("one projected leaf has duplicate observations")]
    DuplicateObservation,
}

/// Invalid canonical graph data rejected before persistence or projection.
#[derive(Clone, Debug, Error, PartialEq)]
pub enum GraphValidationError {
    #[error("unsupported graph contract revision")]
    UnsupportedVersion,
    #[error("duplicate entity identity")]
    DuplicateEntity,
    #[error("entity lifetime or availability interval is invalid")]
    InvalidTime,
    #[error("relation endpoint is unknown")]
    UnknownEndpoint,
    #[error("relation crosses authenticated organization scope")]
    CrossOrganization,
    #[error("availability coverage or resolution is invalid")]
    InvalidAvailability,
    #[error("record provenance is incomplete")]
    InvalidProvenance,
    #[error("metric contains a non-finite number")]
    InvalidMetric,
    #[error(
        "at most {} dashboard definitions per batch",
        crate::dashboard::MAX_BATCH_DASHBOARDS
    )]
    TooManyDashboards,
    #[error("dashboard definition {0} appears twice in the batch")]
    DuplicateDashboard(String),
    #[error("dashboard definition {id}: {source}")]
    InvalidDashboard {
        id: String,
        source: DashboardDefinitionError,
    },
}

/// Failure while converting a validated source contract into canonical graph records.
#[derive(Clone, Debug, Error, PartialEq)]
pub enum GraphImportError {
    #[error(transparent)]
    Telemetry(#[from] crate::TelemetryValidationError),
    #[error(transparent)]
    Graph(#[from] GraphValidationError),
}

impl AdditiveProjection {
    /// Build only the selected hierarchy; unrelated selection/causal cycles are ignored.
    pub fn build(
        spec: &ProjectionSpec,
        relations: &[TemporalRelation],
        at_unix_nano: u64,
    ) -> Result<Self, ProjectionError> {
        let rules = spec
            .edges
            .iter()
            .map(|edge| (edge.kind.clone(), edge.direction.clone()))
            .collect::<BTreeMap<_, _>>();
        if spec
            .edges
            .iter()
            .any(|edge| !edge.kind.semantics().additive_candidate)
        {
            return Err(ProjectionError::UnsafeRelationKind);
        }
        let mut candidates: BTreeMap<EntityKey, Vec<(&TemporalRelation, EntityKey)>> =
            BTreeMap::new();
        for relation in relations.iter().filter(|relation| {
            relation.valid_time.from_unix_nano <= at_unix_nano
                && at_unix_nano < relation.valid_time.to_unix_nano
                && rules.contains_key(&relation.kind)
        }) {
            let (parent, child) = match rules[&relation.kind] {
                EdgeDirection::SubjectToObject => (&relation.subject, &relation.object),
                EdgeDirection::ObjectToSubject => (&relation.object, &relation.subject),
            };
            candidates
                .entry(parent.clone())
                .or_default()
                .push((relation, child.clone()));
        }
        let mut children: BTreeMap<EntityKey, BTreeSet<EntityKey>> = BTreeMap::new();
        let mut parents: BTreeMap<EntityKey, EntityKey> = BTreeMap::new();
        let mut pending = vec![spec.root.clone()];
        let mut expanded = BTreeSet::new();
        while let Some(parent) = pending.pop() {
            if !expanded.insert(parent.clone()) {
                continue;
            }
            for (relation, child) in candidates.get(&parent).into_iter().flatten() {
                if parent.organization != child.organization {
                    return Err(ProjectionError::IncompatibleObservation);
                }
                if relation.provenance.join_status != JoinStatus::Resolved {
                    return Err(ProjectionError::UnresolvedRelation);
                }
                if let Some(existing) = parents.insert(child.clone(), parent.clone()) {
                    if existing != parent {
                        return Err(ProjectionError::AdditiveFanIn);
                    }
                }
                children
                    .entry(parent.clone())
                    .or_default()
                    .insert(child.clone());
                pending.push(child.clone());
            }
        }
        fn visit(
            node: &EntityKey,
            children: &BTreeMap<EntityKey, BTreeSet<EntityKey>>,
            visiting: &mut BTreeSet<EntityKey>,
            visited: &mut BTreeSet<EntityKey>,
        ) -> Result<(), ProjectionError> {
            if !visiting.insert(node.clone()) {
                return Err(ProjectionError::Cycle);
            }
            if !visited.contains(node) {
                for child in children.get(node).into_iter().flatten() {
                    visit(child, children, visiting, visited)?;
                }
                visited.insert(node.clone());
            }
            visiting.remove(node);
            Ok(())
        }
        visit(
            &spec.root,
            &children,
            &mut BTreeSet::new(),
            &mut BTreeSet::new(),
        )?;
        Ok(Self {
            root: spec.root.clone(),
            children,
        })
    }

    /// Sum compatible leaf observations exactly once; missing stays missing upstream.
    pub fn sum(
        &self,
        observations: &[Observation],
        descriptor: &MetricDescriptor,
        at_unix_nano: u64,
    ) -> Result<Option<Number>, ProjectionError> {
        if !matches!(
            descriptor.spatial_aggregation,
            crate::SpatialAggregation::Sum
        ) {
            return Err(ProjectionError::NotSummable);
        }
        let mut reachable = BTreeSet::new();
        let mut stack = vec![self.root.clone()];
        while let Some(entity) = stack.pop() {
            if !reachable.insert(entity.clone()) {
                continue;
            }
            if let Some(children) = self.children.get(&entity) {
                stack.extend(children.iter().cloned());
            }
        }
        let leaves = reachable
            .into_iter()
            .filter(|entity| self.children.get(entity).map_or(true, BTreeSet::is_empty))
            .collect::<BTreeSet<_>>();
        let mut values = BTreeMap::<EntityKey, Option<Number>>::new();
        let requires_denominator = matches!(
            descriptor.value_domain,
            crate::ValueDomain::Ratio {
                basis: crate::RatioBasis::SystemCapacity
                    | crate::RatioBasis::ResourceLimit
                    | crate::RatioBasis::Parent
            }
        );
        let mut ratio_denominator: Option<Option<EntityKey>> = None;
        let mut interval: Option<Option<u64>> = None;
        for item in observations
            .iter()
            .filter(|item| leaves.contains(&item.entity) && item.time_unix_nano == at_unix_nano)
        {
            let value = {
                if item.descriptor != *descriptor {
                    return Err(ProjectionError::IncompatibleObservation);
                }
                let current_interval = item.start_time_unix_nano;
                if interval.is_some_and(|expected| expected != current_interval) {
                    return Err(ProjectionError::IncompatibleObservation);
                }
                interval = Some(current_interval);
                if requires_denominator && item.ratio_denominator.is_none() {
                    return Err(ProjectionError::IncompatibleObservation);
                }
                match &ratio_denominator {
                    Some(expected) if *expected != item.ratio_denominator => {
                        return Err(ProjectionError::IncompatibleObservation);
                    }
                    None => ratio_denominator = Some(item.ratio_denominator.clone()),
                    Some(_) => {}
                }
                match &item.value {
                    MetricObservation::Measured {
                        value: TypedMetricValue::Gauge(value),
                    }
                    | MetricObservation::Measured {
                        value: TypedMetricValue::Sum(value),
                    } => Ok(Some(*value)),
                    MetricObservation::Missing { .. } => Ok(None),
                    _ => Err(ProjectionError::NotSummable),
                }
            }?;
            if values.insert(item.entity.clone(), value).is_some() {
                return Err(ProjectionError::DuplicateObservation);
            }
        }
        if values.len() != leaves.len() || values.values().any(Option::is_none) {
            return Ok(None);
        }
        let numbers = values.into_values().flatten().collect::<Vec<_>>();
        match numbers.first() {
            Some(Number::I64(_)) => numbers
                .into_iter()
                .try_fold(0_i64, |sum, value| match value {
                    Number::I64(value) => sum.checked_add(value).ok_or(ProjectionError::Overflow),
                    _ => Err(ProjectionError::IncompatibleObservation),
                })
                .map(|value| Some(Number::I64(value))),
            Some(Number::U64(_)) => numbers
                .into_iter()
                .try_fold(0_u64, |sum, value| match value {
                    Number::U64(value) => sum.checked_add(value).ok_or(ProjectionError::Overflow),
                    _ => Err(ProjectionError::IncompatibleObservation),
                })
                .map(|value| Some(Number::U64(value))),
            Some(Number::F64(_)) => numbers
                .into_iter()
                .try_fold(0_f64, |sum, value| match value {
                    Number::F64(value) if value.is_finite() => {
                        let result = sum + value;
                        if result.is_finite() {
                            Ok(result)
                        } else {
                            Err(ProjectionError::Overflow)
                        }
                    }
                    _ => Err(ProjectionError::IncompatibleObservation),
                })
                .map(|value| Some(Number::F64(value))),
            None => Ok(None),
        }
    }
}

impl GraphBatch {
    /// Validate identity, organization and temporal invariants before storage.
    pub fn validate(&self) -> Result<(), GraphValidationError> {
        if self.schema_version != GRAPH_SCHEMA_VERSION
            || self.contract_revision != GRAPH_CONTRACT_REVISION
        {
            return Err(GraphValidationError::UnsupportedVersion);
        }
        let mut keys = BTreeSet::new();
        for entity in &self.entities {
            if entity.key.organization.0.is_empty()
                || !valid_identity(&entity.key)
                || entity.lifetime.from_unix_nano >= entity.lifetime.to_unix_nano
            {
                return Err(GraphValidationError::InvalidTime);
            }
            if !keys.insert(entity.key.clone()) {
                return Err(GraphValidationError::DuplicateEntity);
            }
        }
        for relation in &self.relations {
            if relation.subject.organization != relation.object.organization {
                return Err(GraphValidationError::CrossOrganization);
            }
            if !keys.contains(&relation.subject) || !keys.contains(&relation.object) {
                return Err(GraphValidationError::UnknownEndpoint);
            }
            if relation.valid_time.from_unix_nano >= relation.valid_time.to_unix_nano {
                return Err(GraphValidationError::InvalidTime);
            }
            validate_provenance(&relation.provenance)?;
        }
        if self.observations.iter().any(|observation| {
            !keys.contains(&observation.entity)
                || observation
                    .ratio_denominator
                    .as_ref()
                    .is_some_and(|entity| {
                        !keys.contains(entity)
                            || entity.organization != observation.entity.organization
                    })
                || observation.time_unix_nano == 0
                || observation
                    .start_time_unix_nano
                    .is_some_and(|start| start > observation.time_unix_nano)
        }) || self
            .events
            .iter()
            .any(|event| !keys.contains(&event.entity))
            || self
                .derivations
                .iter()
                .flat_map(|derivation| &derivation.output_entities)
                .any(|entity| !keys.contains(entity))
        {
            return Err(GraphValidationError::UnknownEndpoint);
        }
        for observation in &self.observations {
            validate_provenance(&observation.provenance)?;
            if let MetricObservation::Measured { value } = &observation.value {
                let finite = match value {
                    TypedMetricValue::Gauge(Number::F64(value))
                    | TypedMetricValue::Sum(Number::F64(value)) => value.is_finite(),
                    TypedMetricValue::Gauge(_) | TypedMetricValue::Sum(_) => true,
                    TypedMetricValue::Histogram(histogram) => {
                        histogram
                            .explicit_bounds
                            .iter()
                            .all(|value| value.is_finite())
                            && [histogram.sum, histogram.min, histogram.max]
                                .into_iter()
                                .flatten()
                                .all(f64::is_finite)
                    }
                    TypedMetricValue::ExponentialHistogram(histogram) => {
                        histogram.zero_threshold.is_finite()
                            && [histogram.sum, histogram.min, histogram.max]
                                .into_iter()
                                .flatten()
                                .all(f64::is_finite)
                    }
                };
                if !finite {
                    return Err(GraphValidationError::InvalidMetric);
                }
            }
        }
        for event in &self.events {
            validate_provenance(&event.provenance)?;
        }
        for derivation in &self.derivations {
            validate_provenance(&derivation.provenance)?;
        }
        for availability in &self.availability {
            if !keys.contains(&availability.entity)
                || availability.available_time.from_unix_nano
                    >= availability.available_time.to_unix_nano
                || availability.resolution_unix_nano == 0
                || !(0.0..=1.0).contains(&availability.coverage_fraction)
            {
                return Err(GraphValidationError::InvalidAvailability);
            }
            validate_provenance(&availability.provenance)?;
        }
        self.validate_dashboards()
    }

    /// Each definition is valid and named once; the receiver keys them by
    /// `(id, revision)`.
    fn validate_dashboards(&self) -> Result<(), GraphValidationError> {
        if self.dashboards.len() > MAX_BATCH_DASHBOARDS {
            return Err(GraphValidationError::TooManyDashboards);
        }
        let mut ids = BTreeSet::new();
        for dashboard in &self.dashboards {
            dashboard
                .validate()
                .map_err(|source| GraphValidationError::InvalidDashboard {
                    id: dashboard.id.clone(),
                    source,
                })?;
            if !ids.insert(dashboard.id.as_str()) {
                return Err(GraphValidationError::DuplicateDashboard(
                    dashboard.id.clone(),
                ));
            }
        }
        Ok(())
    }

    /// Import v3 after transport authentication supplies the organization scope.
    pub fn from_telemetry_v3(
        envelope: &TelemetryEnvelope,
        organization: OrganizationId,
        source_id: &str,
    ) -> Result<Self, GraphImportError> {
        envelope.validate()?;
        let provenance = |observed_at_unix_nano| Provenance {
            source_id: source_id.to_owned(),
            source_revision: crate::TELEMETRY_CONTRACT_REVISION.to_owned(),
            observed_at_unix_nano,
            join_status: JoinStatus::Resolved,
        };
        let key = |resource: &ResourceKey| EntityKey {
            organization: organization.clone(),
            identity: identity_from_v3(resource),
        };
        let widest = TimeRange {
            from_unix_nano: 0,
            to_unix_nano: u64::MAX,
        };
        let mut entities = envelope
            .resources
            .iter()
            .map(|resource| Entity {
                key: key(&resource.key),
                lifetime: widest.clone(),
                attributes: resource.attributes.clone(),
            })
            .collect::<Vec<_>>();
        let relations = envelope
            .relationships
            .iter()
            .map(|relation| TemporalRelation {
                subject: key(&relation.source),
                object: key(&relation.target),
                kind: relation_from_v3(&relation.relationship),
                valid_time: relation.valid_time.clone(),
                provenance: provenance(envelope.exported_at_unix_nano),
                attributes: relation.attributes.clone(),
            })
            .collect();
        let observations = envelope
            .metrics
            .iter()
            .map(|metric: &MetricPoint| Observation {
                entity: key(&metric.resource),
                scope: metric.scope.clone(),
                descriptor: metric.descriptor.clone(),
                ratio_denominator: None,
                attributes: metric.attributes.clone(),
                start_time_unix_nano: metric.start_time_unix_nano,
                time_unix_nano: metric.time_unix_nano,
                value: metric.observation.clone(),
                provenance: provenance(envelope.exported_at_unix_nano),
            })
            .collect();
        let mut events = Vec::new();
        let mut trace_relations = Vec::new();
        for span in &envelope.spans {
            let span_key = EntityKey {
                organization: organization.clone(),
                identity: EntityIdentity::Span {
                    trace_id: span.trace_id.clone(),
                    span_id: span.span_id.clone(),
                },
            };
            entities.push(Entity {
                key: span_key.clone(),
                lifetime: TimeRange {
                    from_unix_nano: span.start_time_unix_nano,
                    to_unix_nano: span.end_time_unix_nano.saturating_add(1),
                },
                attributes: span.attributes.clone(),
            });
            trace_relations.push(TemporalRelation {
                subject: span_key.clone(),
                object: key(&span.resource),
                kind: RelationKind::ExecutesOn,
                valid_time: TimeRange {
                    from_unix_nano: span.start_time_unix_nano,
                    to_unix_nano: span.end_time_unix_nano.saturating_add(1),
                },
                provenance: provenance(envelope.exported_at_unix_nano),
                attributes: BTreeMap::new(),
            });
            if let Some(parent_span_id) = &span.parent_span_id {
                let parent_key = EntityKey {
                    organization: organization.clone(),
                    identity: EntityIdentity::Span {
                        trace_id: span.trace_id.clone(),
                        span_id: parent_span_id.clone(),
                    },
                };
                entities.push(Entity {
                    key: parent_key.clone(),
                    lifetime: widest.clone(),
                    attributes: BTreeMap::new(),
                });
                trace_relations.push(TemporalRelation {
                    subject: span_key.clone(),
                    object: parent_key,
                    kind: RelationKind::TraceParent,
                    valid_time: TimeRange {
                        from_unix_nano: span.start_time_unix_nano,
                        to_unix_nano: span.end_time_unix_nano.saturating_add(1),
                    },
                    provenance: provenance(envelope.exported_at_unix_nano),
                    attributes: BTreeMap::new(),
                });
            }
            for link in &span.links {
                let linked_key = EntityKey {
                    organization: organization.clone(),
                    identity: EntityIdentity::Span {
                        trace_id: link.trace_id.clone(),
                        span_id: link.span_id.clone(),
                    },
                };
                entities.push(Entity {
                    key: linked_key.clone(),
                    lifetime: widest.clone(),
                    attributes: BTreeMap::new(),
                });
                trace_relations.push(TemporalRelation {
                    subject: span_key.clone(),
                    object: linked_key,
                    kind: RelationKind::TraceLink,
                    valid_time: TimeRange {
                        from_unix_nano: span.start_time_unix_nano,
                        to_unix_nano: span.end_time_unix_nano.saturating_add(1),
                    },
                    provenance: provenance(envelope.exported_at_unix_nano),
                    attributes: link.attributes.clone(),
                });
            }
            events.push(event_from_span(
                span,
                span_key.clone(),
                provenance(envelope.exported_at_unix_nano),
            ));
            events.extend(span.events.iter().enumerate().map(|(index, event)| Event {
                entity: span_key.clone(),
                kind: EventKind::SpanEvent,
                event_id: format!("{}:{}:event:{index}", span.trace_id, span.span_id),
                time: TimeRange {
                    from_unix_nano: event.time_unix_nano,
                    to_unix_nano: event.time_unix_nano.saturating_add(1),
                },
                scope: span.scope.clone(),
                attributes: event.attributes.clone(),
                body: Some(AttributeValue::String(event.name.clone())),
                provenance: provenance(envelope.exported_at_unix_nano),
            }));
        }
        events.extend(envelope.logs.iter().map(|log| {
            event_from_log(
                log,
                key(&log.resource),
                provenance(envelope.exported_at_unix_nano),
            )
        }));
        let mut all_relations: Vec<TemporalRelation> = relations;
        all_relations.extend(trace_relations);
        entities.sort_by(|left, right| {
            left.key.cmp(&right.key).then_with(|| {
                let left_is_placeholder = left.lifetime == widest;
                let right_is_placeholder = right.lifetime == widest;
                left_is_placeholder.cmp(&right_is_placeholder)
            })
        });
        entities.dedup_by(|left, right| left.key == right.key);
        let batch = Self {
            schema_version: GRAPH_SCHEMA_VERSION,
            contract_revision: GRAPH_CONTRACT_REVISION.to_owned(),
            entities,
            relations: all_relations,
            observations,
            events,
            derivations: Vec::new(),
            availability: Vec::new(),
            dashboards: Vec::new(),
        };
        batch.validate()?;
        Ok(batch)
    }
}

fn valid_identity(key: &EntityKey) -> bool {
    let nonempty = |value: &str| !value.is_empty();
    match &key.identity {
        EntityIdentity::Organization { organization_id } => organization_id == &key.organization.0,
        EntityIdentity::Cluster { cluster_uid } => nonempty(cluster_uid),
        EntityIdentity::StandaloneEnvironment { environment_id } => nonempty(environment_id),
        EntityIdentity::Host {
            environment_id,
            host_id,
        } => nonempty(environment_id) && nonempty(host_id),
        EntityIdentity::Kubernetes {
            cluster_uid,
            resource_kind,
            resource_uid,
        } => nonempty(cluster_uid) && nonempty(resource_kind) && nonempty(resource_uid),
        EntityIdentity::Container {
            cluster_uid,
            pod_uid,
            container_name,
            container_id,
        } => {
            nonempty(cluster_uid)
                && nonempty(pod_uid)
                && nonempty(container_name)
                && nonempty(container_id)
        }
        EntityIdentity::Process {
            environment_id,
            host_id,
            pid,
            start_time_unix_nano,
        } => nonempty(environment_id) && nonempty(host_id) && *pid > 0 && *start_time_unix_nano > 0,
        EntityIdentity::Application { application_id } => nonempty(application_id),
        EntityIdentity::Service {
            service_name,
            instance_id,
        } => nonempty(service_name) && instance_id.as_deref().map_or(true, nonempty),
        EntityIdentity::Queue {
            application_id,
            queue_id,
        } => nonempty(application_id) && nonempty(queue_id),
        EntityIdentity::Workflow {
            application_id,
            workflow_id,
        } => nonempty(application_id) && nonempty(workflow_id),
        EntityIdentity::Request { request_id } => nonempty(request_id),
        EntityIdentity::Invocation {
            application_id,
            invocation_id,
        } => nonempty(application_id) && nonempty(invocation_id),
        EntityIdentity::Task {
            application_id,
            task_id,
        } => nonempty(application_id) && nonempty(task_id),
        EntityIdentity::TaskAttempt {
            application_id,
            task_id,
            invocation_id,
            ..
        } => nonempty(application_id) && nonempty(task_id) && nonempty(invocation_id),
        EntityIdentity::Worker {
            application_id,
            worker_id,
        } => nonempty(application_id) && nonempty(worker_id),
        EntityIdentity::Runner {
            application_id,
            runner_id,
        } => nonempty(application_id) && nonempty(runner_id),
        EntityIdentity::Span { trace_id, span_id } => nonempty(trace_id) && nonempty(span_id),
        EntityIdentity::Other { namespace, id } => nonempty(namespace) && nonempty(id),
    }
}

fn validate_provenance(provenance: &Provenance) -> Result<(), GraphValidationError> {
    if provenance.source_id.is_empty()
        || provenance.source_revision.is_empty()
        || provenance.observed_at_unix_nano == 0
    {
        Err(GraphValidationError::InvalidProvenance)
    } else {
        Ok(())
    }
}

fn identity_from_v3(key: &ResourceKey) -> EntityIdentity {
    match key {
        ResourceKey::Host { host_id } => EntityIdentity::Host {
            environment_id: host_id.clone(),
            host_id: host_id.clone(),
        },
        ResourceKey::Kubernetes {
            cluster_uid,
            resource_kind,
            resource_uid,
        } => EntityIdentity::Kubernetes {
            cluster_uid: cluster_uid.clone(),
            resource_kind: format!("{resource_kind:?}").to_lowercase(),
            resource_uid: resource_uid.clone(),
        },
        ResourceKey::Process {
            host_id,
            pid,
            start_time_unix_nano,
        } => EntityIdentity::Process {
            environment_id: host_id.clone(),
            host_id: host_id.clone(),
            pid: *pid,
            start_time_unix_nano: *start_time_unix_nano,
        },
        ResourceKey::Application { application_id } => EntityIdentity::Application {
            application_id: application_id.clone(),
        },
        ResourceKey::Service {
            service_name,
            service_instance_id,
        } => EntityIdentity::Service {
            service_name: service_name.clone(),
            instance_id: service_instance_id.clone(),
        },
        ResourceKey::Container {
            cluster_uid,
            pod_uid,
            container_name,
            container_id,
        } => EntityIdentity::Container {
            cluster_uid: cluster_uid.clone(),
            pod_uid: pod_uid.clone(),
            container_name: container_name.clone(),
            container_id: container_id.clone(),
        },
        ResourceKey::Queue {
            application_id,
            queue_id,
        } => EntityIdentity::Queue {
            application_id: application_id.clone(),
            queue_id: queue_id.clone(),
        },
        ResourceKey::Workflow {
            application_id,
            workflow_id,
        } => EntityIdentity::Workflow {
            application_id: application_id.clone(),
            workflow_id: workflow_id.clone(),
        },
        ResourceKey::Worker {
            application_id,
            worker_id,
        } => EntityIdentity::Worker {
            application_id: application_id.clone(),
            worker_id: worker_id.clone(),
        },
        ResourceKey::Task {
            application_id,
            task_id,
        } => EntityIdentity::Task {
            application_id: application_id.clone(),
            task_id: task_id.clone(),
        },
        ResourceKey::TaskAttempt {
            application_id,
            task_id,
            invocation_id,
            attempt,
        } => EntityIdentity::TaskAttempt {
            application_id: application_id.clone(),
            task_id: task_id.clone(),
            invocation_id: invocation_id.clone(),
            attempt: *attempt,
        },
        ResourceKey::Other { namespace, id } => EntityIdentity::Other {
            namespace: namespace.clone(),
            id: id.clone(),
        },
    }
}

fn relation_from_v3(kind: &TelemetryRelationshipKind) -> RelationKind {
    match kind {
        TelemetryRelationshipKind::Contains => RelationKind::Contains,
        TelemetryRelationshipKind::RunsOn => RelationKind::ScheduledOn,
        TelemetryRelationshipKind::Represents => RelationKind::Represents,
        TelemetryRelationshipKind::Executes => RelationKind::ExecutesOn,
        TelemetryRelationshipKind::AttemptOf => RelationKind::AttemptOf,
        TelemetryRelationshipKind::MemberOf => RelationKind::MemberOf,
        TelemetryRelationshipKind::ObservedBy => RelationKind::ObservedBy,
    }
}

fn event_from_span(span: &SpanRecord, entity: EntityKey, provenance: Provenance) -> Event {
    Event {
        entity,
        kind: EventKind::Span,
        event_id: format!("{}:{}", span.trace_id, span.span_id),
        time: TimeRange {
            from_unix_nano: span.start_time_unix_nano,
            to_unix_nano: span.end_time_unix_nano.saturating_add(1),
        },
        scope: span.scope.clone(),
        attributes: span.attributes.clone(),
        body: Some(AttributeValue::String(span.name.clone())),
        provenance,
    }
}

fn event_from_log(log: &LogRecord, entity: EntityKey, provenance: Provenance) -> Event {
    let time = log
        .event_time_unix_nano
        .unwrap_or(log.observed_time_unix_nano);
    Event {
        entity,
        kind: EventKind::Log,
        event_id: log
            .event_id
            .clone()
            .unwrap_or_else(|| format!("observed:{}", log.observed_time_unix_nano)),
        time: TimeRange {
            from_unix_nano: time,
            to_unix_nano: time.saturating_add(1),
        },
        scope: log.scope.clone(),
        attributes: log.attributes.clone(),
        body: log.body.clone(),
        provenance,
    }
}
