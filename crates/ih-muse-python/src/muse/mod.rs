// crates/ih-muse-python/src/muse/mod.rs

#[cfg(feature = "pymethods")]
mod general;
#[cfg(feature = "pymethods")]
mod io;
#[cfg(feature = "pymethods")]
mod serde;

use std::sync::Arc;
use tokio::sync::Mutex;

use pyo3::pyclass;

use ih_muse::Muse as RustMuse;

#[pyclass]
#[repr(transparent)]
pub struct PyMuse {
    muse: Arc<Mutex<RustMuse>>,
}

impl From<RustMuse> for PyMuse {
    fn from(muse: RustMuse) -> Self {
        PyMuse {
            muse: Arc::new(Mutex::new(muse)),
        }
    }
}

impl PyMuse {
    pub(crate) fn new(muse: RustMuse) -> Self {
        PyMuse {
            muse: Arc::new(Mutex::new(muse)),
        }
    }
}
