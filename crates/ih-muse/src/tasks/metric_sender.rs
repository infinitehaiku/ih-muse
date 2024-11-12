fn start_metric_sender(&self) {
    let client = self.client.clone();
    let metrics_buffer = self.metrics_buffer.clone();
    let state = self.state.clone();

    tokio::spawn(async move {
        loop {
            let flush_interval = state.get_flush_interval().await;
            tokio::time::sleep(flush_interval).await;

            let metrics_to_send = {
                let mut buffer = metrics_buffer.lock().await;
                std::mem::take(&mut *buffer)
            };

            if !metrics_to_send.is_empty() {
                let payloads: Vec<MetricPayload> = metrics_to_send.into_values().collect();
                // Ensure metrics are ordered according to metric_order
                let ordered_payloads = state.order_metrics(payloads).await;
                if let Err(e) = client.send_metrics(ordered_payloads).await {
                    eprintln!("Error sending metrics: {:?}", e);
                }
            }
        }
    });
}
