// crates/ih-muse-core/src/recording/recording.rs

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use async_trait::async_trait;

use super::SerializationFormat;
use crate::{Error, RecordedEvent, Replayer};

pub struct FileReplayer {
    reader: BufReader<File>,
    format: SerializationFormat,
}

impl FileReplayer {
    pub fn new(path: &Path) -> Result<Self, Error> {
        let ext = path.extension().and_then(|e| e.to_str());
        let format = SerializationFormat::from_extension(ext)?;

        let file = File::open(path)
            .map_err(|e| Error::ReplayingError(format!("Failed to open file: {}", e)))?;
        log::info!("Using {:?} format for replaying.", format);
        Ok(Self {
            reader: BufReader::new(file),
            format,
        })
    }
}

#[async_trait]
impl Replayer for FileReplayer {
    async fn next_event(&mut self) -> Result<Option<RecordedEvent>, Error> {
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
                        Err(Error::ReplayingError(format!(
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
                            Error::ReplayingError(format!("Failed to deserialize event: {}", e))
                        })?;
                        Ok(Some(event))
                    }
                    Err(e) => Err(Error::ReplayingError(format!("Failed to read line: {}", e))),
                }
            }
        }
    }
}
