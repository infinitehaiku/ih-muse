pub type ElementKindId = u64;
pub type ElementId = u64;
pub type MetricId = u32;
pub type MetricValue = f32;
pub type Timestamp = i64;

#[derive(Debug)]
pub enum ValidationError {
    Empty,
    TooLong,
}
