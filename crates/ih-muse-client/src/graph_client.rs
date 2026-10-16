//! Authenticated, bounded delivery for the canonical graph intake contract.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use ih_muse_core::{MuseError, MuseResult};
use ih_muse_proto::trace::{DeliveryTrace, Sampler, TRACEPARENT, TRACESTATE};
use ih_muse_proto::{GraphIntakeAnswer, GraphIntakeRequest};
use reqwest::{Client, StatusCode};
use tokio::time::sleep;

/// Sends validated `ih.graph.intake.v1` batches to a Poet cluster.
///
/// Any Poet of the cluster accepts a batch; the Poets replicate among
/// themselves. The client sticks to the Poet that last accepted a batch and
/// moves to the next one when it fails transiently, so one Muse feeds the
/// cluster through an outage. The bearer token is held only in this client
/// and is never included in an error. Delivery IDs are caller-owned and stay
/// stable across retries and Poets, so a retry after a lost acknowledgement
/// is stored once.
///
/// Every delivery carries a W3C trace context (`traceparent` and
/// `tracestate` headers, [`DeliveryTrace`]), sampled by [`Sampler`] from
/// `IH_TRACE_SAMPLE_RATIO`: the Poet that accepts it records the Muse's
/// delivery and its own intake as one trace. The body is unchanged.
///
/// A Poet that is busy answers `429 Too Many Requests` (or `503 Service
/// Unavailable`) with `Retry-After`: the client waits that long (clamped to
/// [`Self::RETRY_AFTER_CAP`]) and sends the same delivery to the same Poet
/// again. When the Poet is still busy after that much waiting in total, the
/// call ends in [`MuseError::Backpressure`], which callers keep queued.
#[derive(Clone)]
pub struct GraphPoetClient {
    endpoints: Vec<String>,
    /// Index of the Poet that last accepted a batch; shared by clones.
    preferred: Arc<AtomicUsize>,
    token: String,
    client: Client,
    /// Head sampling of the delivery traces this client starts.
    sampler: Sampler,
    /// Longest single `Retry-After` wait, and the total waiting per call.
    retry_after_cap: Duration,
}

impl GraphPoetClient {
    /// Attempts per Poet before moving to the next one.
    const MAX_ATTEMPTS: usize = 2;
    const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
    /// Default bound on one `Retry-After` wait and on the total waiting of
    /// one call; a busy Poet cannot stall a Muse longer than this.
    pub const RETRY_AFTER_CAP: Duration = Duration::from_secs(60);
    /// The wait when a busy answer names no usable `Retry-After`.
    const DEFAULT_RETRY_AFTER: Duration = Duration::from_secs(1);

    /// Build a bounded graph client for an HTTPS or loopback HTTP Poet endpoint.
    pub fn new(endpoint: impl Into<String>, token: impl Into<String>) -> MuseResult<Self> {
        Self::build(vec![endpoint.into()], token.into(), None)
    }

    /// Build a client for every Poet of a cluster, in preference order.
    pub fn cluster(endpoints: Vec<String>, token: impl Into<String>) -> MuseResult<Self> {
        Self::build(endpoints, token.into(), None)
    }

    /// Build a bounded HTTPS graph client that trusts one owner-provided CA.
    pub fn private_tls(
        endpoint: impl Into<String>,
        token: impl Into<String>,
        ca_pem: &[u8],
    ) -> MuseResult<Self> {
        Self::build(vec![endpoint.into()], token.into(), Some(ca_pem))
    }

    /// Build a client for every Poet of a cluster that serves HTTPS with one
    /// owner-provided CA (an in-cluster Muse reaching Poet's private TLS port).
    pub fn cluster_private_tls(
        endpoints: Vec<String>,
        token: impl Into<String>,
        ca_pem: &[u8],
    ) -> MuseResult<Self> {
        Self::build(endpoints, token.into(), Some(ca_pem))
    }

    /// The Poet the next batch goes to first.
    pub fn preferred_endpoint(&self) -> &str {
        &self.endpoints[self.preferred.load(Ordering::Relaxed) % self.endpoints.len()]
    }

    fn build(endpoints: Vec<String>, token: String, ca_pem: Option<&[u8]>) -> MuseResult<Self> {
        let endpoints = endpoints
            .into_iter()
            .map(|endpoint| endpoint.trim().trim_end_matches('/').to_owned())
            .filter(|endpoint| !endpoint.is_empty())
            .collect::<Vec<_>>();
        if endpoints.is_empty() || endpoints.len() > 16 {
            return Err(MuseError::Configuration(
                "graph client needs 1 to 16 Poet endpoints".into(),
            ));
        }
        for endpoint in &endpoints {
            if !(endpoint.starts_with("https://") || endpoint.starts_with("http://127.0.0.1")) {
                return Err(MuseError::Configuration(
                    "graph Poet endpoint must use HTTPS or explicit loopback HTTP".into(),
                ));
            }
            if ca_pem.is_some() && !endpoint.starts_with("https://") {
                return Err(MuseError::Configuration(
                    "an owner CA requires an HTTPS graph Poet endpoint".into(),
                ));
            }
        }
        if token.is_empty() || token.len() > 4096 || token.contains(['\r', '\n', '\0']) {
            return Err(MuseError::Configuration(
                "graph Poet bearer token is invalid".into(),
            ));
        }
        let mut builder = Client::builder().timeout(Self::REQUEST_TIMEOUT);
        if let Some(ca_pem) = ca_pem {
            if ca_pem.len() > 1024 * 1024
                || !ca_pem.starts_with(b"-----BEGIN CERTIFICATE-----")
                || !ca_pem.ends_with(b"-----END CERTIFICATE-----\n")
            {
                return Err(MuseError::Configuration(
                    "graph Poet owner CA is not bounded PEM".into(),
                ));
            }
            let certificate = reqwest::Certificate::from_pem(ca_pem).map_err(|_| {
                MuseError::Configuration("graph Poet owner CA is not valid PEM".into())
            })?;
            builder = builder.add_root_certificate(certificate);
        }
        let client = builder
            .build()
            .map_err(|error| MuseError::Unavailable(error.to_string()))?;
        Ok(Self {
            endpoints,
            preferred: Arc::new(AtomicUsize::new(0)),
            token,
            client,
            sampler: Sampler::from_env(),
            retry_after_cap: Self::RETRY_AFTER_CAP,
        })
    }

    /// Replaces the bound on `Retry-After` waits ([`Self::RETRY_AFTER_CAP`]).
    pub fn with_retry_after_cap(mut self, cap: Duration) -> Self {
        self.retry_after_cap = cap;
        self
    }

    /// The wait a busy answer asks for: `Retry-After` in seconds (an HTTP
    /// date or a missing header waits [`Self::DEFAULT_RETRY_AFTER`]),
    /// clamped to `cap`.
    fn retry_after(headers: &reqwest::header::HeaderMap, cap: Duration) -> Duration {
        headers
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok())
            .map_or(Self::DEFAULT_RETRY_AFTER, Duration::from_secs)
            .min(cap)
    }

    /// A Poet asking the Muse to slow down: 429, or 503 naming a
    /// `Retry-After`. A bare 503 is an outage and fails over instead.
    fn is_busy(response: &reqwest::Response) -> bool {
        response.status() == StatusCode::TOO_MANY_REQUESTS
            || (response.status() == StatusCode::SERVICE_UNAVAILABLE
                && response
                    .headers()
                    .contains_key(reqwest::header::RETRY_AFTER))
    }

    /// Replaces the sampling of the delivery traces this client starts.
    pub fn with_sampler(mut self, sampler: Sampler) -> Self {
        self.sampler = sampler;
        self
    }

    /// A delivery trace for `request`: a new trace that starts when the
    /// batch's newest observation was made (else now).
    pub fn delivery_trace(&self, request: &GraphIntakeRequest) -> DeliveryTrace {
        let observed = request
            .batch
            .observations
            .iter()
            .map(|observation| observation.provenance.observed_at_unix_nano)
            .max()
            .filter(|time| *time > 0);
        let created = observed.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos().min(u64::MAX as u128) as u64)
        });
        DeliveryTrace::start(&self.sampler, created)
    }

    /// Deliver a prevalidated graph batch, retrying only transient failures,
    /// and return the accepting Poet's answer (its definitions epoch).
    ///
    /// Starts at the preferred Poet and moves on after `MAX_ATTEMPTS`
    /// transient failures. A busy answer (429, or 503 with `Retry-After`)
    /// is retried on the same Poet after the asked delay; see the type's
    /// documentation. Any other 4xx answer is final: another Poet of the
    /// same cluster would reject the same batch for the same reason.
    pub async fn publish(&self, request: &GraphIntakeRequest) -> MuseResult<GraphIntakeAnswer> {
        let trace = self.delivery_trace(request);
        self.publish_traced(request, &trace).await
    }

    /// [`Self::publish`] with the caller's delivery trace (the same on
    /// every retry of one delivery).
    pub async fn publish_traced(
        &self,
        request: &GraphIntakeRequest,
        trace: &DeliveryTrace,
    ) -> MuseResult<GraphIntakeAnswer> {
        request
            .validate()
            .map_err(|error| MuseError::Validation(error.to_string()))?;
        let traceparent = trace.context.traceparent();
        let tracestate = trace.tracestate();
        let body = self
            .post(
                "/api/v1/graph/batches",
                "graph intake",
                Body::Json(request),
                Some((&traceparent, &tracestate)),
            )
            .await
            .map_err(|failure| failure.error)?;
        // Stored already: an answer that does not parse (an empty body)
        // only means the Poet named no epoch.
        Ok(serde_json::from_slice(&body).unwrap_or_default())
    }

    /// Deliver an OTLP/HTTP logs export (`ExportLogsServiceRequest`,
    /// protobuf) to `POST /v1/logs`, with the same failover and
    /// back-pressure as [`Self::publish`], and return the accepting Poet's
    /// `ExportLogsServiceResponse` bytes (empty when it sent none).
    ///
    /// A refused export (another 4xx) is [`MuseError::Validation`]: the
    /// same bytes would be refused again. A refused token (401, 403) is
    /// [`MuseError::Client`]: the export is fine and is kept for when the
    /// token is fixed.
    pub async fn publish_otlp_logs(&self, export: &[u8]) -> MuseResult<Vec<u8>> {
        self.post("/v1/logs", "OTLP logs intake", Body::Protobuf(export), None)
            .await
            .map_err(|failure| match failure.status {
                Some(StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) => {
                    MuseError::Client(failure.error.to_string())
                }
                _ => failure.error,
            })
    }

    /// One delivery to `path` on the preferred Poet, failing over and
    /// waiting out back-pressure as the type's documentation says; the
    /// accepting Poet's answer body.
    async fn post(
        &self,
        path: &str,
        what: &str,
        body: Body<'_>,
        trace: Option<(&str, &str)>,
    ) -> Result<Vec<u8>, Failure> {
        let start = self.preferred.load(Ordering::Relaxed);
        let mut failures = Vec::new();
        let mut waited = Duration::ZERO;
        for offset in 0..self.endpoints.len() {
            let index = (start + offset) % self.endpoints.len();
            let url = format!("{}{path}", self.endpoints[index]);
            let mut attempt = 0;
            while attempt < Self::MAX_ATTEMPTS {
                let mut builder = self.client.post(&url).bearer_auth(&self.token);
                if let Some((traceparent, tracestate)) = trace {
                    builder = builder
                        .header(TRACEPARENT, traceparent)
                        .header(TRACESTATE, tracestate);
                }
                builder = match body {
                    Body::Json(request) => builder.json(request),
                    Body::Protobuf(bytes) => builder
                        .header(reqwest::header::CONTENT_TYPE, "application/x-protobuf")
                        .body(bytes.to_vec()),
                };
                match builder.send().await {
                    Ok(response)
                        if matches!(response.status(), StatusCode::CREATED | StatusCode::OK) =>
                    {
                        self.preferred.store(index, Ordering::Relaxed);
                        return Ok(response.bytes().await.unwrap_or_default().to_vec());
                    }
                    Ok(response) if Self::is_busy(&response) => {
                        let delay = Self::retry_after(response.headers(), self.retry_after_cap);
                        if waited + delay > self.retry_after_cap {
                            return Err(Failure {
                                error: MuseError::Backpressure(format!(
                                    "{what} busy on Poet {index} after {}s of waiting: HTTP {}",
                                    waited.as_secs(),
                                    response.status()
                                )),
                                status: Some(response.status()),
                            });
                        }
                        waited += delay;
                        sleep(delay).await;
                        // Back-pressure is not a failure: same Poet, same
                        // delivery, without using up an attempt.
                        continue;
                    }
                    Ok(response) if response.status().is_client_error() => {
                        return Err(Failure {
                            error: MuseError::Validation(format!(
                                "{what} rejected delivery: HTTP {}",
                                response.status()
                            )),
                            status: Some(response.status()),
                        });
                    }
                    Ok(response) => {
                        failures.push(format!("Poet {index}: HTTP {}", response.status()))
                    }
                    Err(error) => failures.push(format!("Poet {index}: {error}")),
                }
                if attempt + 1 < Self::MAX_ATTEMPTS {
                    sleep(Duration::from_millis(25_u64 << attempt)).await;
                }
                attempt += 1;
            }
        }
        Err(Failure {
            error: MuseError::Unavailable(format!(
                "{what} unavailable on {} Poet(s): {}",
                self.endpoints.len(),
                failures.join("; ")
            )),
            status: None,
        })
    }
}

/// What a delivery carries.
#[derive(Clone, Copy)]
enum Body<'a> {
    Json(&'a GraphIntakeRequest),
    Protobuf(&'a [u8]),
}

/// A delivery that did not end in a 2xx, with the final answer's status.
struct Failure {
    error: MuseError,
    status: Option<StatusCode>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_transport_security_or_explicit_loopback() {
        assert!(GraphPoetClient::new("http://poet.example", "token").is_err());
        assert!(GraphPoetClient::new("https://poet.example", "token").is_ok());
        assert!(GraphPoetClient::new("http://127.0.0.1:18080", "token").is_ok());
        assert!(
            GraphPoetClient::private_tls("http://127.0.0.1:18080", "token", b"not-used").is_err()
        );
        assert!(GraphPoetClient::private_tls(
            "https://poet.example",
            "token",
            b"not-a-certificate"
        )
        .is_err());
        assert!(GraphPoetClient::cluster(Vec::new(), "token").is_err());
        assert!(GraphPoetClient::cluster_private_tls(
            vec![
                "https://poet-0.example".into(),
                "http://127.0.0.1:18080".into()
            ],
            "token",
            b"not-used"
        )
        .is_err());
    }

    /// A tiny HTTP server that answers every request with `status`.
    async fn server(status: &'static str) -> (String, Arc<AtomicUsize>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                counter.fetch_add(1, Ordering::SeqCst);
                let mut buffer = vec![0_u8; 64 * 1024];
                let _ = socket.read(&mut buffer).await;
                let reply =
                    format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                let _ = socket.write_all(reply.as_bytes()).await;
            }
        });
        (format!("http://{address}"), hits)
    }

    fn request() -> GraphIntakeRequest {
        serde_json::from_value(serde_json::json!({
            "schema_version": ih_muse_proto::GRAPH_INTAKE_SCHEMA_VERSION,
            "contract_revision": ih_muse_proto::GRAPH_INTAKE_CONTRACT_REVISION,
            "delivery_id": "muse-test-1",
            "organization": "org",
            "owner_id": "org",
            "batch": {
                "schema_version": ih_muse_proto::GRAPH_SCHEMA_VERSION,
                "contract_revision": ih_muse_proto::GRAPH_CONTRACT_REVISION,
                "entities": [], "relations": [], "observations": [],
                "events": [], "derivations": [], "availability": []
            }
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn a_down_poet_fails_over_and_the_healthy_one_stays_preferred() {
        let (down, down_hits) = server("503 Service Unavailable").await;
        let (up, up_hits) = server("201 Created").await;
        let client = GraphPoetClient::cluster(vec![down.clone(), up.clone()], "token").unwrap();
        client.publish(&request()).await.unwrap();
        assert_eq!(client.preferred_endpoint(), up);
        assert_eq!(
            down_hits.load(Ordering::SeqCst),
            2,
            "retried the first Poet, then moved on"
        );
        client.publish(&request()).await.unwrap();
        assert_eq!(
            down_hits.load(Ordering::SeqCst),
            2,
            "the healthy Poet stays preferred"
        );
        assert_eq!(up_hits.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_rejected_batch_is_not_retried_elsewhere() {
        let (reject, _) = server("400 Bad Request").await;
        let (up, up_hits) = server("201 Created").await;
        let client = GraphPoetClient::cluster(vec![reject, up], "token").unwrap();
        assert!(matches!(
            client.publish(&request()).await,
            Err(MuseError::Validation(_))
        ));
        assert_eq!(up_hits.load(Ordering::SeqCst), 0);
    }

    /// Like [`server`], keeping each request's head (request line and headers).
    async fn recording_server(
        status: &'static str,
    ) -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let heads = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = heads.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let mut buffer = vec![0_u8; 64 * 1024];
                let read = socket.read(&mut buffer).await.unwrap_or(0);
                let text = String::from_utf8_lossy(&buffer[..read]).to_string();
                let head = text
                    .split("\r\n\r\n")
                    .next()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                seen.lock().unwrap().push(head);
                let reply =
                    format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                let _ = socket.write_all(reply.as_bytes()).await;
            }
        });
        (format!("http://{address}"), heads)
    }

    fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
        head.lines()
            .find_map(|line| line.strip_prefix(&format!("{name}: ")))
    }

    #[tokio::test]
    async fn every_attempt_of_a_delivery_carries_the_same_trace() {
        let (down, down_heads) = recording_server("503 Service Unavailable").await;
        let (up, up_heads) = recording_server("201 Created").await;
        let client = GraphPoetClient::cluster(vec![down, up], "token")
            .unwrap()
            .with_sampler(Sampler {
                ratio: 1.0,
                ..Sampler::default()
            });
        let mut request = request();
        request.delivery_id = "muse-test-traced".into();
        let trace = client.delivery_trace(&request);
        client.publish_traced(&request, &trace).await.unwrap();
        let heads: Vec<String> = down_heads
            .lock()
            .unwrap()
            .iter()
            .chain(up_heads.lock().unwrap().iter())
            .cloned()
            .collect();
        assert_eq!(
            heads.len(),
            3,
            "two attempts on the down Poet, one on the healthy one"
        );
        for head in &heads {
            let parent = header(head, TRACEPARENT).expect("traceparent on every attempt");
            let state = header(head, TRACESTATE).expect("tracestate on every attempt");
            assert_eq!(
                DeliveryTrace::from_headers(Some(parent), Some(state)),
                Some(trace.clone())
            );
            assert!(parent.ends_with("-01"), "sampled at ratio 1: {parent}");
        }
    }

    #[tokio::test]
    async fn an_unsampled_delivery_still_names_its_trace() {
        let (up, heads) = recording_server("201 Created").await;
        let client = GraphPoetClient::cluster(vec![up], "token")
            .unwrap()
            .with_sampler(Sampler {
                ratio: 0.0,
                ..Sampler::default()
            });
        client.publish(&request()).await.unwrap();
        let head = heads.lock().unwrap()[0].clone();
        let parent = header(&head, TRACEPARENT).unwrap();
        assert!(parent.ends_with("-00"), "{parent}");
        let created = DeliveryTrace::from_headers(Some(parent), header(&head, TRACESTATE))
            .unwrap()
            .created_unix_nano;
        assert!(created > 0, "an empty batch starts its trace now");
    }
    /// A graph intake that answers its requests with `replies` in order
    /// (the last one repeats), each a status line plus optional headers.
    /// It records the delivery id of every request and stores those it
    /// answered 2xx, as a Poet does.
    struct Intake {
        url: String,
        sent: Arc<std::sync::Mutex<Vec<String>>>,
        stored: Arc<std::sync::Mutex<std::collections::BTreeSet<String>>>,
    }

    async fn intake(replies: &'static [&'static str]) -> Intake {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let sent = Arc::new(std::sync::Mutex::new(Vec::new()));
        let stored = Arc::new(std::sync::Mutex::new(std::collections::BTreeSet::new()));
        let (seen, kept) = (sent.clone(), stored.clone());
        tokio::spawn(async move {
            for turn in 0.. {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                // Read the head, then the body its Content-Length names.
                let mut data = Vec::new();
                let mut chunk = vec![0_u8; 64 * 1024];
                let body = loop {
                    let read = socket.read(&mut chunk).await.unwrap_or(0);
                    data.extend_from_slice(&chunk[..read]);
                    let text = String::from_utf8_lossy(&data).to_string();
                    if let Some((head, body)) = text.split_once("\r\n\r\n") {
                        let length = head
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length: ")
                                    .and_then(|value| value.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if body.len() >= length || read == 0 {
                            break body.to_owned();
                        }
                    } else if read == 0 {
                        break String::new();
                    }
                };
                let delivery: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                let delivery = delivery["delivery_id"].as_str().unwrap_or("").to_owned();
                let reply = replies[turn.min(replies.len() - 1)];
                seen.lock().unwrap().push(delivery.clone());
                if reply.starts_with('2') {
                    kept.lock().unwrap().insert(delivery);
                }
                let reply =
                    format!("HTTP/1.1 {reply}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                let _ = socket.write_all(reply.as_bytes()).await;
            }
        });
        Intake {
            url: format!("http://{address}"),
            sent,
            stored,
        }
    }

    #[tokio::test]
    async fn too_many_requests_waits_retry_after_and_resends_the_same_delivery() {
        let poet = intake(&["429 Too Many Requests\r\nRetry-After: 2", "201 Created"]).await;
        let client = GraphPoetClient::new(poet.url.clone(), "token").unwrap();
        let started = std::time::Instant::now();
        client.publish(&request()).await.unwrap();
        let elapsed = started.elapsed();
        assert!(elapsed >= Duration::from_secs(2), "waited {elapsed:?}");
        assert!(elapsed < Duration::from_secs(5), "waited {elapsed:?}");
        assert_eq!(
            *poet.sent.lock().unwrap(),
            ["muse-test-1", "muse-test-1"],
            "sent twice, with the same delivery id"
        );
        assert_eq!(poet.stored.lock().unwrap().len(), 1, "stored once");
    }

    #[tokio::test]
    async fn unavailable_with_retry_after_waits_on_the_same_poet_and_ok_is_success() {
        let busy = intake(&["503 Service Unavailable\r\nRetry-After: 1", "200 OK"]).await;
        let other = intake(&["201 Created"]).await;
        let client =
            GraphPoetClient::cluster(vec![busy.url.clone(), other.url.clone()], "token").unwrap();
        let started = std::time::Instant::now();
        client.publish(&request()).await.unwrap();
        assert!(started.elapsed() >= Duration::from_secs(1));
        assert_eq!(busy.sent.lock().unwrap().len(), 2);
        assert!(other.sent.lock().unwrap().is_empty(), "not a failover");
        assert_eq!(client.preferred_endpoint(), busy.url);
    }

    #[tokio::test]
    async fn a_long_retry_after_is_clamped_to_the_cap() {
        let poet = intake(&["429 Too Many Requests\r\nRetry-After: 3600", "201 Created"]).await;
        let client = GraphPoetClient::new(poet.url.clone(), "token")
            .unwrap()
            .with_retry_after_cap(Duration::from_secs(1));
        let started = std::time::Instant::now();
        client.publish(&request()).await.unwrap();
        let elapsed = started.elapsed();
        assert!(elapsed >= Duration::from_secs(1), "waited {elapsed:?}");
        assert!(
            elapsed < Duration::from_secs(5),
            "clamped: waited {elapsed:?}"
        );
        assert_eq!(poet.stored.lock().unwrap().len(), 1);
    }

    #[test]
    fn retry_after_is_read_in_seconds_and_clamped() {
        use reqwest::header::{HeaderMap, HeaderValue, RETRY_AFTER};
        let wait = |value: Option<&'static str>| {
            let mut headers = HeaderMap::new();
            if let Some(value) = value {
                headers.insert(RETRY_AFTER, HeaderValue::from_static(value));
            }
            GraphPoetClient::retry_after(&headers, GraphPoetClient::RETRY_AFTER_CAP)
        };
        assert_eq!(wait(Some("2")), Duration::from_secs(2));
        assert_eq!(wait(Some("0")), Duration::ZERO);
        assert_eq!(
            wait(Some("3600")),
            Duration::from_secs(60),
            "clamped to 60 s"
        );
        assert_eq!(wait(None), Duration::from_secs(1));
        assert_eq!(
            wait(Some("Wed, 21 Oct 2026 07:28:00 GMT")),
            Duration::from_secs(1),
            "an HTTP date waits the default"
        );
    }

    #[tokio::test]
    async fn a_poet_busy_beyond_the_cap_ends_in_backpressure_not_a_rejection() {
        let poet = intake(&["429 Too Many Requests\r\nRetry-After: 1"]).await;
        let client = GraphPoetClient::new(poet.url.clone(), "token")
            .unwrap()
            .with_retry_after_cap(Duration::from_secs(1));
        let result = client.publish(&request()).await;
        assert!(
            matches!(result, Err(MuseError::Backpressure(_))),
            "{result:?}"
        );
        assert_eq!(
            poet.sent.lock().unwrap().len(),
            2,
            "one wait, then given back"
        );
        assert!(poet.stored.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_bad_request_is_still_a_rejection() {
        let poet = intake(&["400 Bad Request\r\nRetry-After: 1"]).await;
        let client = GraphPoetClient::new(poet.url.clone(), "token").unwrap();
        assert!(matches!(
            client.publish(&request()).await,
            Err(MuseError::Validation(_))
        ));
        assert_eq!(poet.sent.lock().unwrap().len(), 1, "not retried");
    }

    #[tokio::test]
    async fn otlp_logs_go_to_v1_logs_as_protobuf_and_wait_out_retry_after() {
        let poet = intake(&["429 Too Many Requests\r\nRetry-After: 1", "200 OK"]).await;
        let client = GraphPoetClient::new(poet.url.clone(), "token").unwrap();
        let started = std::time::Instant::now();
        client.publish_otlp_logs(b"\x0a\x00").await.unwrap();
        assert!(started.elapsed() >= Duration::from_secs(1));
        assert_eq!(
            poet.sent.lock().unwrap().len(),
            2,
            "the same export, resent once"
        );
        let (up, heads) = recording_server("200 OK").await;
        let client = GraphPoetClient::new(up, "token").unwrap();
        client.publish_otlp_logs(b"\x0a\x00").await.unwrap();
        let head = heads.lock().unwrap()[0].clone();
        assert!(head.starts_with("post /v1/logs "), "{head}");
        assert_eq!(
            header(&head, "content-type"),
            Some("application/x-protobuf")
        );
        assert_eq!(header(&head, "authorization"), Some("bearer token"));
    }

    #[tokio::test]
    async fn a_refused_token_keeps_the_export_and_a_refused_export_does_not() {
        for (status, kept) in [
            ("401 Unauthorized", true),
            ("403 Forbidden", true),
            ("400 Bad Request", false),
            ("413 Payload Too Large", false),
        ] {
            let (poet, _) = server(status).await;
            let client = GraphPoetClient::new(poet, "token").unwrap();
            let result = client.publish_otlp_logs(b"").await;
            match result {
                Err(MuseError::Client(_)) => assert!(kept, "{status}"),
                Err(MuseError::Validation(_)) => assert!(!kept, "{status}"),
                other => panic!("{status}: {other:?}"),
            }
        }
    }
}
