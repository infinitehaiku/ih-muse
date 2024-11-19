// crates/ih-muse-python/src/muse/mod.rs

#[cfg(feature = "pymethods")]
mod general;
#[cfg(feature = "pymethods")]
mod io;
#[cfg(feature = "pymethods")]
mod serde;

use std::sync::{atomic::AtomicBool, Arc};
use tokio::sync::Mutex;

use pyo3::pyclass;

use ih_muse::Muse as RustMuse;

#[pyclass]
pub struct PyMuse {
    muse: Arc<Mutex<RustMuse>>,
    is_initialized: Arc<AtomicBool>,
}

impl From<RustMuse> for PyMuse {
    fn from(muse: RustMuse) -> Self {
        let is_initialized = muse.is_initialized.clone();
        PyMuse {
            muse: Arc::new(Mutex::new(muse)),
            is_initialized,
        }
    }
}
