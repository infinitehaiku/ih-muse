use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use ih_muse_client::{MockClient, PoetClient, PoetEndpoint};
use ih_muse_core::{time, CacheStrategy, Error, Transport};
use ih_muse_proto::metric_id_from_code;
use ih_muse_proto::{types::*, ElementId, ElementRegistration, MetricPayload};
use ih_muse_record::{FileRecorder, FileReplayer, RecordedEvent, Recorder, Replayer};

pub enum ClientType {
    Poet,
    Mock,
}

pub struct Config {
    pub endpoints: Vec<String>,
    pub cache_strategy: Arc<dyn CacheStrategy + Send + Sync>,
    pub client_type: ClientType,
    pub recording_enabled: bool,
    pub recording_path: Option<String>,
}

pub struct Muse {
    client: Arc<dyn Transport + Send + Sync>,
    recorder: Option<Arc<Mutex<dyn Recorder + Send + Sync>>>,
    cache_strategy: Arc<dyn CacheStrategy + Send + Sync>,
    metric_tx: mpsc::Sender<MetricPayload>,
    metric_task: Option<JoinHandle<()>>,
}

impl Muse {
    pub fn new(config: Config) -> Self {
        let client: Arc<dyn Transport + Send + Sync> = match config.client_type {
            ClientType::Poet => {
                let endpoints = config
                    .endpoints
                    .into_iter()
                    .map(|url| PoetEndpoint { url })
                    .collect();
                Arc::new(PoetClient::new(endpoints))
            }
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

        let cache_strategy = config.cache_strategy.clone();

        let (metric_tx, mut metric_rx) = mpsc::channel::<MetricPayload>(100);
        let client_clone = client.clone();

        // Start background task to send metrics
        let metric_task = tokio::spawn(async move {
            let mut buffer = Vec::new();
            let mut interval = tokio::time::interval(Duration::from_secs(1));

            loop {
                tokio::select! {
                    Some(payload) = metric_rx.recv() => {
                        buffer.push(payload);
                        if buffer.len() >= 100 {
                            // Send metrics
                            let _ = client_clone.send_metrics(buffer.clone()).await;
                            buffer.clear();
                        }
                    },
                    _ = interval.tick() => {
                        if !buffer.is_empty() {
                            // Send metrics
                            let _ = client_clone.send_metrics(buffer.clone()).await;
                            buffer.clear();
                        }
                    }
                }
            }
        });

        Self {
            client,
            recorder,
            cache_strategy,
            metric_tx,
            metric_task: Some(metric_task),
        }
    }

    pub async fn register_element(
        &self,
        element: ElementRegistration,
    ) -> Result<Vec<Result<ElementId, Error>>, Error> {
        // Record the event if recorder is enabled
        if let Some(recorder) = &self.recorder {
            let event = RecordedEvent::ElementRegistration(element.clone());
            recorder.lock().await.record(event).await?;
        }
        // Directly call client for simplicity; you can batch these calls similarly
        self.client.register_elements(vec![element]).await
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

        let payload = MetricPayload {
            time: time::utc_now_i64(),
            element_id,
            metric_ids: vec![metric_id_from_code(metric_code)],
            values: vec![Some(value)],
        };

        // Send the payload to the metric_tx
        self.metric_tx
            .send(payload)
            .await
            .map_err(|e| Error::ClientError(format!("Failed to send metric to buffer: {}", e)))?;

        Ok(())
    }

    pub async fn replay(&self, replay_path: &Path) -> Result<(), Error> {
        // TODO record should store time deltas between the functions
        // TODO so replay will replay it with the exact same delays
        // * Like that it also tests caches, rates,
        let mut replayer = FileReplayer::new(replay_path)?;
        while let Some(event) = replayer.next_event().await? {
            match event {
                RecordedEvent::ElementRegistration(element) => {
                    self.register_element(element).await?;
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
