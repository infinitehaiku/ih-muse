# Welcome to IH-Muse Documentation

```{raw} html
<h1 style="text-align:center">IH-Muse: A Powerful Library for Muse Integration</h1>
<div align="center">
  <a href="https://docs.rs/ih-muse/latest/ih_muse/">
    <img src="https://docs.rs/ih-muse/badge.svg" alt="Rust docs latest"/>
  </a>
  <a href="https://crates.io/crates/ih-muse">
    <img src="https://img.shields.io/crates/v/ih-muse.svg" alt="Rust crates Latest Release"/>
  </a>
  <a href="https://pypi.org/project/ih-muse/">
    <img src="https://img.shields.io/pypi/v/ih-muse.svg" alt="PyPI Latest Release"/>
  </a>
  <a href="https://github.com/infinitehaiku/ih-muse">
    <img src="https://img.shields.io/github/v/release/infinitehaiku/ih-muse.svg" alt="GitHub Latest Release"/>
  </a>
</div>
```

```{toctree}
:hidden:
:maxdepth: 2
:caption: Table of Contents

installation
getting_started
configuration/index
recording/index
cli/index
api/index
contributing/index
faq
changelog
license
```

IH-Muse is a library designed to interact seamlessly with the Muse system. It provides functionality for element registration, metric reporting, and event recording/replaying, making it easier to integrate Muse capabilities into your applications in both Rust and Python.

## Key Features

- **Element Registration:** Easily register elements with the Muse system, including metadata and hierarchical relationships.
- **Metric Reporting:** Send metrics associated with elements to monitor performance and other metrics.
- **Event Recording & Replaying:** Record events for later analysis or replay them for testing and debugging purposes.
- **Configurable Client:** Choose between different client types (`Poet`, `Mock`) to suit your development and testing needs.
- **Asynchronous Operations:** Built with async capabilities, enabling efficient, non-blocking operations.
- **Multi-Language Support:** Available for both Rust and Python.

```{note}
**New to Muse?**

Muse is a system designed for real-time monitoring and analytics. It allows for the registration of elements and the reporting of metrics associated with those elements, providing insights into system performance and behavior.
```

## Philosophy

The goal of IH-Muse is to provide an efficient and user-friendly library that:

- Utilizes the performance and safety features of Rust and the simplicity of Python.
- Simplifies integration with the Muse system.
- Provides flexibility through configuration and client types.
- Enables easy recording and replaying of events for testing and analysis.

## Example

````{tab-set-code}

```{code-block} python
import asyncio
from ih_muse import Muse, Config, ClientType
from ih_muse.proto import (
    ElementKindRegistration,
    MetricDefinition,
    TimestampResolution,
)

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

    local_elem_id = await muse.register_element(
        kind_code="kind_code",
        name="Element Name",
        metadata={},
        parent_id=None,
    )

    await muse.send_metric(local_elem_id, "metric_code", 42.0)

asyncio.run(main())
```

```{code-block} rust
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

    let local_elem_id = muse
        .register_element(
            "kind_code",
            "Element Name".to_string(),
            HashMap::new(),
            None,
        )
        .await?;

    muse.send_metric(local_elem_id, "metric_code", MetricValue::from(42.0))
        .await?;

    Ok(())
}
```
````

A more extensive introduction can be found in the {doc}`getting_started`.

## Community

IH-Muse is an open-source project, and we welcome contributions from the community. Join us on [GitHub](https://github.com/your-username/ih-muse) to contribute, report issues, or request features.

## Contributing

We appreciate all contributions, from reporting bugs to implementing new features. Read our {doc}`contributing/index` to learn more.

## License

This project is licensed under the terms of the [MIT license](https://github.com/your-username/ih-muse/blob/main/LICENSE).
