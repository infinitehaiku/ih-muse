# IH Muse Protocol Fixtures

These compact files are reviewed protocol inputs and expected semantic outputs.
They are not deployment manifests, build artifacts, production captures, or a
test-time code generator cache.

| Directory | Contract and producer | Purpose | Human-review oracle |
| --- | --- | --- | --- |
| `telemetry-v2/` | `ih.telemetry.v2` profile | Accepted/rejected signal semantics | `profile.json` |
| `telemetry-v3/` | `ih.telemetry.v3`; Rust OpenTelemetry 0.32.0, Python SDK 1.41.1 | Real Python OTLP/HTTP-Protobuf metric/log/trace wires, including attempt zero | `task-retry.expected.json`, `task-retry.projected.json` |
| `rustvello-otel-v1/` | `ih.rustvello-otel.v1`; Rust OpenTelemetry 0.32.x and Python SDK 1.41.1 | Lifecycle/retry/trace mapping over captured Protobuf wires | `mapping.json`, `sdk.expected.json` |
| `graph-v1/` | `ih.graph.v1`; imports qualified `ih.telemetry.v3` | Canonical entities, temporal relations, observations/events, availability and safe multi-view accounting | `views.expected.json`, `task-retry.expected.json` |

The Python generator is `../generate_python_otlp.py`. It is a manual review
utility: it emits only loopback traffic and synthetic `fixture-token`,
`python-fixture`, and `fixture-1` values. Its package versions are declared in
`../otlp-python-requirements.txt`. Generate candidates outside this directory,
compare them with the reviewed files, then update every changed SHA-256 manifest
and readable oracle in the same review. Normal tests must never run the
generator or overwrite an expected output.

From the IH workspace root, verify the declared hashes and secret-free
provenance without a cluster, Docker, database, provider write, or generator:

```sh
make release-check-fixtures
```

The scanner rejects private-key and common cloud-secret material. Keep all
identities synthetic. Large compiler output, mutable soak captures, browser
recordings, private stores, OCI archives and retained runtime evidence belong in
ignored/protected evidence storage, not this fixture directory.
