# Dashboard packs

A pack is a default dashboard for a source that has no Muse of its own: a
service that sends OpenTelemetry data (for example a PostgreSQL server read by
the OpenTelemetry Collector, or a Java service with the OpenTelemetry Java
agent). A Muse sends its own dashboards to Poet; a pack is the same kind of
file, installed into Poet by hand.

Each file here is one `DashboardDefinition` (see
`schemas/dashboard-definition.schema.json`):

- `id` is `otel.<system>` and `muse_kind` is `otel`;
- `applies_to.recognition` says which sources get the dashboard: metric name
  prefixes, root attributes or OpenTelemetry instrumentation scope names. A
  source matches when any one rule matches;
- `panels` are the charts, and `blocks` group them. Panels with a golden
  `signal` form the Measurements row at the top.

| File | Id | Source |
| --- | --- | --- |
| `postgresql.json` | `otel.postgresql` | Collector `postgresql` receiver |
| `mongodb.json` | `otel.mongodb` | Collector `mongodb` receiver |
| `jvm.json` | `otel.jvm` | OpenTelemetry Java agent |
| `rabbitmq.json` | `otel.rabbitmq` | `rabbitmq_prometheus` plugin scraped by the Collector |
| `otel-collector.json` | `otel.collector` | The Collector's own telemetry |
| `redis.json` | `otel.redis` | Redis INFO read beside the server |
| `rustvello.json` | `otel.rustvello` | Rustvello runner OTLP export |

The panels are the same as Poet's built-in profiles of the same purpose. An
installed pack takes the place of that built-in profile.

## Check a pack

```sh
ih-muse-cli dashboard check dashboards/packs/*.json
```

It prints `OK` or `ERROR` per file and exits with 1 if any file fails. CI
runs the same check over every pack (`crates/ih-muse-cli/tests/dashboard_check.rs`).

## Install a pack into Poet

With a running Poet and its operator token:

```sh
poet dashboards install --url http://poet:8000 \
  --operator-token-file /path/to/operator-token \
  dashboards/packs/postgresql.json dashboards/packs/jvm.json
```

The command sends each file to `POST /api/v1/dashboards/definitions`. Poet
stores it as data and copies it to the other Poets of its cluster. Sending
the same file again changes nothing. A file with the same `id` and `revision`
but other content is refused, and the first one stays. To change a pack,
raise its `revision`.
