use std::ops::RangeInclusive;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

#[derive(Deserialize, Serialize, Debug)]
pub struct NodeElementRange {
    pub node_id: Uuid,
    pub range: OrdRangeInc,
}

#[derive(Deserialize, Serialize, Debug, Default)]
pub struct GetRangesRequest {
    pub ini: Option<u64>,
    pub end: Option<u64>,
}

/// Wrapper around `RangeInclusive<u64>` to implement `Ord` and `PartialOrd`.
#[derive(Debug, Clone)]
pub struct OrdRangeInc(RangeInclusive<u64>);

impl OrdRangeInc {
    pub const MIN_SIZE: u64 = 10;

    pub fn new(start: u64, end: u64) -> Self {
        if (end - start) + 1 < OrdRangeInc::MIN_SIZE {
            panic!("Range len cannot be smaller than {}", OrdRangeInc::MIN_SIZE)
        }
        OrdRangeInc(start..=end)
    }

    pub fn len(&self) -> u64 {
        self.end() - self.start() + 1
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn start(&self) -> &u64 {
        self.0.start()
    }

    pub fn end(&self) -> &u64 {
        self.0.end()
    }

    pub fn contains(&self, item: &u64) -> bool {
        self.0.contains(item)
    }
}

impl From<RangeInclusive<u64>> for OrdRangeInc {
    fn from(range: RangeInclusive<u64>) -> Self {
        OrdRangeInc(range)
    }
}

// Implement Serialize for OrdRangeInc
impl Serialize for OrdRangeInc {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let (start, end) = self.0.clone().into_inner();
        let tuple = (start, end);
        tuple.serialize(serializer)
    }
}

// Implement Deserialize for OrdRangeInc
impl<'de> Deserialize<'de> for OrdRangeInc {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let (start, end) = <(u64, u64)>::deserialize(deserializer)?;
        Ok(OrdRangeInc::new(start, end))
    }
}
