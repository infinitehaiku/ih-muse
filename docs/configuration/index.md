# Configuration

The `Config` class (in Python) and the `Config` struct (in Rust) are central to initializing and customizing the behavior of the Muse client. They encapsulate all the necessary settings required to interact with the Muse system effectively.

## Overview

The configuration allows you to:

- Specify endpoints for connecting to the Muse system.
- Choose the client type (e.g., `Poet` or `Mock`).
- Define element kinds and metric definitions for registration and reporting.
- Set default timestamp resolutions.
- Enable or disable event recording and specify recording paths.
- Configure retries and intervals for various operations.

## Configuration Fields

- **Endpoints**: List of endpoint URLs for the Muse client.
- **Client Type**: Determines the type of client (`Poet` or `Mock`).
- **Recording Enabled**: Enables or disables event recording.
- **Recording Path**: File path for recording events (required if recording is enabled).
- **Default Resolution**: Default timestamp resolution for metrics.
- **Element Kinds**: List of element kinds to register.
- **Metric Definitions**: List of metric definitions available for reporting.
- **Cluster Monitor Interval**: Interval for cluster monitoring tasks (optional).
- **Max Registration Retries**: Maximum number of retries for element registration.

## Validation Rules

The configuration settings undergo validation to ensure that:

- **Endpoints**: Must not be empty if using the `Poet` client type.
- **Element Kinds**: Cannot be empty; at least one must be provided.
- **Metric Definitions**: Cannot be empty; at least one must be provided.
- **Recording Path**: Must be specified if recording is enabled.

## Example Usage

Below are examples of how to define the configuration in both Python and Rust.

::::{tab-set}

::: {tab-item} Python

```python
import asyncio
from ih_muse import Muse, Config, ClientType, TimestampResolution
from ih_muse.proto import ElementKindRegistration, MetricDefinition

# Define the configuration
config = Config(
    endpoints=["http://localhost:8080"],
    client_type=ClientType.Poet,
    default_resolution=TimestampResolution.Milliseconds,
    element_kinds=[
        ElementKindRegistration("kind_code", "description")
    ],
    metric_definitions=[
        MetricDefinition("metric_code", "description")
    ],
    max_reg_elem_retries=3,
    recording_enabled=False,
    recording_path=None,  # Optional if recording is disabled
)

# Initialize the Muse client
async def main():
    muse = Muse(config)
    await muse.initialize(timeout=5.0)
    # ... use the muse client

asyncio.run(main())
```

:::

::: {tab-item} Rust

```rust
use ih_muse::prelude::*;
use ih_muse::config::{ClientType, Config};
use ih_muse_proto::prelude::*;
use std::collections::HashMap;
use std::time::Duration;

#[tokio::main]
async fn main() -> MuseResult<()> {
    // Define the configuration
    let config = Config::new(
        vec!["http://localhost:8080".to_string()],
        ClientType::Poet,
        false,                      // recording_enabled
        None,                       // recording_path
        TimestampResolution::Milliseconds,
        vec![ElementKindRegistration::new("kind_code", "description")],
        vec![MetricDefinition::new("metric_code", "description")],
        Some(Duration::from_secs(60)),  // cluster_monitor_interval
        3,                              // max_reg_elem_retries
    )?;

    // Initialize the Muse client
    let mut muse = Muse::new(&config)?;
    muse.initialize(Some(Duration::from_secs(5))).await?;
    // ... use the muse client

    Ok(())
}
```

:::

::::

## Detailed Explanation

### Endpoints

A list of URLs where the Muse client can connect to. At least one endpoint must be provided when using the `Poet` client type.

### Client Type

Determines the type of client to instantiate:

- `Poet`: Communicates with the actual Muse system.
- `Mock`: Uses a mock client for testing purposes without making real network calls.

### Recording Options

- **Recording Enabled**: When set to `True` (Python) or `true` (Rust), the client will record events for later analysis.
- **Recording Path**: Specifies where the recorded events should be saved. This must be provided if recording is enabled.

### Default Resolution

Specifies the default timestamp resolution for metrics (e.g., milliseconds, seconds).

### Element Kinds

A list of `ElementKindRegistration` instances that define the kinds of elements you plan to register with the Muse system.

### Metric Definitions

A list of `MetricDefinition` instances that define the metrics you plan to report.

### Cluster Monitor Interval

(Optional) The interval at which the client should perform cluster monitoring tasks. This is typically relevant for advanced configurations.

### Max Registration Retries

Specifies how many times the client should retry registering an element in case of failure.

## Validation Behavior (Rust Implementation)

The Rust implementation includes a `validate` method that ensures the configuration is correct before the client is initialized.

Validation checks include:

- **Client Type and Endpoints**: If the client type is `Poet`, there must be at least one endpoint specified.
- **Element Kinds and Metric Definitions**: Both must contain at least one entry.
- **Recording Path**: If recording is enabled, a valid recording path must be provided.

If any of these validations fail, the client initialization will return a `MuseError::Configuration` error.

## Common Errors

- **Missing Endpoints**: Ensure that you provide at least one endpoint when using the `Poet` client.
- **Empty Element Kinds or Metric Definitions**: Provide at least one element kind and one metric definition.
- **Recording Enabled Without Path**: If you enable recording, you must specify a valid path for the recording file.

## Tips

- **Asynchronous Initialization**: Both Python and Rust clients use asynchronous initialization methods. Ensure you await these calls appropriately.
- **Error Handling**: Always handle potential errors that may arise during client initialization or operation, especially network-related issues.

## Conclusion

Properly configuring the Muse client is crucial for successful integration with the Muse system. By understanding and utilizing the configuration options available, you can tailor the client to meet your specific needs, whether you're developing in Python or Rust.
