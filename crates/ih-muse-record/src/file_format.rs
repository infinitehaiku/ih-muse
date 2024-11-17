// crates/ih-muse-record/src/file_format.rs

//! Defines the serialization formats for recording files.
//!
//! The recording files should have a specific extension to determine
//! the encoding/serialization format. Supported extensions are:
//!
//! - `.bin` for Bincode serialization
//! - `.json` for JSON serialization
//!
//! The serialization format is used by the [`FileRecorder`] and [`FileReplayer`].
//!
//! # Example
//!
//! ```rust
//! use ih_muse_record::SerializationFormat;
//!
//! let format = SerializationFormat::from_extension(Some("bin")).unwrap();
//! assert_eq!(format, SerializationFormat::Bincode);
//! ```

use ih_muse_core::{MuseError, MuseResult};

/// Enum representing the supported serialization formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SerializationFormat {
    /// Bincode serialization format.
    Bincode,
    /// JSON serialization format.
    Json,
}

impl SerializationFormat {
    /// Determines the serialization format from a file extension.
    ///
    /// # Arguments
    ///
    /// - `ext`: The file extension (without the dot).
    ///
    /// # Returns
    ///
    /// A `SerializationFormat` corresponding to the file extension.
    ///
    /// # Errors
    ///
    /// Returns [`MuseError::InvalidFileExtension`] if the extension is not supported.
    pub fn from_extension(ext: Option<&str>) -> MuseResult<Self> {
        match ext.map(|s| s.to_lowercase()) {
            Some(ref s) if s == "bin" => Ok(SerializationFormat::Bincode),
            Some(ref s) if s == "json" => Ok(SerializationFormat::Json),
            other => Err(MuseError::InvalidFileExtension(other)),
        }
    }
}
