// crates/ih-muse-core/src/transport.rs

use std::net::SocketAddr;

use async_trait::async_trait;

pub use crate::errors::Error;
pub use ih_muse_proto::{
    types::*, ElementKindRegistration, ElementRegistration, MetricDefinition, MetricPayload,
    MetricQuery, NodeElementRange, NodeState, TimestampResolution,
};

#[async_trait]
pub trait Transport {
    async fn health_check(&self) -> Result<(), Error>;
    async fn get_node_state(&self) -> Result<NodeState, Error>;
    async fn get_finest_resolution(&self) -> Result<TimestampResolution, Error>;
    async fn register_element_kinds(
        &self,
        element_kinds: &[ElementKindRegistration],
    ) -> Result<(), Error>;
    async fn register_elements(
        &self,
        elements: &[ElementRegistration],
    ) -> Result<Vec<Result<ElementId, Error>>, Error>;
    async fn get_node_elem_ranges(
        &self,
        ini: Option<u64>,
        end: Option<u64>,
    ) -> Result<Vec<NodeElementRange>, Error>;
    async fn register_metrics(&self, payload: &[MetricDefinition]) -> Result<(), Error>;
    async fn get_metric_order(&self) -> Result<Vec<MetricDefinition>, Error>;
    async fn get_metrics(
        &self,
        query: &MetricQuery,
        node_addr: Option<SocketAddr>,
    ) -> Result<Vec<MetricPayload>, Error>;
    async fn send_metrics(
        &self,
        payload: Vec<MetricPayload>,
        node_addr: Option<SocketAddr>,
    ) -> Result<(), Error>;
}
