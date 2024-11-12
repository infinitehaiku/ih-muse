use std::collections::HashMap;

use tokio::time::{sleep, Duration};

use ih_muse::test_utils::init_logger;
use ih_muse::{ClientType, Config, Muse};
use ih_muse_proto::{ElementKindRegistration, TimestampResolution};

#[tokio::test]
async fn test_cluster_monitor_updates_state() {
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
        metric_definitions: vec![], // Not needed for this test
        // Use a fast interval to get node information quickly
        cluster_monitor_interval: Some(Duration::from_millis(100)),
        max_reg_elem_retries: 3,
    };

    // Step 2: Initialize Muse instance
    let muse = Muse::new(config);

    // Wait for Muse to be initialized
    let max_wait_time = Duration::from_secs(5);
    let start_time = tokio::time::Instant::now();
    while !muse.is_initialized() && start_time.elapsed() < max_wait_time {
        sleep(Duration::from_millis(100)).await;
    }
    assert!(
        muse.is_initialized(),
        "Muse did not initialize within the expected time"
    );

    let state = muse.get_state();

    // Step 3: Register an element (this adds it to the buffer)
    let local_elem_id = muse
        .register_element("server", "TestServer".to_string(), HashMap::new(), None)
        .await
        .expect("Failed to register element");

    // Step 4: Wait for the element registration task to process the buffer and update the state
    let max_wait_time = Duration::from_secs(5);
    let start_time = tokio::time::Instant::now();
    while state.get_element_id(&local_elem_id).is_none() && start_time.elapsed() < max_wait_time {
        sleep(Duration::from_millis(100)).await;
    }

    // Step 5: Wait for the cluster_monitor task to run and update the state
    sleep(Duration::from_secs(2)).await; // Adjust as needed

    // Step 6: Verify that update_nodes worked
    let nodes = state.get_nodes().await;
    assert!(
        !nodes.is_empty(),
        "State nodes should not be empty after cluster monitoring"
    );
    println!("Nodes: {:?}", nodes);

    // Step 7: Verify that update_node_elem_ranges worked
    let ranges = state.get_node_elem_ranges().await;
    assert!(
        !ranges.is_empty(),
        "State ranges should not be empty after cluster monitoring"
    );
    println!("Node Element Ranges: {:?}", ranges);

    // Step 8: Verify that find_element_node_addr works
    // We'll test with an element ID within a plausible range
    let test_element_id = 1; // Adjust based on your system's element IDs
    let found_node_addr = state.find_element_node_addr(test_element_id);
    assert!(
        found_node_addr.is_some(),
        "Node address should be found for element ID {}",
        test_element_id
    );
    println!(
        "Found node address for element ID {}: {:?}",
        test_element_id,
        found_node_addr.unwrap()
    );
}
