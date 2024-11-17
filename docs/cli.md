# IH-Muse Command-Line Interface (CLI)

The IH-Muse CLI provides utilities for interacting with the Muse system directly from the command line. It includes commands for recording and replaying events, among other functionalities.

## Installation

To install the CLI tool, ensure that you have the `cli` feature enabled when building IH-Muse.

=== ":fontawesome-brands-python: Python"

    ```bash
    # Currently, the CLI is only available for Rust.
    # Python users can interact with IH-Muse programmatically.
    ```

=== ":fontawesome-brands-rust: Rust"

    ```shell
    cargo install ih-muse --features "cli"
    ```

## Available Commands

### `replay`

Replays events from a recording file to the Muse system.

**Usage:**

    ```shell
    ih-muse-cli replay --input <file> [OPTIONS]
    ```

**Options:**

- `-i, --input <file>`: Input file path containing the recorded events.
- `-u, --poet-url <url>`: Muse system endpoint URL (default: `http://localhost:8000`).

**Example:**

    ```shell
    ih-muse-cli replay --input events.bin --poet-url http://localhost:8000
    ```

This command replays events from `events.bin` to the Muse system at `http://localhost:8000`.

---

### Common Arguments

The CLI uses common arguments across commands for consistency.

**Common Arguments:**

- `-u, --poet-url <url>`: Muse system endpoint URL (default: `http://localhost:8000`).

**Example:**

    ```shell
    ih-muse-cli <command> --poet-url http://localhost:8000
    ```

---

## Future Enhancements

Additional commands and options will be added to the CLI tool in future releases to extend its capabilities.

---

For more detailed information on the available commands and options, use the `--help` flag:

    ```shell
    ih-muse-cli --help
    ```
