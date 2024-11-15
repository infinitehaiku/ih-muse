use pyo3::prelude::*;

use ih_muse_proto::TimestampResolution as RustTimestampResolution;

#[pyclass(eq, eq_int, name = "TimestampResolution")]
#[derive(Clone, PartialEq)]
pub enum PyTimestampResolution {
    Years,
    Months,
    Weeks,
    Days,
    Hours,
    Minutes,
    Seconds,
    Milliseconds,
    Microseconds,
}

impl From<RustTimestampResolution> for PyTimestampResolution {
    fn from(ts: RustTimestampResolution) -> Self {
        match ts {
            RustTimestampResolution::Years => PyTimestampResolution::Years,
            RustTimestampResolution::Months => PyTimestampResolution::Months,
            RustTimestampResolution::Weeks => PyTimestampResolution::Weeks,
            RustTimestampResolution::Days => PyTimestampResolution::Days,
            RustTimestampResolution::Hours => PyTimestampResolution::Hours,
            RustTimestampResolution::Minutes => PyTimestampResolution::Minutes,
            RustTimestampResolution::Seconds => PyTimestampResolution::Seconds,
            RustTimestampResolution::Milliseconds => PyTimestampResolution::Milliseconds,
            RustTimestampResolution::Microseconds => PyTimestampResolution::Microseconds,
        }
    }
}

impl From<PyTimestampResolution> for RustTimestampResolution {
    fn from(py_res: PyTimestampResolution) -> Self {
        match py_res {
            PyTimestampResolution::Years => RustTimestampResolution::Years,
            PyTimestampResolution::Months => RustTimestampResolution::Months,
            PyTimestampResolution::Weeks => RustTimestampResolution::Weeks,
            PyTimestampResolution::Days => RustTimestampResolution::Days,
            PyTimestampResolution::Hours => RustTimestampResolution::Hours,
            PyTimestampResolution::Minutes => RustTimestampResolution::Minutes,
            PyTimestampResolution::Seconds => RustTimestampResolution::Seconds,
            PyTimestampResolution::Milliseconds => RustTimestampResolution::Milliseconds,
            PyTimestampResolution::Microseconds => RustTimestampResolution::Microseconds,
        }
    }
}
