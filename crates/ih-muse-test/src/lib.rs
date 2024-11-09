use ih_muse_core::{ElementId, MetricId, MetricValue};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MetricRecord {
    pub element_id: ElementId,
    pub metric_id: MetricId,
    pub value: MetricValue,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

pub struct MetricRecorder {
    records: Vec<MetricRecord>,
}

impl Default for MetricRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricRecorder {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
        }
    }

    pub fn record(&mut self, element: ElementId, metric: MetricId, value: MetricValue) {
        self.records.push(MetricRecord {
            element_id: element,
            metric_id: metric,
            value,
            timestamp: chrono::Utc::now(),
        });
    }

    pub fn replay(&self) -> impl Iterator<Item = &MetricRecord> {
        self.records.iter()
    }
}
