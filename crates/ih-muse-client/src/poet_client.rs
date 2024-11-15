// crates/ih-muse-client/src/poet_client.rs

use std::net::SocketAddr;

use async_trait::async_trait;
use reqwest::{Client, StatusCode};

use ih_muse_core::{MuseError, MuseResult, Transport};
use ih_muse_proto::*;

pub struct PoetClient {
    client: Client,
    endpoints: Vec<String>,
    // You can add cache_strategy here if needed in the future
}

impl PoetClient {
    pub fn new(endpoints: &[String]) -> Self {
        let client = Client::new();
        Self {
            client,
            endpoints: endpoints.to_vec(),
        }
    }

    fn get_base_url(&self) -> &str {
        // TODO rotate endpoints on failure
        self.endpoints.first().unwrap()
    }

    /// Returns the base URL or constructs it from `node_addr` if provided.
    fn build_url(&self, path: &str, node_addr: Option<SocketAddr>) -> String {
        match node_addr {
            Some(addr) => format!("http://{}{}", addr, path),
            None => format!("{}{}", self.get_base_url(), path),
        }
    }
}

#[async_trait]
impl Transport for PoetClient {
    async fn health_check(&self) -> MuseResult<()> {
        let url = format!("{}/health", self.get_base_url());
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| MuseError::Client(format!("Failed to perform health check: {}", e)))?;

        if response.status().is_success() {
            Ok(())
        } else {
            Err(MuseError::Client(format!(
                "Health check failed: HTTP {}",
                response.status()
            )))
        }
    }

    async fn get_node_state(&self) -> MuseResult<NodeState> {
        let url = format!("{}/sync/state", self.get_base_url());
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| MuseError::Client(format!("Failed to retrieve node state: {}", e)))?;

        if response.status().is_success() {
            let resp_haikus: NodeState = response.json().await.map_err(|e| {
                MuseError::Client(format!("Failed to parse response as NodeState: {e}"))
            })?;
            Ok(resp_haikus)
        } else {
            Err(MuseError::Client(format!(
                "Get Finest Resolution failed: {}",
                response.status()
            )))
        }
    }

    async fn get_finest_resolution(&self) -> MuseResult<TimestampResolution> {
        let url = format!("{}/config/finest_resolution", self.get_base_url());
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| MuseError::Client(format!("Failed to perform health check: {}", e)))?;

        if response.status().is_success() {
            let resp_haikus: TimestampResolution = response.json().await.map_err(|e| {
                MuseError::Client(format!(
                    "Failed to parse response as TimestampResolution: {e}"
                ))
            })?;
            Ok(resp_haikus)
        } else {
            Err(MuseError::Client(format!(
                "Get Finest Resolution failed: {}",
                response.status()
            )))
        }
    }

    async fn get_node_elem_ranges(
        &self,
        ini: Option<u64>,
        end: Option<u64>,
    ) -> MuseResult<Vec<NodeElementRange>> {
        let url = format!("{}/ds/elements/ranges", self.get_base_url());
        let response = self
            .client
            .get(&url)
            .json(&GetRangesRequest { ini, end })
            .send()
            .await
            .map_err(|e| MuseError::Client(format!("Failed to retrieve node state: {}", e)))?;

        if response.status().is_success() {
            let ranges: Vec<NodeElementRange> = response.json().await.map_err(|e| {
                MuseError::Client(format!(
                    "Failed to parse response as Vec<NodeElementRange>: {e}"
                ))
            })?;
            Ok(ranges)
        } else {
            Err(MuseError::Client(format!(
                "Get All Element Ranges failed: {}",
                response.status()
            )))
        }
    }

    async fn register_metrics(&self, payload: &[MetricDefinition]) -> MuseResult<()> {
        let url = format!("{}/ds/metrics", self.get_base_url());
        let response = self
            .client
            .post(&url)
            .json(payload)
            .send()
            .await
            .map_err(|e| MuseError::Client(format!("Failed to send metric: {e}")))?;

        if response.status().is_success() {
            Ok(())
        } else {
            Err(MuseError::Client(format!(
                "Failed to send metric: HTTP {}",
                response.status()
            )))
        }
    }

    async fn get_metric_order(&self) -> MuseResult<Vec<MetricDefinition>> {
        let url = format!("{}/ds/metrics", self.get_base_url());
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| MuseError::Client(format!("Failed to send metric: {}", e)))?;

        if response.status().is_success() {
            let metric_defs: Vec<MetricDefinition> = response.json().await.map_err(|e| {
                MuseError::Client(format!(
                    "Failed to parse response as Vec<MetricDefinition>: {e}"
                ))
            })?;
            Ok(metric_defs)
        } else {
            Err(MuseError::Client(format!(
                "Failed to send metric: HTTP {}",
                response.status()
            )))
        }
    }

    async fn send_metrics(
        &self,
        payload: Vec<MetricPayload>,
        node_addr: Option<SocketAddr>,
    ) -> MuseResult<()> {
        let url = self.build_url("/ds/abs_metrics", node_addr);
        let response = self
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| MuseError::Client(format!("Failed to send metric: {}", e)))?;

        if response.status().is_success() {
            Ok(())
        } else {
            Err(MuseError::Client(format!(
                "Failed to send metric: HTTP {}",
                response.status()
            )))
        }
    }

    async fn get_metrics(
        &self,
        query: &MetricQuery,
        node_addr: Option<SocketAddr>,
    ) -> MuseResult<Vec<MetricPayload>> {
        let url = self.build_url("/ds/abs_metrics", node_addr);
        let response = self
            .client
            .get(&url)
            .json(query)
            .send()
            .await
            .map_err(|e| MuseError::Client(format!("Failed to get metrics: {}", e)))?;

        if response.status().is_success() {
            let metrics: Vec<MetricPayload> = response.json().await.map_err(|e| {
                MuseError::Client(format!("Failed to parse metrics response: {}", e))
            })?;
            Ok(metrics)
        } else {
            Err(MuseError::Client(format!(
                "Failed to get metrics: HTTP {}",
                response.status()
            )))
        }
    }

    async fn register_elements(
        &self,
        elements: &[ElementRegistration],
    ) -> MuseResult<Vec<MuseResult<ElementId>>> {
        let url = format!("{}/ds/elements", self.get_base_url());
        let response = self
            .client
            .post(&url)
            .json(elements)
            .send()
            .await
            .map_err(|e| MuseError::Client(format!("Failed to register elements: {}", e)))?;

        match response.status() {
            StatusCode::CREATED | StatusCode::MULTI_STATUS | StatusCode::BAD_REQUEST => {
                // Deserialize response as `NewElementsResponse` for all relevant cases
                let response_data: NewElementsResponse = response
                    .json()
                    .await
                    .map_err(|e| MuseError::Client(format!("Failed to parse response: {}", e)))?;

                // Convert Vec<Result<u64, String>> to Vec<Result<ElementId, Error>>
                let results = response_data
                    .results
                    .into_iter()
                    .map(|res| res.map_err(MuseError::Client))
                    .collect();

                Ok(results)
            }
            status => {
                // Handle any unexpected HTTP status codes
                Err(MuseError::Client(format!(
                    "Failed to register elements: HTTP {}",
                    status
                )))
            }
        }
    }

    async fn register_element_kinds(
        &self,
        element_kind: &[ElementKindRegistration],
    ) -> MuseResult<()> {
        let url = format!("{}/ds/element_kinds", self.get_base_url());
        let response = self
            .client
            .post(&url)
            .json(element_kind)
            .send()
            .await
            .map_err(|e| MuseError::Client(format!("Failed to register element kind: {}", e)))?;

        if response.status().is_success() {
            Ok(())
        } else {
            Err(MuseError::Client(format!(
                "Failed to register element kind: HTTP {}",
                response.status()
            )))
        }
    }
}
