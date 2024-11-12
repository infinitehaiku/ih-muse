use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use once_cell::sync::Lazy;
use tokio::sync::Mutex;

use ih_muse_core::{Error, Transport};
use ih_muse_proto::*;

static NEXT_ELEMENT_ID: Lazy<AtomicU64> = Lazy::new(|| AtomicU64::new(0));

/// Get a new unique ElementId as a `u64`
pub fn get_new_element_id() -> u64 {
    NEXT_ELEMENT_ID.fetch_add(1, Ordering::SeqCst)
}

pub struct MockClient {
    metrics: Arc<Mutex<Vec<MetricDefinition>>>,
    sent_metrics: Arc<Mutex<Vec<MetricPayload>>>,
    finest_resolution: Arc<Mutex<TimestampResolution>>,
}

impl Default for MockClient {
    fn default() -> Self {
        Self::new()
    }
}

impl MockClient {
    pub fn new() -> Self {
        MockClient {
            metrics: Arc::new(Mutex::new(Vec::new())),
            sent_metrics: Arc::new(Mutex::new(Vec::new())),
            finest_resolution: Arc::new(Mutex::new(TimestampResolution::default())),
        }
    }

    /// Sets a new finest resolution value
    pub async fn set_finest_resolution(&self, resolution: TimestampResolution) {
        let mut finest_res = self.finest_resolution.lock().await;
        *finest_res = resolution;
    }
}

#[async_trait]
impl Transport for MockClient {
    async fn health_check(&self) -> Result<(), Error> {
        log::info!("MockClient: health_check called");
        Ok(())
    }

    async fn get_node_state(&self) -> Result<NodeState, Error> {
        log::info!("MockClient: get_node_state called");
        Err(Error::ClientError(
            "NodeState not implemented for mock client yet".to_string(),
        ))
    }

    async fn get_finest_resolution(&self) -> Result<TimestampResolution, Error> {
        log::info!("MockClient: get_finest_resolution called");
        Ok(*self.finest_resolution.lock().await)
    }

    async fn get_node_elem_ranges(
        &self,
        ini: Option<u64>,
        end: Option<u64>,
    ) -> Result<Vec<NodeElementRange>, Error> {
        log::info!(
            "MockClient: get_node_elem_ranges called with {:?}..{:?}",
            ini,
            end
        );
        // TODO made up range(s) based in current register elements
        Err(Error::ClientError(
            "get_node_elem_ranges not implemented".to_string(),
        ))
    }

    async fn register_element_kinds(
        &self,
        element_kinds: &[ElementKindRegistration],
    ) -> Result<(), Error> {
        log::info!(
            "MockClient: register_element_kinds called with {:?}",
            element_kinds
        );
        Ok(())
    }

    async fn register_elements(
        &self,
        elements: &[ElementRegistration],
    ) -> Result<Vec<Result<ElementId, Error>>, Error> {
        log::info!("MockClient: register_elements called with {:?}", elements);
        let results = elements.iter().map(|_| Ok(get_new_element_id())).collect();
        Ok(results)
    }

    async fn register_metrics(&self, payload: &[MetricDefinition]) -> Result<(), Error> {
        log::info!("MockClient: register_metrics called with {:?}", payload);
        let mut metrics = self.metrics.lock().await;
        metrics.extend(payload.iter().cloned());
        Ok(())
    }

    async fn get_metric_order(&self) -> Result<Vec<MetricDefinition>, Error> {
        log::info!("MockClient: get_metric_order called");
        let metrics = self.metrics.lock().await;
        Ok(metrics.clone())
    }

    async fn get_metrics(&self, query: &MetricQuery) -> Result<Vec<MetricPayload>, Error> {
        log::info!("MockClient: get_metrics called with query: {:?}", query);
        if query.parent_id.is_some() {
            return Err(Error::ClientError(
                "parent_id not implemented in MockClient".to_string(),
            ));
        }
        let mut results = Vec::new();
        for payload in self.sent_metrics.lock().await.iter() {
            // Filter by time range
            if let Some(start_time) = query.start_time {
                if payload.time < start_time {
                    continue;
                }
            }
            if let Some(end_time) = query.end_time {
                if payload.time > end_time {
                    continue;
                }
            }
            // Filter by element_id
            if let Some(query_element_id) = query.element_id {
                if payload.element_id != query_element_id {
                    continue;
                }
            }
            // Filter by metric_id if specified
            if let Some(query_metric_id) = query.metric_id {
                // Check if any metric_id in the payload matches `query_metric_id`
                if !payload.metric_ids.contains(&query_metric_id) {
                    continue;
                }
            }
            results.push(payload.clone());
        }
        Ok(results)
    }

    async fn send_metrics(&self, payload: Vec<MetricPayload>) -> Result<(), Error> {
        log::info!("MockClient: send_metrics called with {:?}", payload);
        let mut sent_metrics = self.sent_metrics.lock().await;
        sent_metrics.extend(payload);
        Ok(())
    }
}
