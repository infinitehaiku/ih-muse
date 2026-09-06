// crates/ih-muse-core/src/buffer/metric_buffer.rs

use std::collections::HashMap;

use tokio::sync::Mutex;

use ih_muse_proto::{LocalElementId, MetricValue};

use crate::{MuseError, MuseResult};

pub const DEFAULT_MAX_PENDING_METRIC_VALUES: usize = 16_384;

pub struct MetricBuffer {
    buffer: Mutex<HashMap<LocalElementId, HashMap<String, MetricValue>>>,
    max_pending_values: usize,
}

impl Default for MetricBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricBuffer {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_MAX_PENDING_METRIC_VALUES)
    }

    pub fn with_capacity(max_pending_values: usize) -> Self {
        Self {
            buffer: Mutex::new(HashMap::new()),
            max_pending_values,
        }
    }

    /// Adds a metric to the buffer.
    pub async fn add_metric(
        &self,
        local_elem_id: LocalElementId,
        metric_code: String,
        value: MetricValue,
    ) -> MuseResult<()> {
        let mut buffer = self.buffer.lock().await;
        let is_new_value = match buffer.get(&local_elem_id) {
            Some(metrics) => !metrics.contains_key(&metric_code),
            None => true,
        };
        let pending_values = buffer.values().map(HashMap::len).sum::<usize>();
        if is_new_value && pending_values >= self.max_pending_values {
            return Err(MuseError::Backpressure(format!(
                "metric buffer reached its {} value limit",
                self.max_pending_values
            )));
        }
        buffer
            .entry(local_elem_id)
            .or_insert_with(HashMap::new)
            .insert(metric_code, value);
        Ok(())
    }

    /// Retrieves and clears all buffered metrics.
    pub async fn get_and_clear(&self) -> HashMap<LocalElementId, HashMap<String, MetricValue>> {
        let mut buffer = self.buffer.lock().await;
        let data = buffer.clone();
        buffer.clear();
        data
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[tokio::test]
    async fn buffer_applies_backpressure_without_rejecting_replacements() {
        let buffer = MetricBuffer::with_capacity(1);
        let element_id = Uuid::new_v4();
        buffer
            .add_metric(element_id, "cpu".to_string(), 1.0)
            .await
            .unwrap();
        buffer
            .add_metric(element_id, "cpu".to_string(), 2.0)
            .await
            .unwrap();
        assert!(matches!(
            buffer
                .add_metric(element_id, "memory".to_string(), 3.0)
                .await,
            Err(MuseError::Backpressure(_))
        ));
        let values = buffer.get_and_clear().await;
        assert_eq!(values[&element_id]["cpu"], 2.0);
    }
}
