// tests/it/common.rs

use std::collections::HashMap;

use tokio::time::{sleep, Duration};

use crate::logger::init_logger;
use ih_muse::{ClientType, Config, Muse};
use ih_muse_proto::*;

pub const TEST_ENDPOINT: &str = "http://localhost:8000";
pub const DEFAULT_WAIT_TIME: Duration = Duration::from_secs(5);
pub const EXTENDED_WAIT_TIME: Duration = Duration::from_secs(10);

pub struct TestContext {
    pub muse: Muse,
    pub endpoint: String,
}

impl TestContext {
    pub async fn new(client_type: ClientType) -> Self {
        init_logger();

        let config = Config {
            endpoints: vec![TEST_ENDPOINT.to_string()],
            client_type,
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
            cluster_monitor_interval: Some(Duration::from_millis(100)),
            max_reg_elem_retries: 3,
        };

        let muse = Muse::new(config);
        Self::wait_for_init(&muse).await;

        Self {
            muse,
            endpoint: TEST_ENDPOINT.to_string(),
        }
    }

    pub async fn wait_for_init(muse: &Muse) {
        let start_time = tokio::time::Instant::now();
        while !muse.is_initialized() && start_time.elapsed() < EXTENDED_WAIT_TIME {
            sleep(Duration::from_millis(100)).await;
        }
        assert!(
            muse.is_initialized(),
            "Muse failed to initialize within timeout"
        );
    }

    pub async fn register_test_element(&self) -> LocalElementId {
        let local_elem_id = self
            .muse
            .register_element("server", "TestServer".to_string(), HashMap::new(), None)
            .await
            .expect("Failed to register element");

        let state = self.muse.get_state();
        let start_time = tokio::time::Instant::now();
        while state.get_element_id(&local_elem_id).is_none()
            && start_time.elapsed() < DEFAULT_WAIT_TIME
        {
            sleep(Duration::from_millis(100)).await;
        }

        local_elem_id
    }
}
