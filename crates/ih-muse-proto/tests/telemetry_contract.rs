use std::fs;
use std::path::PathBuf;

use ih_muse_proto::{
    AttributeValue, MetricObservation, MissingReason, Number, PersistenceProjection, SignalKind,
    TelemetryEnvelope, TelemetryQuery, TelemetryQueryValidationError, TelemetryValidationError,
    TimeRange, TypedMetricValue as MetricValue, MAX_TELEMETRY_QUERY_RECORDS,
    TELEMETRY_CONTRACT_REVISION, TELEMETRY_SCHEMA_VERSION,
};
use serde::Deserialize;

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/telemetry-v3")
        .join(name);
    fs::read_to_string(path).expect("telemetry contract fixture must be readable")
}

/// Counts and selected records expected from the canonical persistence projection.
#[derive(Debug, Deserialize)]
struct ExpectedProjection {
    schema_version: u16,
    resource_count: usize,
    relationship_count: usize,
    metric_count: usize,
    span_count: usize,
    log_count: usize,
    metric_results: Vec<ExpectedMetric>,
    span_ids: Vec<String>,
    log_event_ids: Vec<String>,
}

/// Expected typed outcome for one golden metric record.
#[derive(Debug, Deserialize)]
struct ExpectedMetric {
    name: String,
    time_unix_nano: u64,
    status: String,
    number_type: Option<String>,
    number: Option<serde_json::Value>,
    histogram_count: Option<u64>,
    missing_reason: Option<String>,
}

/// Machine-readable statement of the currently normalized signal profile.
#[derive(Debug, Deserialize)]
struct CompatibilityProfile {
    schema_version: u16,
    contract_revision: String,
    normalized_signals: Vec<String>,
    unsupported_otlp_metric_kinds: Vec<UnsupportedKind>,
    tenant_identity: String,
    native_delivery: String,
    otlp_log_delivery_without_event_id: String,
}

/// Explicit rejection behavior for one source protocol signal kind.
#[derive(Debug, Deserialize)]
struct UnsupportedKind {
    kind: String,
    behavior: String,
    reason: String,
}

#[test]
fn golden_task_retry_has_expected_normalized_persistence_projection() {
    let envelope: TelemetryEnvelope = serde_json::from_str(&fixture("task-retry.json")).unwrap();
    let expected: ExpectedProjection =
        serde_json::from_str(&fixture("task-retry.expected.json")).unwrap();
    let projection = envelope.persistence_projection().unwrap();

    assert_projection_counts(&projection, &expected);
    for metric in &expected.metric_results {
        let record = projection
            .metrics
            .iter()
            .find(|record| {
                record.series.descriptor.name == metric.name
                    && record.time_unix_nano == metric.time_unix_nano
            })
            .unwrap_or_else(|| panic!("missing expected metric {}", metric.name));
        assert_eq!(record.time_unix_nano, metric.time_unix_nano);
        assert_expected_observation(&record.observation, metric);
    }

    let mut span_ids = projection
        .spans
        .iter()
        .map(|span| span.span_id.clone())
        .collect::<Vec<_>>();
    span_ids.sort();
    assert_eq!(span_ids, expected.span_ids);
    let mut event_ids = projection
        .logs
        .iter()
        .filter_map(|log| log.event_id.clone())
        .collect::<Vec<_>>();
    event_ids.sort();
    assert_eq!(event_ids, expected.log_event_ids);
}

#[test]
fn golden_task_retry_full_projection_is_locked() {
    let envelope: TelemetryEnvelope = serde_json::from_str(&fixture("task-retry.json")).unwrap();
    let expected: PersistenceProjection =
        serde_json::from_str(&fixture("task-retry.projected.json")).unwrap();
    assert_eq!(envelope.persistence_projection().unwrap(), expected);
}

#[test]
fn missing_and_measured_zero_remain_different_records() {
    let envelope: TelemetryEnvelope = serde_json::from_str(&fixture("task-retry.json")).unwrap();
    let projection = envelope.persistence_projection().unwrap();
    let zero = projection
        .metrics
        .iter()
        .find(|record| record.series.descriptor.name == "process.cpu.utilization")
        .unwrap();
    let missing = projection
        .metrics
        .iter()
        .find(|record| record.series.descriptor.name == "process.network.io")
        .unwrap();

    assert!(matches!(
        zero.observation,
        MetricObservation::Measured {
            value: MetricValue::Gauge(Number::F64(value))
        } if value == 0.0
    ));
    assert_eq!(
        missing.observation,
        MetricObservation::Missing {
            reason: MissingReason::Unsupported
        }
    );
}

#[test]
fn payload_tenant_identity_is_rejected() {
    let mut envelope: TelemetryEnvelope =
        serde_json::from_str(&fixture("task-retry.json")).unwrap();
    envelope.resources[0].attributes.insert(
        "tenant.id".into(),
        AttributeValue::String("untrusted".into()),
    );
    assert_eq!(
        envelope.validate(),
        Err(TelemetryValidationError::PayloadTenantIdentity)
    );
}

#[test]
fn compatibility_profile_makes_unsupported_summary_and_delivery_explicit() {
    let profile: CompatibilityProfile = serde_json::from_str(&fixture("profile.json")).unwrap();
    assert_eq!(profile.schema_version, TELEMETRY_SCHEMA_VERSION);
    assert_eq!(profile.contract_revision, TELEMETRY_CONTRACT_REVISION);
    assert!(profile.normalized_signals.contains(&"span".to_string()));
    assert!(profile.normalized_signals.contains(&"log".to_string()));
    assert_eq!(profile.tenant_identity, "authenticated_transport_only");
    assert_eq!(profile.native_delivery, "batch_id_deduplicated");
    assert_eq!(profile.otlp_log_delivery_without_event_id, "at_least_once");
    assert_eq!(profile.unsupported_otlp_metric_kinds.len(), 1);
    let summary = &profile.unsupported_otlp_metric_kinds[0];
    assert_eq!(summary.kind, "summary");
    assert_eq!(summary.behavior, "reject");
    assert!(summary.reason.contains("reaggregated"));
}

#[test]
fn descriptions_are_non_identifying_and_the_longest_is_canonical() {
    let mut envelope: TelemetryEnvelope =
        serde_json::from_str(&fixture("task-retry.json")).unwrap();
    let mut duplicate = envelope.metrics[0].clone();
    duplicate.descriptor.description = "A deliberately longer compatible description".into();
    duplicate.time_unix_nano += 1;
    envelope.metrics.push(duplicate);

    let projection = envelope.persistence_projection().unwrap();
    let descriptions = projection
        .metrics
        .iter()
        .filter(|metric| metric.series.descriptor.name == "process.cpu.utilization")
        .map(|metric| metric.series.descriptor.description.as_str())
        .collect::<Vec<_>>();
    assert_eq!(descriptions.len(), 2);
    assert!(descriptions
        .iter()
        .all(|description| *description == "A deliberately longer compatible description"));
}

#[test]
fn conflicting_semantics_for_one_series_are_rejected() {
    let mut envelope: TelemetryEnvelope =
        serde_json::from_str(&fixture("task-retry.json")).unwrap();
    let mut conflicting = envelope.metrics[0].clone();
    conflicting.descriptor.spatial_aggregation = ih_muse_proto::SpatialAggregation::Mean;
    conflicting.time_unix_nano += 1;
    envelope.metrics.push(conflicting);

    assert!(matches!(
        envelope.validate(),
        Err(TelemetryValidationError::InvalidMetric(message))
            if message.contains("conflicting metric descriptors")
    ));
}

#[test]
fn zero_based_attempts_and_resource_only_batches_are_valid() {
    let mut envelope: TelemetryEnvelope =
        serde_json::from_str(&fixture("task-retry.json")).unwrap();
    assert!(envelope.resources.iter().any(|resource| matches!(
        resource.key,
        ih_muse_proto::ResourceKey::TaskAttempt { attempt: 0, .. }
    )));
    assert_eq!(envelope.validate(), Ok(()));

    envelope.relationships.clear();
    envelope.metrics.clear();
    envelope.spans.clear();
    envelope.logs.clear();
    assert_eq!(envelope.validate(), Ok(()));
}

#[test]
fn otlp_optionality_and_recursive_log_bodies_are_preserved() {
    let mut envelope: TelemetryEnvelope =
        serde_json::from_str(&fixture("task-retry.json")).unwrap();
    let cumulative = envelope
        .metrics
        .iter_mut()
        .find(|metric| metric.descriptor.name == "task.attempt.completed")
        .unwrap();
    cumulative.start_time_unix_nano = None;
    cumulative.scope.name.clear();
    let log = envelope.logs.first_mut().unwrap();
    log.observed_time_unix_nano = 0;
    log.span_id = None;
    assert!(matches!(log.body, Some(AttributeValue::KeyValueList(_))));
    assert_eq!(envelope.validate(), Ok(()));
}

#[test]
fn cpu_ratio_basis_controls_valid_range_and_spatial_sum() {
    let mut envelope: TelemetryEnvelope =
        serde_json::from_str(&fixture("task-retry.json")).unwrap();
    let cpu = envelope
        .metrics
        .iter_mut()
        .find(|metric| metric.descriptor.name == "process.cpu.utilization")
        .unwrap();
    cpu.observation = MetricObservation::Measured {
        value: MetricValue::Gauge(Number::F64(3.5)),
    };
    assert_eq!(envelope.validate(), Ok(()));

    let cpu = envelope
        .metrics
        .iter_mut()
        .find(|metric| metric.descriptor.name == "process.cpu.utilization")
        .unwrap();
    cpu.descriptor.value_domain = ih_muse_proto::ValueDomain::Ratio {
        basis: ih_muse_proto::RatioBasis::SystemCapacity,
    };
    assert!(matches!(
        envelope.validate(),
        Err(TelemetryValidationError::InvalidMetric(_))
    ));
}

#[test]
fn typed_query_requires_bounded_pages_and_valid_time() {
    let query = TelemetryQuery {
        time_range: TimeRange {
            from_unix_nano: 100,
            to_unix_nano: 200,
        },
        signal_kinds: [SignalKind::Metric, SignalKind::Span].into_iter().collect(),
        resource: None,
        metric_name: None,
        trace_id: Some("00112233445566778899aabbccddeeff".into()),
        scope_name: Some("rustvello".into()),
        attribute_filters: Default::default(),
        minimum_log_severity: Some(9),
        limit: 500,
        cursor: None,
    };
    assert_eq!(query.validate(), Ok(()));

    let mut invalid = query;
    invalid.limit = MAX_TELEMETRY_QUERY_RECORDS + 1;
    assert_eq!(
        invalid.validate(),
        Err(TelemetryQueryValidationError::InvalidLimit)
    );
}

#[test]
fn late_reset_and_duplicate_projection_semantics_are_stable() {
    let envelope: TelemetryEnvelope = serde_json::from_str(&fixture("task-retry.json")).unwrap();
    let first = envelope.persistence_projection().unwrap();
    let retry = envelope.persistence_projection().unwrap();
    assert_eq!(
        first, retry,
        "the same native batch has one canonical projection"
    );

    let mut counter_records = first
        .metrics
        .iter()
        .filter(|record| record.series.descriptor.name == "task.attempt.completed")
        .collect::<Vec<_>>();
    counter_records.sort_by_key(|record| record.time_unix_nano);
    assert_eq!(counter_records.len(), 2);
    assert_eq!(
        counter_records[0].start_time_unix_nano,
        Some(1_788_875_900_000_000_000)
    );
    assert_eq!(
        counter_records[1].start_time_unix_nano,
        Some(1_788_876_004_200_000_000),
        "a changed cumulative start time preserves reset provenance"
    );
    assert!(first
        .metrics
        .iter()
        .all(|record| record.time_unix_nano < envelope.exported_at_unix_nano));
}

fn assert_projection_counts(projection: &PersistenceProjection, expected: &ExpectedProjection) {
    assert_eq!(projection.schema_version, expected.schema_version);
    assert_eq!(projection.resources.len(), expected.resource_count);
    assert_eq!(projection.relationships.len(), expected.relationship_count);
    assert_eq!(projection.metrics.len(), expected.metric_count);
    assert_eq!(projection.spans.len(), expected.span_count);
    assert_eq!(projection.logs.len(), expected.log_count);
}

fn assert_expected_observation(observation: &MetricObservation, expected: &ExpectedMetric) {
    assert_eq!(
        match observation {
            MetricObservation::Measured { .. } => "measured",
            MetricObservation::Missing { .. } => "missing",
        },
        expected.status
    );
    match observation {
        MetricObservation::Measured {
            value: MetricValue::Gauge(number) | MetricValue::Sum(number),
        } => {
            let number_type = match number {
                Number::I64(_) => "i64",
                Number::U64(_) => "u64",
                Number::F64(_) => "f64",
            };
            assert_eq!(expected.number_type.as_deref(), Some(number_type));
            let expected_number = expected.number.as_ref().expect("expected typed number");
            match number {
                Number::I64(value) => assert_eq!(expected_number.as_i64(), Some(*value)),
                Number::U64(value) => assert_eq!(expected_number.as_u64(), Some(*value)),
                Number::F64(value) => assert_eq!(expected_number.as_f64(), Some(*value)),
            }
        }
        MetricObservation::Measured {
            value: MetricValue::Histogram(histogram),
        } => assert_eq!(expected.histogram_count, Some(histogram.count)),
        MetricObservation::Missing { reason } => assert_eq!(
            expected.missing_reason.as_deref(),
            Some(match reason {
                MissingReason::NotCollected => "not_collected",
                MissingReason::Unsupported => "unsupported",
                MissingReason::PermissionDenied => "permission_denied",
                MissingReason::SourceUnavailable => "source_unavailable",
                MissingReason::NotYetAvailable => "not_yet_available",
                MissingReason::Unknown => "unknown",
            })
        ),
        MetricObservation::Measured {
            value: MetricValue::ExponentialHistogram(histogram),
        } => assert_eq!(expected.histogram_count, Some(histogram.count)),
    }
}
