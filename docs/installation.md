# Installation

IH-Muse is available for both Rust and Python. Installation is straightforward using the respective package managers.

## Installing IH-Muse

Install Python

```bash
pip install ih-muse
```

Install Rust

```shell
cargo add ih-muse

# Or add it directly to your Cargo.toml
[dependencies]
ih-muse = "0.1.0"
```

## Importing

To use the library, import it into your project:

````{tab-set-code}
```{code-block} python
from ih_muse import Muse, Config
```

```{code-block} rust
use ih_muse::prelude::*;
```
````

## Feature Flags (Rust Only)

Depending on your use case, you might want to enable optional features to extend the functionality of IH-Muse in Rust. These are made optional to minimize the footprint.

### Optional Features

- `recording`: Enables event recording and replaying functionality.
- `cli`: Includes the command-line interface utilities.
- `mock-client`: Provides a mock client implementation for testing.
- `poet-client`: Enables the Poet client for communication with the Muse system.

To enable these features, specify them in your `Cargo.toml`:

```toml
[dependencies]
ih-muse = { version = "0.1.0", features = ["recording", "poet-client"] }
```

---

**Note:** Replace `"0.1.0"` with the actual version of IH-Muse you're using.
