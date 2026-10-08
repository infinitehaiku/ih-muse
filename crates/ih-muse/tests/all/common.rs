// tests/it/common.rs

use std::collections::HashMap;
use std::env;

use std::future::Future;

use tokio::time::{sleep, Duration, Instant};

use crate::logger::init_logger;
use ih_muse::Muse;
use ih_muse_core::{ClientType, Config};
use ih_muse_proto::*;

pub const TEST_ENDPOINT: &str = "http://localhost:8000";

/// How long a test waits for a background task's effect before failing.
///
/// Generous so that a loaded machine never fails a correct Muse; a broken
/// one still fails, only later.
pub const WAIT_DEADLINE: Duration = Duration::from_secs(10);

/// Step between two checks of a condition a test waits for.
const POLL_STEP: Duration = Duration::from_millis(10);

/// Polls `condition` every [`POLL_STEP`] until it holds or [`WAIT_DEADLINE`]
/// passes; returns whether it held. Tests wait with this instead of fixed
/// sleeps, then assert the condition with their own message.
pub async fn eventually<F, Fut>(mut condition: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let deadline = Instant::now() + WAIT_DEADLINE;
    loop {
        if condition().await {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        sleep(POLL_STEP).await;
    }
}

/// Waits until `muse`'s transport holds at least `at_least` metrics for
/// `element_id` and returns them (or what is there at the deadline).
pub async fn wait_for_metrics(
    muse: &Muse,
    element_id: ElementId,
    at_least: usize,
) -> Vec<MetricPayload> {
    let query = MetricQuery {
        start_time: None,
        end_time: None,
        element_id: Some(element_id),
        parent_id: None,
        metric_id: None,
    };
    let client = muse.get_client();
    let deadline = Instant::now() + WAIT_DEADLINE;
    loop {
        let metrics = client
            .get_metrics(&query, None)
            .await
            .expect("Failed to get metrics");
        if metrics.len() >= at_least || Instant::now() >= deadline {
            return metrics;
        }
        sleep(POLL_STEP).await;
    }
}

/// Timeout for `Muse::initialize` in tests: initialization takes a few
/// init-task ticks, which a loaded machine can stretch far beyond the
/// interval itself.
pub const INIT_TIMEOUT: Duration = WAIT_DEADLINE;

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
        default_resolution: TimestampResolution::Milliseconds,
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
        initialization_interval: Some(Duration::from_millis(100)),
        cluster_monitor_interval: Some(Duration::from_millis(100)),
        max_reg_elem_retries: 3,
    }
}

impl TestContext {
    pub async fn new_with_config(config: Config) -> Self {
        init_logger();
        let mut muse = Muse::new(&config).expect("Failed to create the Muse");
        muse.initialize(Some(INIT_TIMEOUT))
            .await
            .expect("Initialization issues");
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

    /// Waits until the Muse's background tasks delivered at least
    /// `at_least` metrics for `element_id`, and returns what was delivered.
    ///
    /// Polls the transport instead of sleeping one send interval, which a
    /// loaded machine can miss. On timeout it returns what is there, so the
    /// caller's assertion reports the shortfall.
    pub async fn wait_for_metrics(
        &self,
        element_id: ElementId,
        at_least: usize,
    ) -> Vec<MetricPayload> {
        wait_for_metrics(&self.muse, element_id, at_least).await
    }

    /// Waits until the cluster monitor has seen the nodes, the element
    /// ranges and the node owning `element_id`.
    ///
    /// Returns whether all three appeared before the deadline; the caller
    /// asserts each one with its own message.
    pub async fn wait_for_cluster_monitoring(&self, element_id: ElementId) -> bool {
        let state = self.muse.get_state();
        eventually(|| {
            let state = state.clone();
            async move {
                !state.get_nodes().await.is_empty()
                    && !state.get_node_elem_ranges().await.is_empty()
                    && state.find_element_node_addr(element_id).is_some()
            }
        })
        .await
    }

    pub async fn register_test_element(&self) -> LocalElementId {
        let local_elem_id = self
            .muse
            .register_element("server", "TestServer".to_string(), HashMap::new(), None)
            .await
            .expect("Failed to register element");

        // Registration runs on the Muse's background task: wait for it by
        // polling, not for fixed intervals, which a loaded machine can miss.
        let state = self.muse.get_state();
        assert!(
            eventually(|| {
                let state = state.clone();
                async move { state.get_element_id(&local_elem_id).is_some() }
            })
            .await,
            "element not registered by the Muse's background task within {WAIT_DEADLINE:?}"
        );

        local_elem_id
    }
}
