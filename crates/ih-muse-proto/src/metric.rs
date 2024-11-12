// crates/ih-muse-proto/src/metric.rs

use serde::{Deserialize, Serialize};

use crate::types::*;
use crate::utils::deterministic_u32_from_str;

pub fn metric_id_from_code(code: &str) -> u32 {
    deterministic_u32_from_str(code)
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MetricDefinition {
    pub id: MetricId,
    pub code: String,
    pub name: String,
    pub description: String,
}

impl MetricDefinition {
    pub fn new(code: String, name: String, description: String) -> Self {
        Self {
            id: metric_id_from_code(&code),
            code,
            name,
            description,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MetricPayload {
    pub time: Timestamp,
    pub element_id: ElementId,
    pub metric_ids: Vec<MetricId>,
    pub values: Vec<Option<MetricValue>>,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct MetricQuery {
    pub start_time: Option<i64>,
    pub end_time: Option<i64>,
    pub element_id: Option<u64>,
    pub parent_id: Option<u64>,
    pub metric_id: Option<u32>,
}
