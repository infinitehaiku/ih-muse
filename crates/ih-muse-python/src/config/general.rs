// crates/ih-muse-python/src/config/general.rs

use std::time::Duration;

use pyo3::prelude::*;

use super::{PyClientType, PyConfig};
use crate::error::PyMusesErr;
use crate::proto::PyElementKindRegistration;
use crate::proto::PyMetricDefinition;
use crate::proto::PyTimestampResolution;
use ih_muse::prelude::Config as RustConfig;

#[pymethods]
impl PyConfig {
    #[new]
    #[pyo3(signature = (endpoints, client_type, default_resolution, element_kinds, metric_definitions, max_reg_elem_retries, recording_enabled, recording_path=None, recording_flush_secs=None))]
    pub fn __init__(
        endpoints: Vec<String>,
        client_type: PyClientType,
        default_resolution: PyTimestampResolution,
        element_kinds: Vec<PyElementKindRegistration>,
        metric_definitions: Vec<PyMetricDefinition>,
        max_reg_elem_retries: usize,
        recording_enabled: bool,
        recording_path: Option<String>,
        recording_flush_secs: Option<u64>,
    ) -> PyResult<Self> {
        let muse = RustConfig::new(
            endpoints,
            client_type.into(),
            recording_enabled,
            recording_path,
            recording_flush_secs.map(Duration::from_secs),
            default_resolution.into(),
            element_kinds.into_iter().map(|p| p.inner).collect(),
            metric_definitions.into_iter().map(|p| p.inner).collect(),
            None,
            max_reg_elem_retries,
        )
        .map_err(PyMusesErr::from)?;
        Ok(Self::from(muse))
    }
}
