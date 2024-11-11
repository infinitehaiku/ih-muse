// crates/ih-muse/tests/it/test_initialization.rs

use std::collections::HashMap;

use tokio::time::{sleep, Duration};

use ih_muse::{ClientType, Config, Muse};
use ih_muse_proto::{ElementKindRegistration, MetricDefinition, TimestampResolution};

#[tokio::test]
async fn test_muse_initialization_with_poet() {
    // Step 1: Define configuration for the Muse instance
    let config = Config {
        endpoints: vec!["http://localhost:8000".to_string()],
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
    };

    // Step 2: Initialize Muse instance
    let muse = Muse::new(config);

    // Step 3: Wait for initialization to complete (or timeout if something goes wrong)
    let max_wait_time = Duration::from_secs(10); // Adjust as needed
    let start_time = tokio::time::Instant::now();
    while !muse.is_initialized() && start_time.elapsed() < max_wait_time {
        sleep(Duration::from_millis(100)).await;
    }

    // Step 4: Assert that initialization completed successfully
    assert!(
        muse.is_initialized(),
        "Muse did not initialize within the expected time"
    );

    // Additional assertions can be added here, e.g., check that metrics can be sent:
    let element_id = muse
        .register_element("server", "TestServer".to_string(), HashMap::new(), None)
        .await
        .expect("Failed to register element");

    let send_metric_result = muse.send_metric(element_id, "cpu_usage", 50.0).await;
    assert!(
        send_metric_result.is_ok(),
        "Failed to send metric after initialization"
    );

    // Step 5: Cleanup (when Muse is dropped, it should cancel tasks automatically)
}
