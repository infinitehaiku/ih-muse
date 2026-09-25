//! Semantic acceptance for the separately versioned Rustvello SDK mapping fixture.
use serde_json::Value;

fn records() -> Vec<Value> {
    serde_json::from_str(include_str!("fixtures/rustvello-otel-v1/sdk.expected.json")).unwrap()
}

#[test]
fn retries_keep_trace_parent_but_change_attempt_span_and_worker() {
    let records = records();
    for invocation in ["rust-invocation", "python-invocation"] {
        let mut spans: Vec<_> = records
            .iter()
            .filter(|r| r["signal"] == "traces" && r["invocation_id"] == invocation)
            .collect();
        spans.sort_by_key(|r| r["attempt"].as_u64().unwrap());
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0]["attempt"], 0);
        assert_eq!(spans[1]["attempt"], 1);
        assert_ne!(spans[0]["resource_id"], spans[1]["resource_id"]);
        assert_eq!(spans[0]["trace_id"], spans[1]["trace_id"]);
        assert_ne!(spans[0]["span_id"], spans[1]["span_id"]);
        for r in &spans {
            assert_eq!(r["data"]["record"]["parentSpanId"], "0000000000000001");
        }
        assert_eq!(
            spans[1]["data"]["record"]["links"][0]["spanId"],
            spans[0]["span_id"]
        );
        assert_eq!(spans[0]["data"]["record"]["status"]["code"], 2);
        assert_eq!(spans[1]["data"]["record"]["status"]["code"], 1);
    }
}

#[test]
fn child_uses_active_execution_parent_and_unsampled_log_survives() {
    let rows = records();
    let child = rows
        .iter()
        .find(|r| r["invocation_id"] == "python-child")
        .unwrap();
    let parent = rows
        .iter()
        .find(|r| {
            r["signal"] == "traces"
                && r["invocation_id"] == "python-invocation"
                && r["attempt"] == 1
        })
        .unwrap();
    assert_eq!(child["data"]["record"]["parentSpanId"], parent["span_id"]);
    let unsampled: Vec<_> = rows
        .iter()
        .filter(|r| r["invocation_id"] == "unsampled-invocation")
        .collect();
    assert_eq!(unsampled.len(), 1);
    assert_eq!(unsampled[0]["signal"], "logs");
    assert_eq!(unsampled[0]["data"]["record"]["flags"], 0);
    assert!(!unsampled[0]["trace_id"].as_str().unwrap().is_empty());
}

#[test]
fn worker_logs_and_metric_dimensions_do_not_invent_attempts() {
    let mapping: Value =
        serde_json::from_str(include_str!("fixtures/rustvello-otel-v1/mapping.json")).unwrap();
    assert_eq!(mapping["revision"], "ih.rustvello-otel.v1");
    assert_eq!(mapping["base_contract"], "ih.telemetry.v3");
    assert_eq!(
        mapping["metrics"]["rustvello.task.duration"]["receiver_supported"],
        false
    );
    for r in records() {
        if r["data"]["record"]["body"]["stringValue"]
            .as_str()
            .is_some_and(|s| s.starts_with("worker."))
        {
            assert!(r["attempt"].is_null());
            assert!(r["invocation_id"].is_null());
        }
        if r["signal"] == "metrics" {
            assert!(r["attempt"].is_null());
            let point = &r["data"]["record"]["metric"]["sum"]["dataPoints"][0];
            assert_eq!(point["asInt"], 0);
            assert_eq!(r["data"]["record"]["missing"], false);
        }
    }
}
