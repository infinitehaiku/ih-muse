use pyo3::prelude::*;

// use ih_muse_core;

#[pyfunction]
fn double(x: usize) -> usize {
    ih_muse_core::double(x)
}

/// This module is implemented in Rust.
#[pymodule]
fn ih_muse(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(double, m)?)
}
