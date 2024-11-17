// crates/ih-muse-record/src/file_replayer.rs

//! Provides the [`FileReplayer`] for replaying recorded events.
//!
//! The recording files should have a specific extension to determine
//! the encoding/serialization format. Supported extensions are:
//!
//! - `.bin` for Bincode serialization
//! - `.json` for JSON serialization

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use async_trait::async_trait;

use super::SerializationFormat;
use crate::{RecordedEvent, Replayer};
use ih_muse_core::{MuseError, MuseResult};

/// A replayer that reads events from a recording file.
///
/// The serialization format is determined by the file extension.
/// Supported extensions are `.bin` for Bincode and `.json` for JSON.
pub struct FileReplayer {
    reader: BufReader<File>,
    format: SerializationFormat,
}

impl FileReplayer {
    /// Creates a new `FileReplayer`.
    ///
    /// # Arguments
    ///
    /// - `path`: The file path to read recordings from.
    ///
    /// # Errors
    ///
    /// Returns a [`MuseError::Replaying`] if the file cannot be opened.
    pub fn new(path: &Path) -> MuseResult<Self> {
        let ext = path.extension().and_then(|e| e.to_str());
        let format = SerializationFormat::from_extension(ext)?;

        let file = File::open(path)
            .map_err(|e| MuseError::Replaying(format!("Failed to open file: {}", e)))?;
        log::info!("Using {:?} format for replaying.", format);
        Ok(Self {
            reader: BufReader::new(file),
            format,
        })
    }
}

#[async_trait]
impl Replayer for FileReplayer {
    /// Reads the next event from the recording file.
    ///
    /// # Returns
    ///
    /// An `Option<RecordedEvent>` representing the next event, or `None` if EOF is reached.
    ///
    /// # Errors
    ///
    /// Returns a [`MuseError::Replaying`] if deserialization fails.
    async fn next_event(&mut self) -> MuseResult<Option<RecordedEvent>> {
        match self.format {
            SerializationFormat::Bincode => {
                match bincode::deserialize_from::<_, RecordedEvent>(&mut self.reader) {
                    Ok(event) => Ok(Some(event)),
                    Err(e) => {
                        if let bincode::ErrorKind::Io(ref io_error) = *e {
                            if io_error.kind() == std::io::ErrorKind::UnexpectedEof {
                                return Ok(None);
                            }
                        }
                        Err(MuseError::Replaying(format!(
                            "Failed to deserialize event: {}",
                            e
                        )))
                    }
                }
            }
            SerializationFormat::Json => {
                let mut line = String::new();
                match self.reader.read_line(&mut line) {
                    Ok(0) => Ok(None), // EOF
                    Ok(_) => {
                        let event: RecordedEvent = serde_json::from_str(&line).map_err(|e| {
                            MuseError::Replaying(format!("Failed to deserialize event: {}", e))
                        })?;
                        Ok(Some(event))
                    }
                    Err(e) => Err(MuseError::Replaying(format!("Failed to read line: {}", e))),
                }
            }
        }
    }
}
