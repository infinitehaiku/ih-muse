// crates/ih-muse-client/src/poet_client.rs

use async_trait::async_trait;
use reqwest::{Client, StatusCode};

use ih_muse_core::{Error, Transport};
use ih_muse_proto::{
    ElementId, ElementKindRegistration, ElementRegistration, MetricPayload, MetricRegistration,
    NewElementsResponse, TimestampResolution,
};

pub struct PoetEndpoint {
    pub url: String,
}

pub struct PoetClient {
    client: Client,
    endpoints: Vec<PoetEndpoint>,
    // You can add cache_strategy here if needed in the future
}

impl PoetClient {
    pub fn new(endpoints: Vec<PoetEndpoint>) -> Self {
        let client = Client::new();
        Self { client, endpoints }
    }

    fn get_base_url(&self) -> String {
        // For simplicity, use the first endpoint
        self.endpoints.first().unwrap().url.clone()
    }
}

#[async_trait]
impl Transport for PoetClient {
    async fn health_check(&self) -> Result<(), Error> {
        let url = format!("{}/health", self.get_base_url());
        let response =
            self.client.get(&url).send().await.map_err(|e| {
                Error::ClientError(format!("Failed to perform health check: {}", e))
            })?;

        if response.status().is_success() {
            Ok(())
        } else {
            Err(Error::ClientError(format!(
                "Health check failed: HTTP {}",
                response.status()
            )))
        }
    }

    async fn get_finest_resolution(&self) -> Result<TimestampResolution, Error> {
        let url = format!("{}/config/finest_resolution", self.get_base_url());
        let response =
            self.client.get(&url).send().await.map_err(|e| {
                Error::ClientError(format!("Failed to perform health check: {}", e))
            })?;

        if response.status().is_success() {
            let resp_haikus: TimestampResolution = response.json().await.map_err(|e| {
                Error::ClientError(format!(
                    "Failed to parse response as TimestampResolution: {}",
                    e
                ))
            })?;
            Ok(resp_haikus)
        } else {
            Err(Error::ClientError(format!(
                "Get Finest Resolution failed: {}",
                response.status()
            )))
        }
    }

    async fn register_metrics(&self, payload: Vec<MetricRegistration>) -> Result<(), Error> {
        let url = format!("{}/ds/metrics", self.get_base_url());
        let response = self
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| Error::ClientError(format!("Failed to send metric: {}", e)))?;

        if response.status().is_success() {
            Ok(())
        } else {
            Err(Error::ClientError(format!(
                "Failed to send metric: HTTP {}",
                response.status()
            )))
        }
    }

    async fn send_metrics(&self, payload: Vec<MetricPayload>) -> Result<(), Error> {
        let url = format!("{}/ds/abs_metrics", self.get_base_url());
        let response = self
            .client
            .post(&url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| Error::ClientError(format!("Failed to send metric: {}", e)))?;

        if response.status().is_success() {
            Ok(())
        } else {
            Err(Error::ClientError(format!(
                "Failed to send metric: HTTP {}",
                response.status()
            )))
        }
    }

    async fn register_elements(
        &self,
        elements: Vec<ElementRegistration>,
    ) -> Result<Vec<Result<ElementId, Error>>, Error> {
        let url = format!("{}/ds/elements", self.get_base_url());
        let response = self
            .client
            .post(&url)
            .json(&elements)
            .send()
            .await
            .map_err(|e| Error::ClientError(format!("Failed to register elements: {}", e)))?;

        match response.status() {
            StatusCode::CREATED | StatusCode::MULTI_STATUS | StatusCode::BAD_REQUEST => {
                // Deserialize response as `NewElementsResponse` for all relevant cases
                let response_data: NewElementsResponse = response
                    .json()
                    .await
                    .map_err(|e| Error::ClientError(format!("Failed to parse response: {}", e)))?;

                // Convert Vec<Result<u64, String>> to Vec<Result<ElementId, Error>>
                let results = response_data
                    .results
                    .into_iter()
                    .map(|res| res.map_err(Error::ClientError))
                    .collect();

                Ok(results)
            }
            status => {
                // Handle any unexpected HTTP status codes
                Err(Error::ClientError(format!(
                    "Failed to register elements: HTTP {}",
                    status
                )))
            }
        }
    }

    async fn register_element_kinds(
        &self,
        element_kind: Vec<ElementKindRegistration>,
    ) -> Result<(), Error> {
        let url = format!("{}/ds/element_kinds", self.get_base_url());
        let response = self
            .client
            .post(&url)
            .json(&element_kind)
            .send()
            .await
            .map_err(|e| Error::ClientError(format!("Failed to register element kind: {}", e)))?;

        if response.status().is_success() {
            Ok(())
        } else {
            Err(Error::ClientError(format!(
                "Failed to register element kind: HTTP {}",
                response.status()
            )))
        }
    }
}
