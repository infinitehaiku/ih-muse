// crates/ih-muse-python/src/muse/general.rs

use std::sync::Arc;
use tokio::sync::Mutex;

use pyo3::prelude::*;
use pyo3_async_runtimes::tokio::future_into_py;
use tokio::time::Duration;

use super::PyMuse;
use crate::config::PyConfig;
use crate::error::PyMusesErr;
use ih_muse::Muse as RustMuse;

#[pymethods]
impl PyMuse {
    #[new]
    pub fn __init__(config: &PyConfig) -> PyResult<Self> {
        let muse = RustMuse::new(&config.inner).map_err(PyMusesErr::from)?;
        Ok(Self {
            muse: Arc::new(Mutex::new(muse)),
        })
    }

    #[pyo3(signature = (timeout=None))]
    pub fn initialize<'p>(
        &self,
        py: Python<'p>,
        timeout: Option<f64>,
    ) -> PyResult<Bound<'p, PyAny>> {
        let muse = self.muse.clone();

        future_into_py(py, async move {
            let timeout = timeout.map(Duration::from_secs_f64);

            // Acquire the mutex lock
            let mut muse_guard = muse.lock().await;

            muse_guard
                .initialize(timeout)
                .await
                .map_err(PyMusesErr::from)?;

            Ok(())
        })
    }
}
