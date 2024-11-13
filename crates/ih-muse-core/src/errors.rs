// crates/ih-muse-core/src/errors.rs

use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Configuration error {0}")]
    ConfigurationError(String),
    #[error("Invalid data provided")]
    InvalidData,
    #[error("Network error occurred")]
    NetworkError,
    #[error("Cache error occurred")]
    CacheError,
    #[error("Client error {0}")]
    ClientError(String),
    #[error("Recording error {0}")]
    RecordingError(String),
    #[error("Replaying error {0}")]
    ReplayingError(String),
    #[error("File has an invalid extension {0:?}")]
    InvalidFileExtension(Option<String>),
    #[error("Invalid Element Kind Code {0}")]
    InvalidElementKindCode(String),
    #[error("Invalid Metric Code {0}")]
    InvalidMetricCode(String),
}
