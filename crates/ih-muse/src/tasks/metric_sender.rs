use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::select;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

use crate::timing::metric_sending_interval;
use ih_muse_core::{time, MetricBuffer, MuseResult, State, Transport};
use ih_muse_proto::{LocalElementId, MetricPayload, MetricValue};

pub async fn start_metric_sender_task(
    cancellation_token: CancellationToken,
    client: Arc<dyn Transport + Send + Sync>,
    state: Arc<State>,
    metric_buffer: Arc<MetricBuffer>,
) {
    loop {
        let interval = metric_sending_interval(state.get_finest_resolution());
        select! {
            _ = cancellation_token.cancelled() => {
                println!("Metric sender task was cancelled.");
                break;
            }
            _ = sleep(interval) => {
                if let Err(e) = send_metrics(
                    &client,
                    &state,
                    &metric_buffer,
                ).await {
                    eprintln!("Error during metric sending: {:?}", e);
                }
            }
        }
    }
}

/// Values taken from the buffer for one send, keyed by element.
type Snapshot = HashMap<LocalElementId, HashMap<String, MetricValue>>;

/// Sends every buffered value now. Values that could not be delivered, or whose
/// element is not registered yet, go back into the buffer for the next attempt.
pub(crate) async fn send_metrics(
    client: &Arc<dyn Transport + Send + Sync>,
    state: &Arc<State>,
    buffer: &Arc<MetricBuffer>,
) -> MuseResult<()> {
    let buffered_metrics = buffer.get_and_clear().await;
    if buffered_metrics.is_empty() {
        log::debug!("No metrics to send. Exiting.");
        return Ok(());
    }
    log::debug!("Processing metrics for {} elements", buffered_metrics.len());
    let metric_order = state.get_metric_order();
    let timestamp = time::utc_now_i64();
    let mut metrics_per_node: HashMap<Option<SocketAddr>, (Vec<MetricPayload>, Snapshot)> =
        HashMap::new();
    let mut unsent: Snapshot = HashMap::new();
    for (local_elem_id, metrics) in buffered_metrics {
        if let Some(element_id) = state.get_element_id(&local_elem_id) {
            let node_addr = state.find_element_node_addr(element_id);
            let metric_ids = metric_order.iter().map(|def| def.id).collect();
            let values = metric_order
                .iter()
                .map(|def| metrics.get(&def.code).cloned())
                .collect();

            let payload = MetricPayload {
                time: timestamp,
                element_id,
                metric_ids,
                values,
            };
            let (payloads, taken) = metrics_per_node.entry(node_addr).or_default();
            payloads.push(payload);
            taken.insert(local_elem_id, metrics);
        } else {
            log::debug!(
                "Keeping metrics for Element {:?} until it is registered.",
                local_elem_id
            );
            unsent.insert(local_elem_id, metrics);
        }
    }
    let mut first_error = None;
    for (node_addr, (payloads, taken)) in metrics_per_node {
        log::debug!(
            "Sending {} metrics to node {:?}.",
            payloads.len(),
            node_addr
        );
        if let Err(error) = client.send_metrics(payloads, node_addr).await {
            unsent.extend(taken);
            first_error.get_or_insert(error);
        }
    }
    let dropped = buffer.restore(unsent).await;
    if dropped > 0 {
        log::warn!("Metric buffer full: {dropped} undelivered values were dropped");
    }
    first_error.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use async_trait::async_trait;
    use ih_muse_client::MockClient;
    use ih_muse_core::{MuseError, Transport};
    use ih_muse_proto::{
        ElementKindRegistration, ElementRegistration, MetricDefinition, MetricQuery,
        NodeElementRange, NodeState, TimestampResolution,
    };
    use uuid::Uuid;

    use super::*;

    /// A transport whose metric sends fail while `failing` is set.
    struct FlakyClient {
        inner: MockClient,
        failing: AtomicBool,
        sent: std::sync::Mutex<Vec<MetricPayload>>,
    }

    #[async_trait]
    impl Transport for FlakyClient {
        async fn health_check(&self) -> MuseResult<()> {
            self.inner.health_check().await
        }
        async fn get_node_state(&self) -> MuseResult<NodeState> {
            self.inner.get_node_state().await
        }
        async fn get_finest_resolution(&self) -> MuseResult<TimestampResolution> {
            self.inner.get_finest_resolution().await
        }
        async fn register_element_kinds(&self, kinds: &[ElementKindRegistration]) -> MuseResult<()> {
            self.inner.register_element_kinds(kinds).await
        }
        async fn register_elements(
            &self,
            elements: &[ElementRegistration],
        ) -> MuseResult<Vec<MuseResult<u64>>> {
            self.inner.register_elements(elements).await
        }
        async fn get_node_elem_ranges(
            &self,
            ini: Option<u64>,
            end: Option<u64>,
        ) -> MuseResult<Vec<NodeElementRange>> {
            self.inner.get_node_elem_ranges(ini, end).await
        }
        async fn register_metrics(&self, payload: &[MetricDefinition]) -> MuseResult<()> {
            self.inner.register_metrics(payload).await
        }
        async fn get_metric_order(&self) -> MuseResult<Vec<MetricDefinition>> {
            self.inner.get_metric_order().await
        }
        async fn get_metrics(
            &self,
            query: &MetricQuery,
            node_addr: Option<SocketAddr>,
        ) -> MuseResult<Vec<MetricPayload>> {
            self.inner.get_metrics(query, node_addr).await
        }
        async fn send_metrics(
            &self,
            payload: Vec<MetricPayload>,
            _node_addr: Option<SocketAddr>,
        ) -> MuseResult<()> {
            if self.failing.load(Ordering::SeqCst) {
                return Err(MuseError::Unavailable("poet restarting".into()));
            }
            self.sent.lock().unwrap().extend(payload);
            Ok(())
        }
    }

    #[tokio::test]
    async fn a_failed_send_keeps_values_until_a_later_send_delivers_them() {
        let flaky = Arc::new(FlakyClient {
            inner: MockClient::new(TimestampResolution::Seconds),
            failing: AtomicBool::new(true),
            sent: std::sync::Mutex::new(Vec::new()),
        });
        let client: Arc<dyn Transport + Send + Sync> = flaky.clone();
        let state = Arc::new(State::new(TimestampResolution::Seconds));
        state
            .update_metric_order(vec![MetricDefinition::new("bytes", "Bytes", "Used bytes")])
            .await;
        let registered = Uuid::new_v4();
        let pending = Uuid::new_v4();
        state.update_element_id(registered, 7).await;
        let buffer = Arc::new(MetricBuffer::new());
        buffer.add_metric(registered, "bytes".into(), 16_777_217.0).await.unwrap();
        buffer.add_metric(pending, "bytes".into(), 1.0).await.unwrap();

        assert!(send_metrics(&client, &state, &buffer).await.is_err());
        assert_eq!(buffer.len().await, 2, "nothing is lost when Poet is down");

        flaky.failing.store(false, Ordering::SeqCst);
        send_metrics(&client, &state, &buffer).await.unwrap();
        let sent = flaky.sent.lock().unwrap().clone();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].values, vec![Some(16_777_217.0)]);
        assert_eq!(buffer.len().await, 1, "an unregistered element waits for its id");
    }
}
