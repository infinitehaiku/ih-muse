use ih_muse_core::{MuseError, MuseResult};

#[derive(Debug, Clone, Copy)]
pub enum SerializationFormat {
    Bincode,
    Json,
}

impl SerializationFormat {
    pub fn from_extension(ext: Option<&str>) -> MuseResult<Self> {
        match ext.map(|s| s.to_lowercase()) {
            Some(ref s) if s == "bin" => Ok(SerializationFormat::Bincode),
            Some(ref s) if s == "json" => Ok(SerializationFormat::Json),
            other => Err(MuseError::InvalidFileExtension(other)),
        }
    }
}
