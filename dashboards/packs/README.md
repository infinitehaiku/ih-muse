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
| `rustvello.json` | `otel.rustvello` | Rustvello's OTLP lifecycle export (`rustvello-otel`); Poet counts task states, run time, busy time, retries and worker starts from its records |
| `piceli.json` | `otel.piceli` | Piceli's GitOps controller (0.16 and later), its own OTLP |

The first seven packs have the same panels as Poet's built-in profiles of
the same purpose. An installed pack takes the place of that built-in profile.

## Bundled packs and event mappings

Poet bundles the Piceli pack: it compiles in `piceli.json` and
`event-mappings/piceli.json` and uses them for every tenant, so a cluster
whose Piceli controller sends OTLP to Poet gets the Deployments dashboard
and the deployment markers with nothing installed and no Muse running. A
stored definition or mapping with the same id and an equal or higher
revision takes the bundled one's place.

An event mapping (`ih_muse_proto::event_mapping`) names a system's own
OpenTelemetry in the shared deployment vocabulary: which resource is the
system's controller, which of its events and attributes are which
`deployment.*` names, and which root spans are releases. It is data: Poet's
CI/CD conventions do the rest. CI checks every file in `event-mappings/`
(`crates/ih-muse-cli/tests/dashboard_check.rs`).

| File | Id | System |
| --- | --- | --- |
| `event-mappings/piceli.json` | `piceli.deployments` | Piceli's controller (`service.name=piceli-controller`) |

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
