// crates/ih-muse-core/src/errors.rs

use std::time::Duration;

use thiserror::Error;

pub type MuseResult<T> = Result<T, MuseError>;

#[derive(Error, Debug)]
pub enum MuseError {
    #[error("Configuration error {0}")]
    Configuration(String),
    #[error("Network error occurred")]
    Client(String),
    #[error("Muse initialization timeout {0:?}")]
    MuseInitializationTimeout(Duration),
    #[error("Recording error {0}")]
    Recording(String),
    #[error("Replaying error {0}")]
    Replaying(String),
    #[error("File has an invalid extension {0:?}")]
    InvalidFileExtension(Option<String>),
    #[error("Invalid Element Kind Code {0}")]
    InvalidElementKindCode(String),
    #[error("Invalid Metric Code {0}")]
    InvalidMetricCode(String),
}
