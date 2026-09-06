use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::MetricPayload;

pub const PRODUCER_PROTOCOL_VERSION: u16 = 1;
pub const MAX_BATCH_MEASUREMENTS: usize = 16_384;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProducerEnvelope {
    pub protocol_version: u16,
    pub batch: ProducerBatch,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProducerBatch {
    pub producer_id: Uuid,
    pub batch_id: Uuid,
    pub sequence: u64,
    pub definitions_revision: u64,
    pub from_dt: i64,
    pub to_dt: i64,
    pub measurements: Vec<MetricPayload>,
}

impl ProducerEnvelope {
    pub fn new(batch: ProducerBatch) -> Self {
        Self {
            protocol_version: PRODUCER_PROTOCOL_VERSION,
            batch,
        }
    }

    pub fn validate(&self) -> Result<(), ProducerValidationError> {
        if self.protocol_version != PRODUCER_PROTOCOL_VERSION {
            return Err(ProducerValidationError::UnsupportedVersion(
                self.protocol_version,
            ));
        }
        self.batch.validate()
    }
}

impl ProducerBatch {
    pub fn validate(&self) -> Result<(), ProducerValidationError> {
        if self.producer_id.is_nil() || self.batch_id.is_nil() || self.sequence == 0 {
            return Err(ProducerValidationError::MissingIdentifier);
        }
        if self.from_dt >= self.to_dt {
            return Err(ProducerValidationError::InvalidInterval);
        }
        if self.measurements.is_empty() || self.measurements.len() > MAX_BATCH_MEASUREMENTS {
            return Err(ProducerValidationError::InvalidMeasurementCount);
        }
        if self.measurements.iter().any(|measurement| {
            measurement.time < self.from_dt
                || measurement.time >= self.to_dt
                || measurement.metric_ids.len() != measurement.values.len()
        }) {
            return Err(ProducerValidationError::InvalidMeasurement);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Error, PartialEq)]
pub enum ProducerValidationError {
    #[error("unsupported producer protocol version {0}")]
    UnsupportedVersion(u16),
    #[error("producer batch is missing an identifier")]
    MissingIdentifier,
    #[error("producer batch must use a non-empty half-open timestamp interval")]
    InvalidInterval,
    #[error("producer batch has an invalid measurement count")]
    InvalidMeasurementCount,
    #[error(
        "producer batch contains a measurement outside its interval or with misaligned values"
    )]
    InvalidMeasurement,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn batch() -> ProducerBatch {
        ProducerBatch {
            producer_id: Uuid::new_v4(),
            batch_id: Uuid::new_v4(),
            sequence: 1,
            definitions_revision: 1,
            from_dt: 10,
            to_dt: 20,
            measurements: vec![MetricPayload::new(10, 1, vec![1], vec![Some(1.0)])],
        }
    }

    #[test]
    fn producer_envelope_round_trips_and_validates() {
        let envelope = ProducerEnvelope::new(batch());
        let encoded = serde_json::to_string(&envelope).unwrap();
        assert_eq!(
            serde_json::from_str::<ProducerEnvelope>(&encoded).unwrap(),
            envelope
        );
        assert!(envelope.validate().is_ok());
    }

    #[test]
    fn producer_envelope_rejects_incompatible_or_invalid_batches() {
        let mut envelope = ProducerEnvelope::new(batch());
        envelope.protocol_version = 2;
        assert!(matches!(
            envelope.validate(),
            Err(ProducerValidationError::UnsupportedVersion(2))
        ));
        envelope.protocol_version = PRODUCER_PROTOCOL_VERSION;
        envelope.batch.measurements[0].time = 20;
        assert_eq!(
            envelope.validate(),
            Err(ProducerValidationError::InvalidMeasurement)
        );
    }
}
