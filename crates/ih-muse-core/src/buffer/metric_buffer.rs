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
    ///
    /// Callers that fail to deliver the snapshot must hand it back with
    /// [`MetricBuffer::restore`], or those values are lost.
    pub async fn get_and_clear(&self) -> HashMap<LocalElementId, HashMap<String, MetricValue>> {
        let mut buffer = self.buffer.lock().await;
        std::mem::take(&mut *buffer)
    }

    /// Puts back values that were taken but not delivered.
    ///
    /// A value buffered since the snapshot was taken is newer and wins. Restored
    /// values still respect the capacity; the number that did not fit is
    /// returned so the caller can report the loss instead of hiding it.
    pub async fn restore(
        &self,
        unsent: HashMap<LocalElementId, HashMap<String, MetricValue>>,
    ) -> usize {
        let mut buffer = self.buffer.lock().await;
        let mut pending_values = buffer.values().map(HashMap::len).sum::<usize>();
        let mut dropped = 0;
        for (element, metrics) in unsent {
            let entry = buffer.entry(element).or_default();
            for (code, value) in metrics {
                if entry.contains_key(&code) {
                    continue;
                }
                if pending_values >= self.max_pending_values {
                    dropped += 1;
                    continue;
                }
                entry.insert(code, value);
                pending_values += 1;
            }
        }
        buffer.retain(|_, metrics| !metrics.is_empty());
        dropped
    }

    /// Number of buffered values.
    pub async fn len(&self) -> usize {
        self.buffer.lock().await.values().map(HashMap::len).sum()
    }

    pub async fn is_empty(&self) -> bool {
        self.len().await == 0
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

    #[tokio::test]
    async fn restore_keeps_newer_values_and_reports_what_did_not_fit() {
        let buffer = MetricBuffer::with_capacity(2);
        let element_id = Uuid::new_v4();
        buffer.add_metric(element_id, "cpu".into(), 1.0).await.unwrap();
        buffer.add_metric(element_id, "memory".into(), 2.0).await.unwrap();
        let unsent = buffer.get_and_clear().await;
        assert!(buffer.is_empty().await);
        // A newer cpu sample arrives while the send is failing.
        buffer.add_metric(element_id, "cpu".into(), 5.0).await.unwrap();
        assert_eq!(buffer.restore(unsent).await, 0);
        let values = buffer.get_and_clear().await;
        assert_eq!(values[&element_id]["cpu"], 5.0);
        assert_eq!(values[&element_id]["memory"], 2.0);

        buffer.add_metric(element_id, "a".into(), 1.0).await.unwrap();
        buffer.add_metric(element_id, "b".into(), 1.0).await.unwrap();
        let mut overflow = HashMap::new();
        overflow.insert(element_id, HashMap::from([("c".to_string(), 1.0)]));
        assert_eq!(buffer.restore(overflow).await, 1);
    }
}
