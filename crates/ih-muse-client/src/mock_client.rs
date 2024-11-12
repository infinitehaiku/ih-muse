use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use once_cell::sync::Lazy;
use tokio::sync::Mutex;

use ih_muse_core::{Error, Transport};
use ih_muse_proto::{
    ElementId, ElementKindRegistration, ElementRegistration, MetricDefinition, MetricPayload,
    NodeElementRange, NodeState, TimestampResolution,
};

static NEXT_ELEMENT_ID: Lazy<AtomicU64> = Lazy::new(|| AtomicU64::new(0));

/// Get a new unique ElementId as a `u64`
pub fn get_new_element_id() -> u64 {
    NEXT_ELEMENT_ID.fetch_add(1, Ordering::SeqCst)
}

pub struct MockClient {
    metrics: Arc<Mutex<Vec<MetricDefinition>>>,
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

    async fn send_metrics(&self, payload: Vec<MetricPayload>) -> Result<(), Error> {
        log::info!("MockClient: send_metrics called with {:?}", payload);
        Ok(())
    }
}
