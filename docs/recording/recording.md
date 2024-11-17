# Recording Events

IH-Muse provides the ability to record events (element registrations and metric sends) to a file. This feature is useful for testing, debugging, and replaying events at a later time.

## Enabling Recording

To enable recording, you need to set `recording_enabled` to `true` in your `Config` and provide a `recording_path`.

=== ":fontawesome-brands-python: Python"

    ```python
    config = Config(
        # ... other configuration fields
        recording_enabled=True,
        recording_path="events.bin",  # Recording path
    )
    ```

=== ":fontawesome-brands-rust: Rust"

    ```rust
    let config = Config::new(
        // ... other configuration fields
        true, // recording_enabled
        Some("events.bin".to_string()), // recording_path
        // ... rest of the configuration
    )?;
    ```

## Serialization Format

The recording file should have a specific extension to determine the serialization format:

- `.bin` for Bincode serialization (binary format)
- `.json` for JSON serialization (text format)

**Example:**

- `events.bin` will use Bincode serialization.
- `events.json` will use JSON serialization.

## Using the Recorder

When recording is enabled, the Muse client automatically records events as you interact with it.

=== ":fontawesome-brands-python: Python"

    ```python
    # This will be recorded
    local_elem_id = await muse.register_element(
        kind_code="kind_code",
        name="Element Name",
        metadata={},
        parent_id=None,
    )

    # This will also be recorded
    await muse.send_metric(local_elem_id, "metric_code", 42.0)
    ```

=== ":fontawesome-brands-rust: Rust"

    ```rust
    // This will be recorded
    let local_elem_id = muse
        .register_element("kind_code", "Element Name".to_string(), HashMap::new(), None)
        .await?;

    // This will also be recorded
    muse.send_metric(local_elem_id, "metric_code", MetricValue::from(42.0))
        .await?;
    ```

The events are written to the specified recording file in the chosen serialization format.
