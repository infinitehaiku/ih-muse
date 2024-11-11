// crates/ih-muse/src/lib.rs

mod tasks;

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use ih_muse_client::{MockClient, PoetClient};
use ih_muse_core::{time, Error, State, Transport};
use ih_muse_proto::metric_id_from_code;
use ih_muse_proto::{
    types::*, ElementId, ElementKindRegistration, ElementRegistration, MetricDefinition,
    MetricPayload, TimestampResolution,
};
use ih_muse_record::{FileRecorder, FileReplayer, RecordedEvent, Recorder, Replayer};

pub enum ClientType {
    Poet,
    Mock,
}

pub struct Config {
    pub endpoints: Vec<String>,
    pub client_type: ClientType,
    pub recording_enabled: bool,
    pub recording_path: Option<String>,
    pub default_resolution: TimestampResolution,
    pub element_kinds: Vec<ElementKindRegistration>,
    pub metric_definitions: Vec<MetricDefinition>,
}

pub struct Muse {
    client: Arc<dyn Transport + Send + Sync>,
    state: Arc<State>,
    recorder: Option<Arc<Mutex<dyn Recorder + Send + Sync>>>,
    tasks: Vec<JoinHandle<()>>,
    cancellation_token: CancellationToken,
    is_initialized: Arc<AtomicBool>,
}

impl Drop for Muse {
    fn drop(&mut self) {
        self.cancellation_token.cancel();
        for task in &self.tasks {
            task.abort();
        }
    }
}

impl Muse {
    pub fn new(config: Config) -> Self {
        // TODO validate element_kinds and metric_definitions

        let client: Arc<dyn Transport + Send + Sync> = match config.client_type {
            ClientType::Poet => Arc::new(PoetClient::new(config.endpoints)),
            ClientType::Mock => Arc::new(MockClient::new()),
        };

        let recorder: Option<Arc<Mutex<dyn Recorder + Send + Sync>>> = if config.recording_enabled {
            if let Some(path) = &config.recording_path {
                let file_recorder =
                    FileRecorder::new(Path::new(path)).expect("Failed to create FileRecorder");
                Some(Arc::new(Mutex::new(file_recorder))) // Wrap in Mutex here
            } else {
                panic!("Recording enabled but no recording path provided");
            }
        } else {
            None
        };

        // Create the cancellation token
        let cancellation_token = CancellationToken::new();

        let mut muse = Self {
            client,
            state: Arc::new(State::new()),
            recorder,
            tasks: Vec::new(),
            cancellation_token: cancellation_token.clone(),
            is_initialized: Arc::new(AtomicBool::new(false)),
        };

        muse.start_tasks(config.element_kinds, config.metric_definitions);

        muse
    }

    fn start_tasks(
        &mut self,
        element_kinds: Vec<ElementKindRegistration>,
        metric_definitions: Vec<MetricDefinition>,
    ) {
        let cancellation_token = self.cancellation_token.clone();
        let client = self.client.clone();
        let state = self.state.clone();
        let is_initialized = self.is_initialized.clone();

        // Start the initialization task
        let init_task_handle = tokio::spawn(tasks::init_task::start_init_task(
            cancellation_token.clone(),
            client,
            state,
            element_kinds,
            metric_definitions,
            is_initialized,
        ));

        self.tasks.push(init_task_handle);
    }

    pub fn is_initialized(&self) -> bool {
        self.is_initialized.load(Ordering::SeqCst)
    }

    pub async fn register_element(
        &self,
        kind_code: &str,
        name: String,
        metadata: HashMap<String, String>,
        parent_id: Option<ElementId>,
    ) -> Result<ElementId, Error> {
        // Record the event if recorder is enabled
        if let Some(recorder) = &self.recorder {
            let event = RecordedEvent::ElementRegistration {
                kind_code: kind_code.to_string(),
                name: name.clone(),
                metadata: metadata.clone(),
                parent_id: parent_id.clone(),
            };
            recorder.lock().await.record(event).await?;
        }

        if !self.state.is_valid_element_kind_code(kind_code) {
            return Err(Error::InvalidElementKindCode(kind_code.to_string()));
        }

        // Create the element registration
        let element = ElementRegistration::new(kind_code, name, metadata, parent_id);

        // // Register the element immediately using the client
        // let result = self.client.register_elements([element.clone()]).await?;
        // let element_id = result.first().ok_or(Error::RegistrationFailed)?.clone()?;

        // // Update state with the new element
        // self.state
        //     .update_elements(&[element], &[Ok(element_id)])
        //     .await;

        Ok(0)
    }

    pub async fn send_metric(
        &self,
        element_id: ElementId,
        metric_code: &str,
        value: MetricValue,
    ) -> Result<(), Error> {
        // Record the event if recorder is enabled
        if let Some(recorder) = &self.recorder {
            let event = RecordedEvent::SendMetric {
                element_id,
                metric_code: metric_code.to_string(),
                value,
            };
            recorder.lock().await.record(event).await?;
        }

        if !self.state.is_valid_metric_code(metric_code) {
            return Err(Error::InvalidMetricCode(metric_code.to_string()));
        }

        let payload = MetricPayload {
            time: time::utc_now_i64(),
            element_id,
            metric_ids: vec![metric_id_from_code(metric_code)],
            values: vec![Some(value)],
        };
        // TODO accumulate the metric in the metric buffer

        Ok(())
    }

    pub async fn replay(&self, replay_path: &Path) -> Result<(), Error> {
        // TODO record should store time deltas between the functions
        // TODO so replay will replay it with the exact same delays
        // * Like that it also tests caches, rates,
        let mut replayer = FileReplayer::new(replay_path)?;
        while let Some(event) = replayer.next_event().await? {
            match event {
                RecordedEvent::ElementRegistration {
                    kind_code,
                    name,
                    metadata,
                    parent_id,
                } => {
                    self.register_element(&kind_code, name, metadata, parent_id)
                        .await?;
                }
                RecordedEvent::SendMetric {
                    element_id,
                    metric_code,
                    value,
                } => {
                    self.send_metric(element_id, &metric_code, value).await?;
                }
                RecordedEvent::EndpointUpdate(_endpoints) => {
                    // Handle endpoint updates if needed
                }
            }
        }
        Ok(())
    }
}
