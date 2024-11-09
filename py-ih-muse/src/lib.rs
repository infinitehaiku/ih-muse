// py-ih-muse/src/lib.rs

use ih_muse_recorder::{FileRecorder, FileReplayer};

pub struct Muse {
    transport: Box<dyn Transport>,
    recorder: Option<Box<dyn Recorder>>,
    cache: Arc<dyn Cache>,
}

impl Muse {
    pub fn new(config: MuseConfig) -> Self {
        // Implementation
    }

    pub async fn register_element(
        &self,
        registration: ElementRegistration,
    ) -> Result<ElementId, Error> {
        let element_id = self
            .transport
            .register_element(registration.clone())
            .await?;
        if let Some(recorder) = &self.recorder {
            recorder
                .record(RecordedEvent::ElementRegistration(registration))
                .await?;
        }
        Ok(element_id)
    }

    pub async fn record_metric(
        &self,
        element: ElementId,
        metric: MetricId,
        value: f64,
    ) -> Result<(), Error> {
        let payload = MetricPayload {
            element_id: element.clone(),
            metric_id: metric.clone(),
            value,
            timestamp: Utc::now(),
        };

        if self
            .cache
            .should_send(&element, &metric, Duration::from_secs(60))
            .await
        {
            self.transport.send_metric(payload.clone()).await?;
            if let Some(recorder) = &self.recorder {
                recorder.record(RecordedEvent::MetricSent(payload)).await?;
            }
        }
        Ok(())
    }
}

pub struct MuseConfig {
    pub endpoints: Vec<String>,
    pub recording_path: Option<PathBuf>,
    pub cache_strategy: Arc<dyn Cache>,
}
