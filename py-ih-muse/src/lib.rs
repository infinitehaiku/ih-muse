// py-ih-muse/src/lib.rs

#![allow(clippy::nonstandard_macro_braces)]
#![allow(clippy::transmute_undefined_repr)]
#![allow(non_local_definitions)]
#![allow(clippy::too_many_arguments)]

use pyo3::prelude::*;
use pyo3::{wrap_pyfunction, wrap_pymodule};

use ih_muse_python::config::{PyClientType, PyConfig};
use ih_muse_python::proto::PyTimestampResolution;
use ih_muse_python::proto::{PyElementKindRegistration, PyMetricDefinition};
// use ih_muse_python::element_kind_registration::PyElementKindRegistration;
// use ih_muse_python::metric_definition::PyMetricDefinition;
use ih_muse_python::exceptions;
use ih_muse_python::muse::PyMuse;

// #[pymodule]
// fn proto(_py: Python, m: &PyModule) -> PyResult<()> {
//     m.add_class::<PyMuse>()?;
//     Ok(())
// }

#[pymodule]
// #[pyo3(name = "ih_muse")]
fn ih_muse(py: Python, m: &Bound<PyModule>) -> PyResult<()> {
    m.add_class::<PyMuse>()?;
    m.add_class::<PyConfig>()?;
    m.add_class::<PyClientType>()?;
    m.add_class::<PyTimestampResolution>()?;
    m.add_class::<PyElementKindRegistration>()?;
    m.add_class::<PyMetricDefinition>()?;

    #[pyfunction]
    fn get_version() -> &'static str {
        env!("CARGO_PKG_VERSION")
    }
    m.add_function(wrap_pyfunction!(get_version, m)?)?;

    // Add submodules
    // m.add_submodule(proto::module(m)?)?;

    // Exceptions - Errors
    m.add(
        "ConfigurationError",
        py.get_type_bound::<exceptions::ConfigurationError>(),
    )
    .unwrap();
    m.add(
        "ClientError",
        py.get_type_bound::<exceptions::ClientError>(),
    )
    .unwrap();
    m.add(
        "RecordingError",
        py.get_type_bound::<exceptions::RecordingError>(),
    )
    .unwrap();
    m.add(
        "InvalidFileExtensionError",
        py.get_type_bound::<exceptions::InvalidFileExtensionError>(),
    )
    .unwrap();
    m.add(
        "InvalidElementKindCodeError",
        py.get_type_bound::<exceptions::InvalidElementKindCodeError>(),
    )
    .unwrap();
    m.add(
        "InvalidMetricCodeError",
        py.get_type_bound::<exceptions::InvalidMetricCodeError>(),
    )
    .unwrap();

    Ok(())
}
