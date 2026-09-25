use std::collections::{BTreeMap, BTreeSet};

use ih_muse_proto::{
    AdditiveProjection, AggregationTemporality, EdgeDirection, EntityIdentity, EntityKey,
    GraphBatch, GraphValidationError, InstrumentationScope, JoinStatus, MetricDescriptor,
    MetricInstrument, MetricObservation, MetricUnit, Number, Observation, OrganizationId,
    ProjectionEdge, ProjectionError, ProjectionSpec, Provenance, RatioBasis, RelationKind,
    ResidualPolicy, SpatialAggregation, TelemetryEnvelope, TimeRange, TypedMetricValue,
    UnitDisplay, ValueDomain, GRAPH_CONTRACT_REVISION,
};
use serde::Deserialize;

const AT: u64 = 1_788_876_000_000_000_000;

fn fixture(path: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/{path}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

#[derive(Deserialize)]
struct Expected {
    contract_revision: String,
    schema_version: u16,
    organization: String,
    entity_count: usize,
    relation_count: usize,
    observation_count: usize,
    event_count: usize,
    attempts: Vec<u32>,
    relation_kinds: Vec<String>,
}

#[derive(Deserialize)]
struct ExpectedViews {
    organization: String,
    entities: Vec<String>,
    placement_view: Vec<String>,
    workload_view: Vec<String>,
    independent_environment_view: Vec<String>,
    causal_relations: Vec<String>,
    accounting: String,
}

fn key(id: &str) -> EntityKey {
    EntityKey {
        organization: OrganizationId("org-a".into()),
        identity: EntityIdentity::Other {
            namespace: "test".into(),
            id: id.into(),
        },
    }
}

fn provenance(status: JoinStatus) -> Provenance {
    Provenance {
        source_id: "fixture".into(),
        source_revision: GRAPH_CONTRACT_REVISION.into(),
        observed_at_unix_nano: AT + 50,
        join_status: status,
    }
}

fn relation(subject: &str, object: &str, kind: RelationKind) -> ih_muse_proto::TemporalRelation {
    ih_muse_proto::TemporalRelation {
        subject: key(subject),
        object: key(object),
        kind,
        valid_time: TimeRange {
            from_unix_nano: AT,
            to_unix_nano: AT + 100,
        },
        provenance: provenance(JoinStatus::Resolved),
        attributes: BTreeMap::new(),
    }
}

fn descriptor() -> MetricDescriptor {
    MetricDescriptor {
        name: "process.cpu.utilization".into(),
        description: "CPU share of one logical core".into(),
        unit: MetricUnit {
            ucum: "1".into(),
            display: UnitDisplay::Percent,
        },
        value_domain: ValueDomain::Ratio {
            basis: RatioBasis::SingleLogicalCore,
        },
        instrument: MetricInstrument::Sum {
            monotonic: true,
            temporality: AggregationTemporality::Delta,
        },
        spatial_aggregation: SpatialAggregation::Sum,
    }
}

fn observation(entity: &str, value: u64) -> Observation {
    Observation {
        entity: key(entity),
        scope: InstrumentationScope {
            name: "fixture".into(),
            version: None,
            schema_url: None,
            attributes: BTreeMap::new(),
        },
        descriptor: descriptor(),
        ratio_denominator: None,
        attributes: BTreeMap::new(),
        start_time_unix_nano: Some(AT - 10),
        time_unix_nano: AT + 10,
        value: MetricObservation::Measured {
            value: TypedMetricValue::Sum(Number::U64(value)),
        },
        provenance: provenance(JoinStatus::Resolved),
    }
}

#[test]
fn real_sdk_fixture_imports_to_readable_expected_graph() {
    let telemetry: TelemetryEnvelope =
        serde_json::from_str(&fixture("telemetry-v3/task-retry.json")).unwrap();
    let expected: Expected =
        serde_json::from_str(&fixture("graph-v1/task-retry.expected.json")).unwrap();
    let graph = GraphBatch::from_telemetry_v3(
        &telemetry,
        OrganizationId(expected.organization.clone()),
        "rust-sdk-fixture",
    )
    .unwrap();
    graph.validate().unwrap();

    let mut attempts = graph
        .entities
        .iter()
        .filter_map(|entity| match entity.key.identity {
            EntityIdentity::TaskAttempt { attempt, .. } => Some(attempt),
            _ => None,
        })
        .collect::<Vec<_>>();
    attempts.sort_unstable();
    let relation_kinds = graph
        .relations
        .iter()
        .map(|relation| {
            serde_json::to_value(&relation.kind)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    assert_eq!(graph.contract_revision, expected.contract_revision);
    assert_eq!(graph.schema_version, expected.schema_version);
    assert_eq!(graph.entities.len(), expected.entity_count);
    assert_eq!(graph.relations.len(), expected.relation_count);
    assert_eq!(graph.observations.len(), expected.observation_count);
    assert_eq!(graph.events.len(), expected.event_count);
    assert_eq!(attempts, expected.attempts);
    assert_eq!(relation_kinds, expected.relation_kinds);
    assert!(graph
        .entities
        .iter()
        .all(|entity| entity.key.organization.0 == expected.organization));

    let cpu = graph
        .observations
        .iter()
        .find(|observation| observation.descriptor.name == "process.cpu.utilization")
        .unwrap();
    assert_eq!(cpu.descriptor.unit.ucum, "1");
    assert!(matches!(
        cpu.value,
        MetricObservation::Measured {
            value: TypedMetricValue::Gauge(Number::F64(value))
        } if value == 0.0
    ));

    let network = graph
        .observations
        .iter()
        .find(|observation| observation.descriptor.name == "process.network.io")
        .unwrap();
    assert!(matches!(network.value, MetricObservation::Missing { .. }));

    let completed = graph
        .observations
        .iter()
        .filter(|observation| observation.descriptor.name == "task.attempt.completed")
        .collect::<Vec<_>>();
    assert_eq!(completed.len(), 2);
    assert!(completed.iter().all(|observation| matches!(
        observation.descriptor.instrument,
        MetricInstrument::Sum {
            monotonic: true,
            temporality: AggregationTemporality::Cumulative
        }
    )));
    assert_eq!(completed[0].descriptor.unit.ucum, "{attempt}");
    assert_ne!(
        completed[0].start_time_unix_nano,
        completed[1].start_time_unix_nano
    );
}

#[test]
fn pid_pod_and_attempt_reuse_require_lifetime_identity() {
    let old = EntityIdentity::Process {
        environment_id: "laptop-a".into(),
        host_id: "macbook".into(),
        pid: 42,
        start_time_unix_nano: 10,
    };
    let new = EntityIdentity::Process {
        environment_id: "laptop-a".into(),
        host_id: "macbook".into(),
        pid: 42,
        start_time_unix_nano: 20,
    };
    assert_ne!(old, new);
    assert_ne!(
        EntityIdentity::Kubernetes {
            cluster_uid: "cluster".into(),
            resource_kind: "pod".into(),
            resource_uid: "uid-old".into(),
        },
        EntityIdentity::Kubernetes {
            cluster_uid: "cluster".into(),
            resource_kind: "pod".into(),
            resource_uid: "uid-new".into(),
        }
    );
    assert_ne!(
        EntityIdentity::TaskAttempt {
            application_id: "app".into(),
            task_id: "task".into(),
            invocation_id: "invocation".into(),
            attempt: 0,
        },
        EntityIdentity::TaskAttempt {
            application_id: "app".into(),
            task_id: "task".into(),
            invocation_id: "invocation".into(),
            attempt: 1,
        }
    );
}

#[test]
fn placement_and_workload_views_share_entities_without_double_counting() {
    let relations = vec![
        relation("org", "cluster", RelationKind::Contains),
        relation("org", "laptop", RelationKind::Contains),
        relation("cluster", "node", RelationKind::Contains),
        relation("pod", "node", RelationKind::ScheduledOn),
        relation("process", "pod", RelationKind::ScheduledOn),
        relation("cluster", "namespace", RelationKind::Contains),
        relation("namespace", "workload", RelationKind::Contains),
        relation("workload", "pod", RelationKind::Contains),
        relation("service", "pod", RelationKind::SelectedBy),
        relation("pod", "service", RelationKind::DependsOn),
    ];
    let placement = AdditiveProjection::build(
        &ProjectionSpec {
            name: "placement".into(),
            root: key("org"),
            edges: vec![
                ProjectionEdge {
                    kind: RelationKind::Contains,
                    direction: EdgeDirection::SubjectToObject,
                },
                ProjectionEdge {
                    kind: RelationKind::ScheduledOn,
                    direction: EdgeDirection::ObjectToSubject,
                },
            ],
            residual_policy: ResidualPolicy::ExplicitOnly,
        },
        &relations[..5],
        AT + 10,
    )
    .unwrap();
    let workload = AdditiveProjection::build(
        &ProjectionSpec {
            name: "workload".into(),
            root: key("org"),
            edges: vec![ProjectionEdge {
                kind: RelationKind::Contains,
                direction: EdgeDirection::SubjectToObject,
            }],
            residual_policy: ResidualPolicy::ExplicitOnly,
        },
        &[
            relations[0].clone(),
            relations[1].clone(),
            relations[5].clone(),
            relations[6].clone(),
            relations[7].clone(),
        ],
        AT + 10,
    )
    .unwrap();
    let samples = [
        observation("process", 3),
        observation("laptop", 5),
        observation("pod", 3),
    ];
    assert_eq!(
        placement
            .sum(&samples[..2], &descriptor(), AT + 10)
            .unwrap(),
        Some(Number::U64(8))
    );
    assert_eq!(
        workload.sum(&samples[1..], &descriptor(), AT + 10).unwrap(),
        Some(Number::U64(8))
    );
    assert!(!RelationKind::SelectedBy.semantics().additive_candidate);
}

#[test]
fn projection_rejects_cycles_fan_in_unsafe_edges_and_mixed_denominators() {
    let spec = ProjectionSpec {
        name: "invalid".into(),
        root: key("a"),
        edges: vec![ProjectionEdge {
            kind: RelationKind::Contains,
            direction: EdgeDirection::SubjectToObject,
        }],
        residual_policy: ResidualPolicy::RejectImplicit,
    };
    assert_eq!(
        AdditiveProjection::build(
            &spec,
            &[
                relation("a", "b", RelationKind::Contains),
                relation("b", "a", RelationKind::Contains)
            ],
            AT + 10
        ),
        Err(ProjectionError::Cycle)
    );
    assert_eq!(
        AdditiveProjection::build(
            &spec,
            &[
                relation("a", "c", RelationKind::Contains),
                relation("a", "b", RelationKind::Contains),
                relation("b", "c", RelationKind::Contains)
            ],
            AT + 10
        ),
        Err(ProjectionError::AdditiveFanIn)
    );
    let unsafe_spec = ProjectionSpec {
        name: "selection".into(),
        root: key("a"),
        edges: vec![ProjectionEdge {
            kind: RelationKind::SelectedBy,
            direction: EdgeDirection::SubjectToObject,
        }],
        residual_policy: ResidualPolicy::ExplicitOnly,
    };
    assert_eq!(
        AdditiveProjection::build(&unsafe_spec, &[], AT + 10),
        Err(ProjectionError::UnsafeRelationKind)
    );

    let projection = AdditiveProjection::build(
        &spec,
        &[
            relation("a", "b", RelationKind::Contains),
            relation("a", "c", RelationKind::Contains),
        ],
        AT + 10,
    )
    .unwrap();
    let mut first = observation("b", 1);
    let mut second = observation("c", 1);
    let system_descriptor = MetricDescriptor {
        value_domain: ValueDomain::Ratio {
            basis: RatioBasis::SystemCapacity,
        },
        ..descriptor()
    };
    first.descriptor = system_descriptor.clone();
    second.descriptor = system_descriptor.clone();
    first.ratio_denominator = Some(key("host-a"));
    second.ratio_denominator = Some(key("host-b"));
    assert_eq!(
        projection.sum(&[first, second], &system_descriptor, AT + 10),
        Err(ProjectionError::IncompatibleObservation)
    );

    let mut missing_denominator = observation("b", 1);
    missing_denominator.descriptor.value_domain = ValueDomain::Ratio {
        basis: RatioBasis::SingleLogicalCore,
    };
    assert_eq!(
        projection.sum(
            &[missing_denominator, observation("c", 1)],
            &descriptor(),
            AT + 10
        ),
        Ok(Some(Number::U64(2)))
    );
}

#[test]
fn projection_ignores_unreachable_cycles_fan_in_and_unresolved_candidates() {
    let spec = ProjectionSpec {
        name: "bounded-root".into(),
        root: key("root"),
        edges: vec![ProjectionEdge {
            kind: RelationKind::Contains,
            direction: EdgeDirection::SubjectToObject,
        }],
        residual_policy: ResidualPolicy::ExplicitOnly,
    };
    let mut unresolved = relation("elsewhere", "unknown", RelationKind::Contains);
    unresolved.provenance.join_status = JoinStatus::Ambiguous;
    let projection = AdditiveProjection::build(
        &spec,
        &[
            relation("root", "leaf", RelationKind::Contains),
            relation("cycle-a", "cycle-b", RelationKind::Contains),
            relation("cycle-b", "cycle-a", RelationKind::Contains),
            relation("parent-a", "shared", RelationKind::Contains),
            relation("parent-b", "shared", RelationKind::Contains),
            unresolved,
        ],
        AT + 10,
    )
    .unwrap();
    assert_eq!(
        projection
            .sum(&[observation("leaf", 7)], &descriptor(), AT + 10)
            .unwrap(),
        Some(Number::U64(7))
    );
}

#[test]
fn arithmetic_is_order_independent_and_missing_is_not_zero() {
    let projection = AdditiveProjection::build(
        &ProjectionSpec {
            name: "sum".into(),
            root: key("root"),
            edges: vec![ProjectionEdge {
                kind: RelationKind::Contains,
                direction: EdgeDirection::SubjectToObject,
            }],
            residual_policy: ResidualPolicy::ExplicitOnly,
        },
        &[
            relation("root", "a", RelationKind::Contains),
            relation("root", "b", RelationKind::Contains),
        ],
        AT + 10,
    )
    .unwrap();
    for (left, right) in [(0, 0), (1, 2), (u32::MAX as u64, 7)] {
        let forward = [observation("a", left), observation("b", right)];
        let reverse = [observation("b", right), observation("a", left)];
        assert_eq!(
            projection.sum(&forward, &descriptor(), AT + 10),
            projection.sum(&reverse, &descriptor(), AT + 10)
        );
    }
    let mut missing = observation("b", 0);
    missing.value = MetricObservation::Missing {
        reason: ih_muse_proto::MissingReason::SourceUnavailable,
    };
    assert_eq!(
        projection
            .sum(&[observation("a", 1), missing], &descriptor(), AT + 10)
            .unwrap(),
        None
    );
    assert_eq!(
        projection
            .sum(&[observation("a", 1)], &descriptor(), AT + 10)
            .unwrap(),
        None
    );
    assert_eq!(
        projection.sum(
            &[
                observation("a", 1),
                observation("a", 2),
                observation("b", 3)
            ],
            &descriptor(),
            AT + 10
        ),
        Err(ProjectionError::DuplicateObservation)
    );
}

#[test]
fn causal_and_late_relations_remain_non_additive_temporal_facts() {
    for kind in [
        RelationKind::ChildInvocationOf,
        RelationKind::TriggeredBy,
        RelationKind::TraceParent,
        RelationKind::TraceLink,
    ] {
        assert!(!kind.semantics().additive_candidate);
    }
    let late = ih_muse_proto::TemporalRelation {
        subject: EntityKey {
            organization: OrganizationId("org-a".into()),
            identity: EntityIdentity::TaskAttempt {
                application_id: "shibuya".into(),
                task_id: "request".into(),
                invocation_id: "child".into(),
                attempt: 1,
            },
        },
        object: EntityKey {
            organization: OrganizationId("org-a".into()),
            identity: EntityIdentity::Invocation {
                application_id: "shibuya".into(),
                invocation_id: "parent".into(),
            },
        },
        kind: RelationKind::TriggeredBy,
        valid_time: TimeRange {
            from_unix_nano: AT,
            to_unix_nano: AT + 10,
        },
        provenance: Provenance {
            observed_at_unix_nano: AT + 1_000,
            ..provenance(JoinStatus::Resolved)
        },
        attributes: BTreeMap::new(),
    };
    assert!(late.provenance.observed_at_unix_nano > late.valid_time.to_unix_nano);
}

#[test]
fn readable_view_oracle_covers_organization_laptop_placement_workload_and_causality() {
    let expected: ExpectedViews =
        serde_json::from_str(&fixture("graph-v1/views.expected.json")).unwrap();
    assert_eq!(expected.organization, "org-a");
    assert!(expected
        .entities
        .iter()
        .any(|item| item.starts_with("request:")));
    assert!(expected
        .entities
        .iter()
        .any(|item| item.starts_with("worker:")));
    assert!(expected
        .entities
        .iter()
        .any(|item| item.starts_with("runner:")));
    assert_eq!(
        expected.placement_view.last(),
        expected.workload_view.last()
    );
    assert!(expected.independent_environment_view[1].starts_with("standalone_environment:"));
    assert!(expected
        .causal_relations
        .iter()
        .any(|item| item.contains("triggered_by")));
    assert!(expected.accounting.contains("never add"));
}

#[test]
fn ambiguous_join_is_retained_but_cannot_enter_an_additive_view() {
    let telemetry: TelemetryEnvelope =
        serde_json::from_str(&fixture("telemetry-v3/task-retry.json")).unwrap();
    let mut graph =
        GraphBatch::from_telemetry_v3(&telemetry, OrganizationId("org-a".into()), "fixture")
            .unwrap();
    let relation = graph
        .relations
        .iter_mut()
        .find(|relation| relation.kind == RelationKind::ScheduledOn)
        .unwrap();
    relation.provenance.join_status = JoinStatus::Ambiguous;
    let root = relation.object.clone();
    graph.validate().unwrap();

    assert_eq!(
        AdditiveProjection::build(
            &ProjectionSpec {
                name: "ambiguous-placement".into(),
                root,
                edges: vec![ProjectionEdge {
                    kind: RelationKind::ScheduledOn,
                    direction: EdgeDirection::ObjectToSubject,
                }],
                residual_policy: ResidualPolicy::ExplicitOnly,
            },
            &graph.relations,
            relation_time(&graph.relations),
        ),
        Err(ProjectionError::UnresolvedRelation)
    );

    graph.relations[0].subject.organization = OrganizationId("other-org".into());
    assert_eq!(
        graph.validate(),
        Err(GraphValidationError::CrossOrganization)
    );
}

fn relation_time(relations: &[ih_muse_proto::TemporalRelation]) -> u64 {
    relations
        .iter()
        .find(|relation| relation.kind == RelationKind::ScheduledOn)
        .unwrap()
        .valid_time
        .from_unix_nano
}

#[test]
fn projection_rejects_float_overflow_and_mismatched_intervals() {
    let projection = AdditiveProjection {
        root: key("root"),
        children: BTreeMap::from([(key("root"), [key("a"), key("b")].into_iter().collect())]),
    };
    let mut items = vec![observation("a", 1), observation("b", 2)];
    items[1].start_time_unix_nano = Some(AT - 20);
    assert_eq!(
        projection.sum(&items, &descriptor(), AT + 10),
        Err(ProjectionError::IncompatibleObservation)
    );
    items[1].start_time_unix_nano = items[0].start_time_unix_nano;
    for item in &mut items {
        item.value = MetricObservation::Measured {
            value: TypedMetricValue::Sum(Number::F64(f64::MAX)),
        };
    }
    assert_eq!(
        projection.sum(&items, &descriptor(), AT + 10),
        Err(ProjectionError::Overflow)
    );
}

#[test]
fn graph_rejects_non_finite_metrics_before_json_can_replace_them_with_null() {
    let telemetry: TelemetryEnvelope =
        serde_json::from_str(&fixture("telemetry-v3/task-retry.json")).unwrap();
    let mut graph =
        GraphBatch::from_telemetry_v3(&telemetry, OrganizationId("org-a".into()), "fixture")
            .unwrap();
    let mut item = observation("a", 1);
    item.entity = graph.entities[0].key.clone();
    item.value = MetricObservation::Measured {
        value: TypedMetricValue::Gauge(Number::F64(f64::NAN)),
    };
    graph.observations.push(item);
    assert_eq!(graph.validate(), Err(GraphValidationError::InvalidMetric));
}

#[test]
fn projection_build_rejects_cross_organization_without_batch_validation() {
    let mut edge = relation("root", "child", RelationKind::Contains);
    edge.object.organization = OrganizationId("different-organization".into());
    let spec = ProjectionSpec {
        name: "placement".into(),
        root: key("root"),
        edges: vec![ProjectionEdge {
            kind: RelationKind::Contains,
            direction: EdgeDirection::SubjectToObject,
        }],
        residual_policy: ResidualPolicy::ExplicitOnly,
    };
    assert_eq!(
        AdditiveProjection::build(&spec, &[edge], AT + 1),
        Err(ProjectionError::IncompatibleObservation)
    );
}
