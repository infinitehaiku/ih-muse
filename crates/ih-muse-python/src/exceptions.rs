// crates/ih-muse-python/src/exceptions.rs

use pyo3::create_exception;
use pyo3::exceptions::PyException;

create_exception!(ih_muse.exceptions, MuseError, PyException);
create_exception!(
    ih_muse.exceptions,
    MuseInitializationTimeoutError,
    MuseError
);
create_exception!(ih_muse.exceptions, ConfigurationError, MuseError);
create_exception!(ih_muse.exceptions, ClientError, MuseError);
create_exception!(ih_muse.exceptions, RecordingError, MuseError);
create_exception!(ih_muse.exceptions, ReplayingError, MuseError);
create_exception!(ih_muse.exceptions, InvalidFileExtensionError, MuseError);
create_exception!(ih_muse.exceptions, InvalidElementKindCodeError, MuseError);
create_exception!(ih_muse.exceptions, InvalidMetricCodeError, MuseError);
create_exception!(ih_muse.exceptions, DurationConversionError, MuseError);
