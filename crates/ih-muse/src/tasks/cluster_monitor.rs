// crates/ih-muse/src/tasks/cluster_monitor.rs

pub async fn start_cluster_monitor(
    cancellation_token: CancellationToken,
    client: Arc<dyn Transport + Send + Sync>,
    state: Arc<State>,
    is_initialized: Arc<AtomicBool>,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(60));

    loop {
        select! {
            _ = cancellation_token.cancelled() => {
                eprintln!("Cluster monitor task was cancelled.");
                break;
            }
            _ = interval.tick() => {
                if is_initialized.load(Ordering::SeqCst) {
                    // Perform cluster monitoring actions
                    if let Err(e) = client.get_node_state().await {
                        eprintln!("Error fetching node state: {:?}", e);
                    } else {
                        // Update state with new node information
                        // ...
                    }
                }
            }
        }
    }
}
