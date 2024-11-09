use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use once_cell::sync::Lazy;

use ih_muse_core::{Error, Transport};
use ih_muse_proto::{
    ElementId, ElementKindRegistration, ElementRegistration, MetricPayload, MetricRegistration,
};

static NEXT_ELEMENT_ID: Lazy<AtomicU64> = Lazy::new(|| AtomicU64::new(0));

/// Get a new unique ElementId as a `u64`
pub fn get_new_element_id() -> u64 {
    NEXT_ELEMENT_ID.fetch_add(1, Ordering::SeqCst)
}

pub struct MockClient;

impl Default for MockClient {
    fn default() -> Self {
        Self::new()
    }
}

impl MockClient {
    pub fn new() -> Self {
        MockClient
    }
}

#[async_trait]
impl Transport for MockClient {
    async fn health_check(&self) -> Result<(), Error> {
        println!("MockClient: health_check called");
        Ok(())
    }

    async fn register_element_kinds(
        &self,
        element_kinds: Vec<ElementKindRegistration>,
    ) -> Result<(), Error> {
        println!(
            "MockClient: register_element_kinds called with {:?}",
            element_kinds
        );
        Ok(())
    }

    async fn register_elements(
        &self,
        elements: Vec<ElementRegistration>,
    ) -> Result<Vec<Result<ElementId, Error>>, Error> {
        println!("MockClient: register_elements called with {:?}", elements);
        let results = elements
            .into_iter()
            .map(|_| Ok(get_new_element_id()))
            .collect();
        Ok(results)
    }

    async fn register_metrics(&self, payload: Vec<MetricRegistration>) -> Result<(), Error> {
        println!("MockClient: register_metrics called with {:?}", payload);
        Ok(())
    }

    async fn send_metrics(&self, payload: Vec<MetricPayload>) -> Result<(), Error> {
        println!("MockClient: send_metrics called with {:?}", payload);
        Ok(())
    }
}
