// crates/ih-muse-client/src/poet_client.rs

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use reqwest::{Client, StatusCode};
use tokio::time::sleep;
use uuid::Uuid;

use ih_muse_core::{MuseError, MuseResult, Transport};
use ih_muse_proto::*;

pub struct PoetClient {
    client: Client,
    endpoints: Vec<String>,
    producer_id: Uuid,
    next_batch_sequence: AtomicU64,
    preferred_endpoint: AtomicUsize,
}

impl PoetClient {
    const MAX_ATTEMPTS_PER_ENDPOINT: usize = 3;

    pub fn new(endpoints: &[String]) -> Self {
        let client = Client::new();
        Self {
            client,
            endpoints: endpoints.to_vec(),
            producer_id: Uuid::new_v4(),
            next_batch_sequence: AtomicU64::new(1),
            preferred_endpoint: AtomicUsize::new(0),
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

    fn producer_batch(&self, measurements: Vec<MetricPayload>) -> MuseResult<ProducerEnvelope> {
        let from_dt = measurements
            .iter()
            .map(|measurement| measurement.time)
            .min()
            .ok_or_else(|| MuseError::Validation("producer batch cannot be empty".to_string()))?;
        let to_dt = measurements
            .iter()
            .map(|measurement| measurement.time)
            .max()
            .and_then(|time| time.checked_add(1))
            .ok_or_else(|| MuseError::Validation("producer timestamp overflow".to_string()))?;
        Ok(ProducerEnvelope::new(ProducerBatch {
            producer_id: self.producer_id,
            batch_id: Uuid::new_v4(),
            sequence: self.next_batch_sequence.fetch_add(1, Ordering::Relaxed),
            definitions_revision: 1,
            from_dt,
            to_dt,
            measurements,
        }))
    }

    async fn post_producer_to(&self, url: &str, envelope: &ProducerEnvelope) -> MuseResult<()> {
        match self.client.post(url).json(envelope).send().await {
            Ok(response) if response.status().is_success() => Ok(()),
            Ok(response) if response.status() == StatusCode::TOO_MANY_REQUESTS => Err(
                MuseError::Backpressure(format!("{url}: HTTP {}", response.status())),
            ),
            Ok(response) if response.status().is_client_error() => Err(MuseError::Validation(
                format!("{url}: HTTP {}", response.status()),
            )),
            Ok(response) => Err(MuseError::Unavailable(format!(
                "{url}: HTTP {}",
                response.status()
            ))),
            Err(error) => Err(MuseError::Unavailable(format!("{url}: {error}"))),
        }
    }

    async fn post_producer_batch(&self, envelope: &ProducerEnvelope) -> MuseResult<()> {
        if self.endpoints.is_empty() {
            return Err(MuseError::Configuration(
                "at least one Poet endpoint is required".to_string(),
            ));
        }
        let first = self.preferred_endpoint.load(Ordering::Relaxed) % self.endpoints.len();
        let mut failures = Vec::new();
        for offset in 0..self.endpoints.len() {
            let endpoint_index = (first + offset) % self.endpoints.len();
            let endpoint = &self.endpoints[endpoint_index];
            let url = format!("{endpoint}/producer/v1/batches");
            for attempt in 0..Self::MAX_ATTEMPTS_PER_ENDPOINT {
                match self.post_producer_to(&url, envelope).await {
                    Ok(()) => {
                        self.preferred_endpoint
                            .store(endpoint_index, Ordering::Relaxed);
                        return Ok(());
                    }
                    Err(MuseError::Unavailable(error)) => {
                        failures.push(error);
                        if attempt + 1 < Self::MAX_ATTEMPTS_PER_ENDPOINT {
                            sleep(Self::retry_delay(envelope, attempt)).await;
                        }
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        Err(MuseError::Unavailable(failures.join("; ")))
    }

    fn retry_delay(envelope: &ProducerEnvelope, attempt: usize) -> Duration {
        let base_ms = 25_u64 << attempt;
        let jitter_ms = envelope.batch.batch_id.as_u128() as u64 % 10;
        Duration::from_millis(base_ms + jitter_ms)
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
        let envelope = self.producer_batch(payload)?;
        if let Some(node_addr) = node_addr {
            let url = format!("http://{node_addr}/producer/v1/batches");
            return self.post_producer_to(&url, &envelope).await;
        }
        self.send_producer_batch(envelope).await
    }

    async fn send_producer_batch(&self, envelope: ProducerEnvelope) -> MuseResult<()> {
        envelope
            .validate()
            .map_err(|error| MuseError::Validation(error.to_string()))?;
        self.post_producer_batch(&envelope).await
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::task::JoinHandle;

    fn payload(time: i64) -> MetricPayload {
        MetricPayload::new(time, 1, vec![1], vec![Some(1.0)])
    }

    #[test]
    fn producer_batches_have_monotonic_sequences_and_half_open_bounds() {
        let client = PoetClient::new(&["http://127.0.0.1:1".to_string()]);
        let first = client
            .producer_batch(vec![payload(12), payload(10)])
            .unwrap();
        let second = client.producer_batch(vec![payload(20)]).unwrap();

        assert_eq!(first.batch.from_dt, 10);
        assert_eq!(first.batch.to_dt, 13);
        assert_eq!(first.batch.sequence, 1);
        assert_eq!(second.batch.sequence, 2);
        assert_ne!(first.batch.batch_id, second.batch.batch_id);
        assert!(first.validate().is_ok());
    }

    #[test]
    fn producer_batches_reject_empty_measurements() {
        let client = PoetClient::new(&["http://127.0.0.1:1".to_string()]);
        assert!(matches!(
            client.producer_batch(Vec::new()),
            Err(MuseError::Validation(_))
        ));
    }

    async fn fixture_server(
        status: &str,
        expected_requests: usize,
    ) -> (String, Arc<Mutex<Vec<String>>>, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let status = status.to_string();
        let task = tokio::spawn(async move {
            for _ in 0..expected_requests {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = vec![0; 16 * 1024];
                let size = socket.read(&mut request).await.unwrap();
                captured
                    .lock()
                    .unwrap()
                    .push(String::from_utf8(request[..size].to_vec()).unwrap());
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 {status}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            }
        });
        (format!("http://{address}"), requests, task)
    }

    #[tokio::test]
    async fn producer_retries_then_fails_over_with_a_stable_batch_id() {
        let (unavailable_url, unavailable_requests, unavailable_task) = fixture_server(
            "503 Service Unavailable",
            PoetClient::MAX_ATTEMPTS_PER_ENDPOINT,
        )
        .await;
        let (healthy_url, healthy_requests, healthy_task) = fixture_server("201 Created", 2).await;
        let client = PoetClient::new(&[unavailable_url, healthy_url]);
        let envelope = client.producer_batch(vec![payload(10)]).unwrap();
        let batch_id = envelope.batch.batch_id.to_string();

        client.send_producer_batch(envelope).await.unwrap();
        client.send_metrics(vec![payload(11)], None).await.unwrap();

        unavailable_task.await.unwrap();
        healthy_task.await.unwrap();
        let unavailable_requests = unavailable_requests.lock().unwrap();
        let healthy_requests = healthy_requests.lock().unwrap();
        assert_eq!(
            unavailable_requests.len(),
            PoetClient::MAX_ATTEMPTS_PER_ENDPOINT
        );
        assert_eq!(healthy_requests.len(), 2);
        assert!(unavailable_requests
            .iter()
            .all(|request| request.contains(&batch_id)));
        assert!(healthy_requests[0].contains(&batch_id));
    }
}
