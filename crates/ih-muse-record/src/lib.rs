// crates/ih-muse-record/src/lib.rs

mod file_format;
mod file_recorder;
mod file_replayer;

use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use file_format::SerializationFormat;
pub use file_recorder::FileRecorder;
pub use file_replayer::FileReplayer;
use ih_muse_core::Error;
use ih_muse_proto::types::*;

#[async_trait]
pub trait Recorder {
    async fn record(&mut self, event: RecordedEvent) -> Result<(), Error>;
    async fn close(&mut self) -> Result<(), Error>;
}

#[async_trait]
pub trait Replayer {
    async fn next_event(&mut self) -> Result<Option<RecordedEvent>, Error>;
}

#[derive(Debug, Serialize, Deserialize)]
pub enum RecordedEvent {
    ElementRegistration {
        kind_code: String,
        name: String,
        metadata: HashMap<String, String>,
        parent_id: Option<ElementId>,
    },
    SendMetric {
        element_id: ElementId,
        metric_code: String,
        value: MetricValue,
    },
    EndpointUpdate(Vec<String>),
}
