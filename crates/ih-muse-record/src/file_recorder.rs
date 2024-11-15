// crates/ih-muse-core/src/recording/file_recording.rs

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;

use async_trait::async_trait;

use super::SerializationFormat;
use crate::{RecordedEvent, Recorder};
use ih_muse_core::{MuseError, MuseResult};

pub struct FileRecorder {
    writer: BufWriter<File>,
    format: SerializationFormat,
}

impl FileRecorder {
    pub fn new(path: &Path) -> MuseResult<Self> {
        let ext = path.extension().and_then(|e| e.to_str());
        let format = SerializationFormat::from_extension(ext)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)
            .map_err(|e| MuseError::Recording(format!("Failed to open file: {}", e)))?;
        log::info!("Using {:?} format for recording.", format);
        Ok(Self {
            writer: BufWriter::new(file),
            format,
        })
    }
}

#[async_trait]
impl Recorder for FileRecorder {
    async fn record(&mut self, event: RecordedEvent) -> MuseResult<()> {
        match self.format {
            SerializationFormat::Bincode => bincode::serialize_into(&mut self.writer, &event)
                .map_err(|e| MuseError::Recording(format!("Failed to record event: {}", e))),
            SerializationFormat::Json => {
                serde_json::to_writer(&mut self.writer, &event)
                    .map_err(|e| MuseError::Recording(format!("Failed to record event: {}", e)))?;
                self.writer
                    .write_all(b"\n")
                    .map_err(|e| MuseError::Recording(format!("Failed to write newline: {}", e)))
            }
        }
    }

    async fn close(&mut self) -> MuseResult<()> {
        self.writer
            .flush()
            .map_err(|e| MuseError::Recording(format!("Failed to close file: {}", e)))
    }
}
