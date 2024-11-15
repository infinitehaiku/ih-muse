// crates/ih-muse-python/src/proto/metric.rs

use pyo3::prelude::*;

use super::PyMetricDefinition;
use ih_muse_proto::MetricDefinition as RustMetricDefinition;

#[pymethods]
impl PyMetricDefinition {
    #[new]
    pub fn __init__(code: String, name: String, description: String) -> PyResult<Self> {
        let ekr = RustMetricDefinition::new(code, name, description);
        Ok(Self::from(ekr))
    }
}
