# ih-muse

[![Release](https://img.shields.io/github/v/release/infinitehaiku/ih-muse)](https://img.shields.io/github/v/release/infinitehaiku/ih-muse)
[![Build status](https://img.shields.io/github/actions/workflow/status/infinitehaiku/ih-muse/main.yml?branch=main)](https://github.com/infinitehaiku/ih-muse/actions/workflows/main.yml?query=branch%3Amain)
[![codecov](https://codecov.io/gh/infinitehaiku/ih-muse/branch/main/graph/badge.svg)](https://codecov.io/gh/infinitehaiku/ih-muse)
[![Commit activity](https://img.shields.io/github/commit-activity/m/infinitehaiku/ih-muse)](https://img.shields.io/github/commit-activity/m/infinitehaiku/ih-muse)
[![License](https://img.shields.io/github/license/infinitehaiku/ih-muse)](https://img.shields.io/github/license/infinitehaiku/ih-muse)

**IH-Muse** is designed for seamless integration with the IH system, providing efficient metrics tracking, element registration, and event recording capabilities in both **Python** and **Rust**.

- **Github repository**: <https://github.com/infinitehaiku/ih-muse/>
- **Documentation** <https://ih-muse.readthedocs.io/en/latest/>

## Key Features

- **Element Registration**: Easily register elements within the Muse system, including metadata and hierarchical relationships.
- **Metric Reporting**: Report metrics related to registered elements to monitor application and system performance.
- **Event Recording & Replaying**: Record events for later analysis or replay them for testing and debugging.
- **Multi-Client Configuration**: Supports different client types, including `Poet` (real-time interaction) and `Mock` (testing).
- **Asynchronous Operations**: Optimized for non-blocking, async-capable operations.
- **Multi-Language Support**: Available in both Python and Rust, with cross-language functionality and interoperability.
- **Extensible and Configurable**: Highly configurable to fit various use cases in monitoring, analytics, and diagnostics.

## Table of Contents

- [Installation](#installation)
- [Getting Started](#getting-started)
- [Configuration](#configuration)
- [Examples](#examples)
- [Contributing](#contributing)
- [License](#license)

## Installation

IH-Muse can be installed in Python and configured to work in conjunction with the Muse system. Rust users can include IH-Muse as a dependency in their Cargo project.

### Python Installation

Ensure you have `maturin` installed for building Rust extensions:

```bash
pip install maturin
```

Clone the repository and install IH-Muse with:

```bash
git clone https://github.com/infinitehaiku/ih-muse.git
cd ih-muse
make install
```

### Rust Installation

To add IH-Muse to your Rust project, use the following `cargo` command:

```bash
cargo add ih-muse
```

This will automatically add the latest version of IH-Muse to your `Cargo.toml` dependencies:

```toml
[dependencies]
ih-muse = "latest-compatible-version"
```

Alternatively, you can manually add it to your `Cargo.toml`:

```toml
[dependencies]
ih-muse = "*"
```

## Getting Started

### Python Example

```python
import asyncio
from ih_muse import Muse, Config, ClientType
from ih_muse.proto import ElementKindRegistration, MetricDefinition, TimestampResolution

async def main():
    config = Config(
        endpoints=["http://localhost:8080"],
        client_type=ClientType.Poet,
        default_resolution=TimestampResolution.Milliseconds,
        element_kinds=[ElementKindRegistration("kind_code", "description")],
        metric_definitions=[MetricDefinition("metric_code", "description")],
        max_reg_elem_retries=3,
        recording_enabled=False,
    )
    muse = Muse(config)
    await muse.initialize(timeout=5.0)
    local_elem_id = await muse.register_element("kind_code", "Element Name", metadata={}, parent_id=None)
    await muse.send_metric(local_elem_id, "metric_code", 42.0)

asyncio.run(main())
```

### Rust Example

```rust
use ih_muse::prelude::*;
use std::collections::HashMap;

#[tokio::main]
async fn main() -> MuseResult<()> {
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
    )?;

    let mut muse = Muse::new(&config)?;
    muse.initialize(Some(std::time::Duration::from_secs(5))).await?;
    let local_elem_id = muse.register_element("kind_code", "Element Name".to_string(), HashMap::new(), None).await?;
    muse.send_metric(local_elem_id, "metric_code", MetricValue::from(42.0)).await?;

    Ok(())
}
```

## Configuration

IH-Muse offers a rich set of configuration options to customize its behavior. Below are key parameters you can set:

- **endpoints**: A list of URLs for Muse services.
- **client_type**: Select between `Poet` (real client) or `Mock` (testing).
- **default_resolution**: Choose a default timestamp resolution (e.g., milliseconds).
- **element_kinds**: Register element types with the Muse system.
- **metric_definitions**: Define metrics for tracking.
- **recording_enabled**: Enable event recording.
- **recording_path**: Specify a file path if recording is enabled.

## Examples

IH-Muse is versatile for various use cases:

- **Monitoring Application Performance**: Track metrics and gain insights into application health.
- **Simulating Metrics and Events for Testing**: Use the `Mock` client to simulate system behavior.
- **Real-Time Data Collection**: Connect to the Muse system to collect live data for real-time analytics.

## Contributing

Contributions are welcome! Please see our [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines on how to contribute, report issues, and request features.

## License

IH-Muse is licensed under the [MIT License](LICENSE).
