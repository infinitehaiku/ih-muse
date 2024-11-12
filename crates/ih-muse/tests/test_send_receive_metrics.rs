// crates/ih-muse/tests/test_send_receive_metrics.rs

use std::collections::HashMap;

use tokio::time::{sleep, Duration};

use ih_muse::test_utils::init_logger;
use ih_muse::{ClientType, Config, Muse};
use ih_muse_client::PoetClient;
use ih_muse_core::Transport;
use ih_muse_proto::{ElementKindRegistration, MetricDefinition, MetricQuery, TimestampResolution};

#[tokio::test]
async fn test_send_and_receive_metric() {
    init_logger();

    // Endpoint for testing
    let test_endpoint = "http://localhost:8000".to_string(); // Replace with your test endpoint

    // Step 1: Define configuration for the Muse instance
    let config = Config {
        endpoints: vec![test_endpoint.clone()],
        client_type: ClientType::Poet,
        recording_enabled: false,
        recording_path: None,
        default_resolution: TimestampResolution::Seconds,
        element_kinds: vec![ElementKindRegistration::new(
            "server".to_string(),
            None,
            "Server".to_string(),
            "A server element kind".to_string(),
        )],
        metric_definitions: vec![MetricDefinition::new(
            "cpu_usage".to_string(),
            "CPU Usage".to_string(),
            "The CPU usage of a server".to_string(),
        )],
        cluster_monitor_interval: None,
        max_reg_elem_retries: 3,
    };

    // Step 2: Initialize Muse instance
    let muse = Muse::new(config);

    // Wait for Muse to be initialized
    let max_wait_time = Duration::from_secs(10);
    let start_time = tokio::time::Instant::now();
    while !muse.is_initialized() && start_time.elapsed() < max_wait_time {
        sleep(Duration::from_millis(100)).await;
    }
    assert!(
        muse.is_initialized(),
        "Muse did not initialize within the expected time"
    );

    let state = muse.get_state();

    // Step 3: Register an element
    let local_elem_id = muse
        .register_element("server", "TestServer".to_string(), HashMap::new(), None)
        .await
        .expect("Failed to register element");

    // Wait for the element registration task to process the buffer and update the state
    let max_wait_time = Duration::from_secs(5);
    let start_time = tokio::time::Instant::now();
    while state.get_element_id(&local_elem_id).is_none() && start_time.elapsed() < max_wait_time {
        sleep(Duration::from_millis(100)).await;
    }

    // Verify the element was registered successfully in the state
    let element_id = state
        .get_element_id(&local_elem_id)
        .expect("Element was not registered within the expected time");

    // Step 4: Send a metric for the registered element
    muse.send_metric(local_elem_id, "cpu_usage", 42.0)
        .await
        .expect("Failed to send metric");

    // Wait for the metric sender task to process the buffer
    sleep(Duration::from_secs(2)).await; // Adjust as needed to ensure the metric is sent

    // Step 5: Retrieve the metrics using PoetClient
    // Instantiate PoetClient separately
    let poet_client = PoetClient::new(vec![test_endpoint]);

    let query = MetricQuery {
        start_time: None,
        end_time: None,
        element_id: Some(element_id),
        parent_id: None,
        metric_id: None,
    };

    let metrics = poet_client
        .get_metrics(&query, None)
        .await
        .expect("Failed to get metrics");

    // Step 6: Verify that the metrics retrieved match the ones sent
    assert!(!metrics.is_empty(), "No metrics retrieved for the element");

    let metric_found = metrics.iter().any(|metric| metric.element_id == element_id);

    assert!(metric_found, "Sent metric not found in retrieved metrics");
}
