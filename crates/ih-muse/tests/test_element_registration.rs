use std::collections::HashMap;

use tokio::time::{sleep, Duration};

use ih_muse::test_utils::init_logger;
use ih_muse::{ClientType, Config, Muse};
use ih_muse_proto::{ElementKindRegistration, TimestampResolution};

#[tokio::test]
async fn test_element_registration_task() {
    init_logger();
    // Step 1: Define configuration for the Muse instance
    let config = Config {
        endpoints: vec!["http://localhost:8000".to_string()],
        client_type: ClientType::Mock,
        recording_enabled: false,
        recording_path: None,
        default_resolution: TimestampResolution::Seconds,
        element_kinds: vec![ElementKindRegistration::new(
            "server".to_string(),
            None,
            "Server".to_string(),
            "A server element kind".to_string(),
        )],
        metric_definitions: vec![],
        cluster_monitor_interval: None,
        max_reg_elem_retries: 3,
    };

    // Step 2: Initialize Muse instance
    let muse = Muse::new(config);
    let max_wait_time = Duration::from_secs(10); // Adjust as needed
    let start_time = tokio::time::Instant::now();
    while !muse.is_initialized() && start_time.elapsed() < max_wait_time {
        sleep(Duration::from_millis(100)).await;
    }

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

    // Step 5: Verify the element was registered successfully in the state
    let element_id = state
        .get_element_id(&local_elem_id)
        .expect("Element was not registered within the expected time");

    // Optional: Additional checks or cleanup
    println!(
        "Element with LocalElementId {:?} was registered with ElementId {:?}",
        local_elem_id, element_id
    );
}
