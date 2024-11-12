use std::sync::Arc;
use tokio::select;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

use super::calculate_interval_duration;
use ih_muse_core::{time, Error, MetricBuffer, State, Transport};
use ih_muse_proto::{MetricPayload, Timestamp};

pub async fn start_metric_sender_task(
    cancellation_token: CancellationToken,
    client: Arc<dyn Transport + Send + Sync>,
    state: Arc<State>,
    metric_buffer: Arc<MetricBuffer>,
) {
    loop {
        let interval = calculate_interval_duration(state.get_finest_resolution(), 1);
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

async fn send_metrics(
    client: &Arc<dyn Transport + Send + Sync>,
    state: &Arc<State>,
    buffer: &Arc<MetricBuffer>,
) -> Result<(), Error> {
    let buffered_metrics = buffer.get_and_clear().await;
    if buffered_metrics.is_empty() {
        log::debug!("No metrics to send. Exiting.");
        return Ok(());
    }
    log::debug!("Processing metrics for {} elements", buffered_metrics.len());
    let metric_order = state.get_metric_order();
    let mut payloads = Vec::new();
    let timestamp = time::utc_now_i64();
    for (local_elem_id, metrics) in buffered_metrics {
        if let Some(element_id) = state.get_element_id(&local_elem_id) {
            let mut metric_ids = Vec::new();
            let mut values = Vec::new();
            for metric_def in &*metric_order {
                metric_ids.push(metric_def.id);
                if let Some(value) = metrics.get(&metric_def.code) {
                    values.push(Some(value.clone()));
                } else {
                    values.push(None);
                }
            }
            let payload = MetricPayload {
                time: timestamp,
                element_id,
                metric_ids,
                values,
            };
            payloads.push(payload);
        } else {
            log::warn!(
                "Skipping metrics for not registered Element {:?}.",
                local_elem_id
            );
        }
    }
    if payloads.is_empty() {
        log::debug!("No metrics to send after filtering unregistered elements.");
        return Ok(());
    }
    log::debug!("Sending {} metric payloads.", payloads.len());
    client.send_metrics(payloads).await?;
    Ok(())
}
