// crates/ih-muse/src/config.rs

use tokio::time::Duration;

use ih_muse_proto::{ElementKindRegistration, MetricDefinition, TimestampResolution};

pub enum ClientType {
    Poet,
    Mock,
}

pub struct Config {
    pub endpoints: Vec<String>,
    pub client_type: ClientType,
    pub recording_enabled: bool,
    pub recording_path: Option<String>,
    pub default_resolution: TimestampResolution,
    pub element_kinds: Vec<ElementKindRegistration>,
    pub metric_definitions: Vec<MetricDefinition>,
    pub cluster_monitor_interval: Option<Duration>,
    pub max_reg_elem_retries: usize,
}
