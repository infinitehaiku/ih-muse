// tests/it/common.rs

use std::collections::HashMap;
use std::env;

use tokio::time::{sleep, Duration};

use crate::logger::init_logger;
use ih_muse::Muse;
use ih_muse_core::{ClientType, Config};
use ih_muse_proto::*;

pub const TEST_ENDPOINT: &str = "http://localhost:8000";
pub const DEFAULT_WAIT_TIME: Duration = Duration::from_secs(5);
pub const EXTENDED_WAIT_TIME: Duration = Duration::from_secs(10);

/// Fetch the client type from the `IH_MUSE_CLIENT_TYPE` environment variable.
pub fn client_type_from_env() -> ClientType {
    match env::var("IH_MUSE_CLIENT_TYPE")
        .unwrap_or_else(|_| "Mock".to_string())
        .to_lowercase()
        .as_str()
    {
        "poet" => ClientType::Poet,
        _ => ClientType::Mock, // Default to Mock
    }
}

pub struct TestContext {
    pub config: Config,
    pub muse: Muse,
}

pub fn default_config(client_type: Option<ClientType>) -> Config {
    Config {
        endpoints: vec![TEST_ENDPOINT.to_string()],
        client_type: client_type.unwrap_or_else(client_type_from_env),
        recording_enabled: false,
        recording_path: None,
        recording_flush_interval: None,
        default_resolution: TimestampResolution::Seconds,
        element_kinds: vec![ElementKindRegistration::new(
            "server",
            None,
            "Server",
            "A server element kind",
        )],
        metric_definitions: vec![MetricDefinition::new(
            "cpu_usage",
            "CPU Usage",
            "The CPU usage of a server",
        )],
        cluster_monitor_interval: Some(Duration::from_millis(100)),
        max_reg_elem_retries: 3,
    }
}

impl TestContext {
    pub async fn new_with_config(config: Config) -> Self {
        init_logger();
        let mut muse = Muse::new(&config).expect("Failed to create the Muse");
        Self::wait_for_init(&mut muse).await;
        Self { config, muse }
    }

    pub async fn new(client_type: Option<ClientType>) -> Self {
        TestContext::new_with_config(default_config(client_type)).await
    }

    pub async fn new_recording(client_type: Option<ClientType>, record_path: String) -> Self {
        let mut config = default_config(client_type);
        config.recording_enabled = true;
        config.recording_path = Some(record_path);
        config.recording_flush_interval = Some(Duration::from_millis(1));
        TestContext::new_with_config(config).await
    }

    pub async fn wait_for_init(muse: &mut Muse) {
        muse.initialize(Some(EXTENDED_WAIT_TIME))
            .await
            .expect("Muse Initialization failed");
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
