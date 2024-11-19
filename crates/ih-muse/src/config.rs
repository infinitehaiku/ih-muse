// crates/ih-muse/src/config.rs

use ih_muse_core::{MuseError, MuseResult};
use ih_muse_proto::{ElementKindRegistration, MetricDefinition, TimestampResolution};
use tokio::time::Duration;

/// Specifies the type of client to use with the Muse system.
///
/// - `Poet`: Communicates with the Poet service.
/// - `Mock`: Uses a mock client for testing purposes.
#[derive(Clone, PartialEq, Debug)]
pub enum ClientType {
    /// Communicates with the Poet service.
    Poet,
    /// Uses a mock client for testing.
    Mock,
}

/// Configuration settings for the Muse client.
///
/// The `Config` struct contains all necessary parameters to initialize the Muse client.
#[derive(Clone)]
pub struct Config {
    /// List of endpoint URLs for the Muse client.
    pub endpoints: Vec<String>,
    /// The type of client to use (`Poet` or `Mock`).
    pub client_type: ClientType,
    /// Enables event recording if set to `true`.
    pub recording_enabled: bool,
    /// File path for recording events (required if `recording_enabled` is `true`).
    pub recording_path: Option<String>,
    /// Default timestamp resolution for metrics.
    pub default_resolution: TimestampResolution,
    /// List of element kinds to register upon initialization.
    pub element_kinds: Vec<ElementKindRegistration>,
    /// List of metric definitions available for reporting.
    pub metric_definitions: Vec<MetricDefinition>,
    /// Interval for cluster monitoring tasks (optional).
    pub cluster_monitor_interval: Option<Duration>,
    /// Maximum number of retries for element registration.
    pub max_reg_elem_retries: usize,
}

impl Config {
    /// Creates a new `Config` instance with the provided settings.
    ///
    /// # Arguments
    ///
    /// - `endpoints`: A vector of endpoint URLs.
    /// - `client_type`: The client type to use.
    /// - `recording_enabled`: Enables event recording.
    /// - `recording_path`: File path for recording events.
    /// - `default_resolution`: Default timestamp resolution.
    /// - `element_kinds`: Element kinds to register.
    /// - `metric_definitions`: Metric definitions for reporting.
    /// - `cluster_monitor_interval`: Interval for cluster monitoring.
    /// - `max_reg_elem_retries`: Max retries for element registration.
    ///
    /// # Errors
    ///
    /// Returns a [`MuseError::Configuration`] if validation fails.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use ih_muse::prelude::*;
    ///
    /// let config = Config::new(
    ///     vec!["http://localhost:8080".to_string()],
    ///     ClientType::Poet,
    ///     false,
    ///     None,
    ///     TimestampResolution::Milliseconds,
    ///     vec![ElementKindRegistration::new("kind_code", Some("parent_code"), "kind_name", "description")],
    ///     vec![MetricDefinition::new("metric_code", "metric_name", "description")],
    ///     Some(std::time::Duration::from_secs(60)),
    ///     3,
    /// ).expect("Failed to create config");
    /// ```
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

    /// Validates the configuration settings.
    ///
    /// Ensures all required fields are properly set.
    ///
    /// # Errors
    ///
    /// Returns a [`MuseError::Configuration`] if any validation check fails.
    pub fn validate(&self) -> MuseResult<()> {
        if self.client_type == ClientType::Poet && self.endpoints.is_empty() {
            return Err(MuseError::Configuration(
                "At least one endpoint needs to be specified for Poet client.".to_string(),
            ));
        }
        if self.element_kinds.is_empty() {
            return Err(MuseError::Configuration(
                "Element kinds cannot be empty.".to_string(),
            ));
        }
        if self.metric_definitions.is_empty() {
            return Err(MuseError::Configuration(
                "Metric definitions cannot be empty.".to_string(),
            ));
        }
        if self.recording_enabled && self.recording_path.is_none() {
            return Err(MuseError::Configuration(
                "Recording enabled without a file path.".to_string(),
            ));
        }
        Ok(())
    }
}
