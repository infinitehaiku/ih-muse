// tests/it/test_init.rs
use super::common::{client_type_from_env, TestContext, DEFAULT_WAIT_TIME};
use ih_muse::{ClientType, Config, Muse};
use ih_muse_proto::{ElementKindRegistration, MetricDefinition, TimestampResolution};
use std::time::Duration;

#[tokio::test]
async fn test_muse_initialization_with_poet() {
    let ctx = TestContext::new(None).await;

    // Test basic initialization succeeded
    assert!(
        ctx.muse.is_initialized(),
        "{}",
        format!(
            "Muse failed to initialize with {:?} client",
            ctx.config.client_type
        )
    );

    // Verify we can perform operations after initialization
    let local_elem_id = ctx.register_test_element().await;
    let send_result = ctx.muse.send_metric(local_elem_id, "cpu_usage", 50.0).await;
    assert!(
        send_result.is_ok(),
        "Failed to send metric after initialization"
    );
}

#[tokio::test]
async fn test_muse_initialization_with_mock() {
    let ctx = TestContext::new(Some(ClientType::Mock)).await;
    assert!(
        ctx.muse.is_initialized(),
        "Muse failed to initialize with Mock client"
    );
}

#[tokio::test]
async fn test_muse_initialization_with_custom_config() {
    // Custom configuration with different settings
    let config = Config {
        endpoints: vec!["http://localhost:8000".to_string()],
        client_type: client_type_from_env(),
        recording_enabled: true,
        recording_path: Some("/tmp/muse_test.json".into()),
        default_resolution: TimestampResolution::Milliseconds,
        element_kinds: vec![
            ElementKindRegistration::new("server", None, "Server", "Server element"),
            ElementKindRegistration::new("database", None, "Database", "Database element"),
        ],
        metric_definitions: vec![
            MetricDefinition::new("cpu_usage", "CPU Usage", "CPU usage metric"),
            MetricDefinition::new("memory_usage", "Memory Usage", "Memory usage metric"),
        ],
        cluster_monitor_interval: Some(Duration::from_secs(1)),
        max_reg_elem_retries: 5,
    };

    let mut muse = Muse::new(&config).expect("Failed to create the Muse");
    TestContext::wait_for_init(&mut muse).await;

    assert!(
        muse.is_initialized(),
        "Muse failed to initialize with custom config"
    );

    // Verify custom configuration was applied
    let state = muse.get_state();

    // Test element kind registration
    let local_elem_id = muse
        .register_element("database", "TestDB".to_string(), Default::default(), None)
        .await
        .expect("Failed to register database element");

    // Wait for element registration
    let start_time = tokio::time::Instant::now();
    while state.get_element_id(&local_elem_id).is_none() && start_time.elapsed() < DEFAULT_WAIT_TIME
    {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    assert!(
        state.get_element_id(&local_elem_id).is_some(),
        "Failed to register element with custom element kind"
    );
}

#[tokio::test]
async fn test_muse_initialization_timeout() {
    let client_type = client_type_from_env();
    if client_type == ClientType::Mock {
        println!("Skipping, only makes sense with Poet client");
        return;
    }
    // Create config with unreachable endpoint to test timeout
    let config = Config {
        endpoints: vec!["http://unreachable:9999".to_string()],
        client_type,
        recording_enabled: false,
        recording_path: None,
        default_resolution: TimestampResolution::Seconds,
        element_kinds: vec![],
        metric_definitions: vec![],
        cluster_monitor_interval: None,
        max_reg_elem_retries: 1, // Set low retry count for faster test
    };

    let mut muse = Muse::new(&config).expect("Failed to create the Muse");
    // Wait for a short time to see if initialization fails as expected
    let ini_result = muse.initialize(Some(Duration::from_secs(2))).await;
    assert!(ini_result.is_err(), "Expect initialization timaout");
    assert!(
        !muse.is_initialized(),
        "Muse should not initialize with unreachable endpoint"
    );
}

#[tokio::test]
async fn test_muse_reinitialization() {
    let ctx = TestContext::new(None).await;
    assert!(ctx.muse.is_initialized(), "Initial initialization failed");

    // Force reinitialization (if your Muse implementation supports this)
    // This is a placeholder - implement according to your Muse API
    // ctx.muse.reinitialize().await;

    // Verify system is still functional
    let local_elem_id = ctx.register_test_element().await;
    let send_result = ctx.muse.send_metric(local_elem_id, "cpu_usage", 50.0).await;
    assert!(
        send_result.is_ok(),
        "Failed to send metric after reinitialization"
    );
}