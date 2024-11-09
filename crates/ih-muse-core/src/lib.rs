// crates/ih-muse-core/src/lib.rs

mod cache;
mod errors;
pub mod time;

use async_trait::async_trait;

pub use cache::{CacheStrategy, DummyCacheStrategy};
pub use errors::Error;
pub use ih_muse_proto::{
    types::*, ElementKindRegistration, ElementRegistration, MetricPayload, MetricRegistration,
    TimestampResolution,
};

#[async_trait]
pub trait Transport {
    async fn health_check(&self) -> Result<(), Error>;
    async fn get_finest_resolution(&self) -> Result<TimestampResolution, Error>;
    async fn register_element_kinds(
        &self,
        element_kinds: Vec<ElementKindRegistration>,
    ) -> Result<(), Error>;
    async fn register_elements(
        &self,
        elements: Vec<ElementRegistration>,
    ) -> Result<Vec<Result<ElementId, Error>>, Error>;
    async fn register_metrics(&self, payload: Vec<MetricRegistration>) -> Result<(), Error>;
    async fn send_metrics(&self, payload: Vec<MetricPayload>) -> Result<(), Error>;
}
