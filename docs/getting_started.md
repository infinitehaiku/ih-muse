# Getting Started

This guide will help you get started with IH-Muse. It covers all the fundamental features and functionalities of the library, making it easy for new users to familiarize themselves with the basics, from initial installation and setup to core functionalities.

## Installing IH-Muse

:::{tab-set}

:::{tab-item} :fontawesome-brands-python: Python

```bash
pip install ih-muse
```

:::

:::{tab-item} :fontawesome-brands-rust: Rust

```bash
cargo add ih-muse

# Or add it directly to your Cargo.toml
[dependencies]
ih-muse = "0.1.0"
```

:::

:::

Ensure that you have Python 3.7+ and/or Rust and Cargo installed on your system. You can get Rust from [rustup.rs](https://rustup.rs/).

## Basic Usage

### Creating a Configuration

First, create a configuration object for your Muse client.

:::{tab-set}

:::{tab-item} :fontawesome-brands-python: Python

```python
from ih_muse import Config, ClientType
from ih_muse.proto import (
    ElementKindRegistration,
    MetricDefinition,
    TimestampResolution,
)

config = Config(
    endpoints=["http://localhost:8080"],
    client_type=ClientType.Poet,
    default_resolution=TimestampResolution.Milliseconds,
    element_kinds=[ElementKindRegistration("kind_code", "description")],
    metric_definitions=[MetricDefinition("metric_code", "description")],
    max_reg_elem_retries=3,
    recording_enabled=False,
)
```

:::

:::{tab-item} :fontawesome-brands-rust: Rust

```rust
use ih_muse::config::{Config, ClientType};
use ih_muse_proto::prelude::*;

let config = Config::new(
    vec!["http://localhost:8080".to_string()],
    ClientType::Poet,
    false,
    None,
    TimestampResolution::Milliseconds,
    vec![ElementKindRegistration::new("kind_code", "description")],
    vec![MetricDefinition::new("metric_code", "description")],
    Some(std::time::Duration::from_secs(60)),
    3,
).expect("Failed to create config");
```

:::

:::

### Initializing the Muse Client

Create and initialize the Muse client.

:::{tab-set}

:::{tab-item} :fontawesome-brands-python: Python

```python
from ih_muse import Muse

muse = Muse(config)
await muse.initialize(timeout=5.0)
```

:::

:::{tab-item} :fontawesome-brands-rust: Rust

```rust
use ih_muse::Muse;

let mut muse = Muse::new(&config).expect("Failed to create Muse client");
muse.initialize(Some(std::time::Duration::from_secs(5)))
    .await
    .expect("Failed to initialize Muse client");
```

:::

:::

### Registering Elements

Register a new element with the Muse system.

:::{tab-set}

:::{tab-item} :fontawesome-brands-python: Python

```python
local_elem_id = await muse.register_element(
    kind_code="kind_code",
    name="Element Name",
    metadata={},
    parent_id=None,
)
```

:::

:::{tab-item} :fontawesome-brands-rust: Rust

```rust
use std::collections::HashMap;

let local_elem_id = muse
    .register_element(
        "kind_code",
        "Element Name".to_string(),
        HashMap::new(),
        None,
    )
    .await
    .expect("Failed to register element");
```

:::

:::

### Sending Metrics

Send a metric associated with the registered element.

:::{tab-set}

:::{tab-item} :fontawesome-brands-python: Python

```python
await muse.send_metric(local_elem_id, "metric_code", 42.0)
```

:::

:::{tab-item} :fontawesome-brands-rust: Rust

```rust
use ih_muse_proto::MetricValue;

muse.send_metric(local_elem_id, "metric_code", MetricValue::from(42.0))
    .await
    .expect("Failed to send metric");
```

:::

:::

## Recording and Replaying Events

IH-Muse allows you to record events to a file and replay them later, which is useful for testing and debugging.

### Recording Events

To enable recording, set `recording_enabled` to `true` in your configuration and provide a `recording_path`.

:::{tab-set}

:::{tab-item} :fontawesome-brands-python: Python

```python
config = Config(
    # ... other configuration fields
    recording_enabled=True,
    recording_path="events.bin",  # Recording path
)
```

:::

:::{tab-item} :fontawesome-brands-rust: Rust

```rust
let config = Config::new(
    // ... other configuration fields
    true, // recording_enabled
    Some("events.bin".to_string()), // recording_path
    // ... rest of the configuration
)?;
```

:::

:::

**Note:** The recording file should have a specific extension to determine the serialization format:

- `.bin` for Bincode serialization
- `.json` for JSON serialization

### Replaying Events

To replay events from a recording:

:::{tab-set}

:::{tab-item} :fontawesome-brands-python: Python

```python
await muse.replay("events.bin")
```

:::

:::{tab-item} :fontawesome-brands-rust: Rust

```rust
use std::path::Path;

muse.replay(Path::new("events.bin"))
    .await
    .expect("Failed to replay events");
```

:::

:::

## Command-Line Interface

IH-Muse includes a command-line interface (CLI) tool for interacting with recordings.

### Replaying with the CLI

You can use the CLI tool to replay events from a file:

```shell
ih-muse-cli replay --input events.bin --poet-url http://localhost:8000
```

**Note:** The `--input` argument specifies the recording file, and the `--poet-url` specifies the Muse system endpoint.

---

For more detailed information on recording and replaying, refer to the [Recording](recording.md) and [Replaying](replaying.md) guides.
