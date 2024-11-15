// crates/ih-muse-python/src/exceptions.rs

use pyo3::create_exception;
use pyo3::exceptions::PyException;

create_exception!(polars.exceptions, MuseError, PyException);
create_exception!(polars.exceptions, MuseInitializationTimeoutError, MuseError);
create_exception!(polars.exceptions, ConfigurationError, MuseError);
create_exception!(polars.exceptions, ClientError, MuseError);
create_exception!(polars.exceptions, RecordingError, MuseError);
create_exception!(polars.exceptions, ReplayingError, MuseError);
create_exception!(polars.exceptions, InvalidFileExtensionError, MuseError);
create_exception!(polars.exceptions, InvalidElementKindCodeError, MuseError);
create_exception!(polars.exceptions, InvalidMetricCodeError, MuseError);
