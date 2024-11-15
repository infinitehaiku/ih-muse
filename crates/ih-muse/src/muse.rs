// crates/ih-muse/src/muse.rs

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;

use crate::config::{ClientType, Config};
use crate::tasks;
use ih_muse_client::{MockClient, PoetClient};
use ih_muse_core::prelude::*;
use ih_muse_proto::prelude::*;
use ih_muse_record::{FileRecorder, FileReplayer, RecordedEvent, Recorder, Replayer};

pub struct Muse {
    client: Arc<dyn Transport + Send + Sync>,
    state: Arc<State>,
    recorder: Option<Arc<Mutex<dyn Recorder + Send + Sync>>>,
    tasks: Vec<JoinHandle<()>>,
    cancellation_token: CancellationToken,
    is_initialized: Arc<AtomicBool>,
    element_buffer: Arc<ElementBuffer>,
    metric_buffer: Arc<MetricBuffer>,
    config: Config,
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
    /// Creates a new instance of Muse.
    pub fn new(config: &Config) -> MuseResult<Self> {
        // TODO validate element_kinds and metric_definitions

        let client: Arc<dyn Transport + Send + Sync> = match config.client_type {
            ClientType::Poet => Arc::new(PoetClient::new(&config.endpoints)),
            ClientType::Mock => Arc::new(MockClient::new()),
        };

        let recorder: Option<Arc<Mutex<dyn Recorder + Send + Sync>>> = if config.recording_enabled {
            if let Some(path) = &config.recording_path {
                let file_recorder =
                    FileRecorder::new(Path::new(path)).expect("Failed to create FileRecorder");
                Some(Arc::new(Mutex::new(file_recorder))) // Wrap in Mutex here
            } else {
                return Err(MuseError::Configuration(
                    "Recording enabled but no recording path provided".to_string(),
                ));
            }
        } else {
            None
        };

        // Create the cancellation token
        let cancellation_token = CancellationToken::new();

        Ok(Self {
            client,
            state: Arc::new(State::new()),
            recorder,
            tasks: Vec::new(),
            cancellation_token: cancellation_token.clone(),
            is_initialized: Arc::new(AtomicBool::new(false)),
            element_buffer: Arc::new(ElementBuffer::new(config.max_reg_elem_retries)),
            metric_buffer: Arc::new(MetricBuffer::new()),
            config: config.clone(),
        })
    }

    /// Initialize and start background tasks
    pub async fn initialize(&mut self, timeout: Option<Duration>) -> MuseResult<()> {
        self.start_tasks(
            self.config.element_kinds.to_vec(),
            self.config.metric_definitions.to_vec(),
            self.config.cluster_monitor_interval,
        );
        // Wait for initialization to complete, with an optional timeout
        let deadline = timeout.map(|t| tokio::time::Instant::now() + t);
        while !self.is_initialized() {
            if let Some(deadline) = deadline {
                if tokio::time::Instant::now() >= deadline {
                    return Err(MuseError::MuseInitializationTimeout(timeout.unwrap()));
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        Ok(())
    }

    /// Get a reference to the internal State.
    pub fn get_state(&self) -> Arc<State> {
        self.state.clone()
    }

    /// Get a reference to the internal Transport client.
    pub fn get_client(&self) -> Arc<dyn Transport + Send + Sync> {
        self.client.clone()
    }

    fn start_tasks(
        &mut self,
        element_kinds: Vec<ElementKindRegistration>,
        metric_definitions: Vec<MetricDefinition>,
        cluster_monitor_interval: Option<Duration>,
    ) {
        let cancellation_token = self.cancellation_token.clone();
        let client = self.client.clone();
        let state = self.state.clone();
        let is_initialized = self.is_initialized.clone();

        // Start the initialization task
        let init_task_handle = tokio::spawn(tasks::start_init_task(
            cancellation_token.clone(),
            client.clone(),
            state.clone(),
            element_kinds,
            metric_definitions,
            is_initialized.clone(),
        ));
        self.tasks.push(init_task_handle);

        // Start the cluster monitoring task
        let cluster_monitoring_handle = tokio::spawn(tasks::start_cluster_monitor(
            cancellation_token.clone(),
            client.clone(),
            state.clone(),
            is_initialized,
            cluster_monitor_interval.unwrap_or(Duration::from_secs(60)),
        ));
        self.tasks.push(cluster_monitoring_handle);

        // Start element registration task
        let elem_reg_handle = tokio::spawn(tasks::start_element_registration_task(
            cancellation_token.clone(),
            client.clone(),
            state.clone(),
            self.element_buffer.clone(),
        ));
        self.tasks.push(elem_reg_handle);

        // Start metric sender task
        let metric_sender_handle = tokio::spawn(tasks::start_metric_sender_task(
            cancellation_token.clone(),
            client,
            state,
            self.metric_buffer.clone(),
        ));
        self.tasks.push(metric_sender_handle);
    }

    /// Returns whether the Muse client is initialized.
    pub fn is_initialized(&self) -> bool {
        self.is_initialized.load(Ordering::SeqCst)
    }

    /// Registers a new element.
    pub async fn register_element(
        &self,
        kind_code: &str,
        name: String,
        metadata: HashMap<String, String>,
        parent_id: Option<ElementId>,
    ) -> MuseResult<LocalElementId> {
        let local_elem_id = generate_local_element_id();
        self.register_element_inner(local_elem_id, kind_code, name, metadata, parent_id)
            .await?;
        Ok(local_elem_id)
    }

    async fn register_element_inner(
        &self,
        local_elem_id: LocalElementId,
        kind_code: &str,
        name: String,
        metadata: HashMap<String, String>,
        parent_id: Option<ElementId>,
    ) -> MuseResult<()> {
        // Record the event if recorder is enabled
        if let Some(recorder) = &self.recorder {
            let event = RecordedEvent::ElementRegistration {
                local_elem_id,
                kind_code: kind_code.to_string(),
                name: name.clone(),
                metadata: metadata.clone(),
                parent_id,
            };
            recorder.lock().await.record(event).await?;
        }

        if !self.state.is_valid_element_kind_code(kind_code) {
            return Err(MuseError::InvalidElementKindCode(kind_code.to_string()));
        }

        let element = ElementRegistration::new(kind_code, name, metadata, parent_id);
        self.element_buffer
            .add_element(local_elem_id, element)
            .await;

        Ok(())
    }

    /// Sends a metric value.
    pub async fn send_metric(
        &self,
        local_elem_id: LocalElementId,
        metric_code: &str,
        value: MetricValue,
    ) -> MuseResult<()> {
        // Record the event if recorder is enabled
        if let Some(recorder) = &self.recorder {
            let event = RecordedEvent::SendMetric {
                local_elem_id,
                metric_code: metric_code.to_string(),
                value,
            };
            recorder.lock().await.record(event).await?;
        }

        if !self.state.is_valid_metric_code(metric_code) {
            return Err(MuseError::InvalidMetricCode(metric_code.to_string()));
        }

        self.metric_buffer
            .add_metric(local_elem_id, metric_code.to_string(), value)
            .await;

        Ok(())
    }

    /// Replays events from a recording.
    pub async fn replay(&self, replay_path: &Path) -> MuseResult<()> {
        // TODO record should store time deltas between the functions
        // TODO so replay will replay it with the exact same delays
        // * Like that it also tests caches, rates,
        let mut replayer = FileReplayer::new(replay_path)?;
        while let Some(event) = replayer.next_event().await? {
            match event {
                RecordedEvent::ElementRegistration {
                    local_elem_id,
                    kind_code,
                    name,
                    metadata,
                    parent_id,
                } => {
                    self.register_element_inner(
                        local_elem_id,
                        &kind_code,
                        name,
                        metadata,
                        parent_id,
                    )
                    .await?;
                }
                RecordedEvent::SendMetric {
                    local_elem_id,
                    metric_code,
                    value,
                } => {
                    self.send_metric(local_elem_id, &metric_code, value).await?;
                }
                RecordedEvent::EndpointUpdate(_endpoints) => {
                    // Handle endpoint updates if needed
                }
            }
        }
        Ok(())
    }
}
