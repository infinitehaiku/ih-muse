// crates/ih-muse/src/config.rs

use ih_muse_core::{MuseError, MuseResult};
use tokio::time::Duration;

use ih_muse_proto::{ElementKindRegistration, MetricDefinition, TimestampResolution};

#[derive(Clone, PartialEq)]
pub enum ClientType {
    Poet,
    Mock,
}

#[derive(Clone)]
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

impl Config {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        endpoints: Vec<String>,
        client_type: ClientType,
        recording_enabled: bool,
        recording_path: Option<String>,
        default_resolution: TimestampResolution,
        element_kinds: Vec<ElementKindRegistration>,
        metric_definitions: Vec<MetricDefinition>,
        cluster_monitor_interval: Option<Duration>,
        max_reg_elem_retries: usize,
    ) -> MuseResult<Self> {
        let config = Self {
            endpoints,
            client_type,
            recording_enabled,
            recording_path,
            default_resolution,
            element_kinds,
            metric_definitions,
            cluster_monitor_interval,
            max_reg_elem_retries,
        };
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> MuseResult<()> {
        if self.client_type == ClientType::Poet && self.endpoints.is_empty() {
            return Err(MuseError::Configuration(
                "At least one endpoint needs to be specified for Poet client".to_string(),
            ));
        }
        if self.element_kinds.is_empty() {
            return Err(MuseError::Configuration(
                "Element kinds cannot be empty".to_string(),
            ));
        }
        if self.metric_definitions.is_empty() {
            return Err(MuseError::Configuration(
                "Metric definitions cannot be empty".to_string(),
            ));
        }
        if self.recording_enabled && self.recording_path.is_none() {
            return Err(MuseError::Configuration(
                "Recording enabled without a filepath".to_string(),
            ));
        }
        Ok(())
    }
}
