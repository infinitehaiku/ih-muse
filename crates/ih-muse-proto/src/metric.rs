// crates/ih-muse-proto/src/metric.rs

use serde::{Deserialize, Serialize};

use crate::types::*;
use crate::utils::deterministic_u32_from_str;

pub fn metric_id_from_code(code: &str) -> u32 {
    deterministic_u32_from_str(code)
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct MetricDefinition {
    pub id: MetricId,
    pub code: String,
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<MetricDisplay>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct MetricDisplay {
    pub unit: String,
    pub kind: String,
    pub aggregation: String,
    pub direction: String,
    pub precision: u8,
    #[serde(default)]
    pub element_kinds: Vec<String>,
}

impl MetricDisplay {
    pub fn infer(code: &str) -> Self {
        let unit = if code.ends_with("percent_per_hour") {
            "percent_per_hour"
        } else if code.ends_with("percent") {
            "percent"
        } else if code.contains("bytes") {
            "bytes"
        } else if code.ends_with("celsius") {
            "celsius"
        } else if code.ends_with("charging") || code.ends_with("on_battery") {
            "boolean"
        } else {
            "number"
        };
        let kind = if code.ends_with("_delta") {
            "delta"
        } else if code.contains(".total_") {
            "counter"
        } else {
            "gauge"
        };
        Self {
            unit: unit.into(),
            kind: kind.into(),
            aggregation: if code.starts_with("process.") || kind == "delta" {
                "sum"
            } else {
                "none"
            }
            .into(),
            direction: "neutral".into(),
            precision: 1,
            element_kinds: Vec::new(),
        }
    }
}

impl MetricDefinition {
    pub fn new(code: &str, name: &str, description: &str) -> Self {
        Self {
            id: metric_id_from_code(code),
            code: code.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            display: Some(MetricDisplay::infer(code)),
        }
    }
}

#[cfg(test)]
mod display_tests {
    use super::*;
    #[test]
    fn old_definitions_and_explicit_metadata_round_trip() {
        let old = r#"{"id":1,"code":"memory_bytes","name":"Memory","description":""}"#;
        let mut definition: MetricDefinition = serde_json::from_str(old).unwrap();
        assert!(definition.display.is_none());
        definition.display = Some(MetricDisplay::infer(&definition.code));
        let restored: MetricDefinition =
            serde_json::from_str(&serde_json::to_string(&definition).unwrap()).unwrap();
        assert_eq!(restored, definition);
        assert_eq!(restored.display.unwrap().unit, "bytes");
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct MetricPayload {
    pub time: Timestamp,
    pub element_id: ElementId,
    pub metric_ids: Vec<MetricId>,
    pub values: Vec<Option<MetricValue>>,
}

impl MetricPayload {
    pub fn new(
        time: Timestamp,
        element_id: ElementId,
        metric_ids: Vec<MetricId>,
        values: Vec<Option<MetricValue>>,
    ) -> Self {
        Self {
            time,
            element_id,
            metric_ids,
            values,
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Default, Clone)]
pub struct MetricQuery {
    pub start_time: Option<i64>,
    pub end_time: Option<i64>,
    pub element_id: Option<u64>,
    pub parent_id: Option<u64>,
    pub metric_id: Option<u32>,
}

impl MetricQuery {
    pub fn new(
        start_time: Option<i64>,
        end_time: Option<i64>,
        element_id: Option<u64>,
        parent_id: Option<u64>,
        metric_id: Option<u32>,
    ) -> Self {
        Self {
            start_time,
            end_time,
            element_id,
            parent_id,
            metric_id,
        }
    }
}
