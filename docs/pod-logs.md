# Pod and service logs

Infinite Haiku collects the logs of every pod on a Kubernetes node in one
of two ways. Pick one per node; running both sends every line twice (the
Poet stores identical records once, but it reads them twice).

1. **Plug and play: the node Muse.** The kernel Muse (`ih-muse-kernel`),
   which already runs on every node as a DaemonSet, also reads kubelet's
   pod log files and sends them to the Poets. Nothing new to deploy.
2. **Standard: your OpenTelemetry Collector.** If you already run a
   Collector on every node, point its logs pipeline at a Poet's OTLP
   (OpenTelemetry Protocol) intake with the configuration below.

Both send the same resource attributes, so the logs land in Logs & Traces
the same way and name the pod and container the Kubernetes Muse
describes.

## What is sent

One OTLP resource per container log file, with:

| Attribute | Value |
|---|---|
| `service.name` | the pod's workload: its name without the generated suffixes (`web-7d9f8b6c5d-x2x4z` → `web`, `poet-2` → `poet`) |
| `k8s.cluster.uid` | the same value as the Kubernetes Muse's `--cluster-uid` |
| `k8s.namespace.name`, `k8s.pod.name`, `k8s.pod.uid` | from the log directory `/var/log/pods/<namespace>_<pod>_<uid>/` |
| `k8s.container.name`, `k8s.container.restart_count` | from `<container>/<restart>.log` |
| `container.id` | from kubelet's `/var/log/containers/*-<id>.log` link, when present |
| `k8s.node.name`, `host.name` | the node |

Each record has the runtime's time for the line, the line as its body,
`log.iostream` (`stdout` or `stderr`), `log.file.name`, and a severity when
the line starts with or states a level (`ERROR …`, `level=warn`,
`"level":"info"`).

## The node Muse

Reads `/var/log/pods/*/*/*.log`:

- **Formats.** CRI (containerd, CRI-O) and Docker JSON. Lines the runtime
  split (`P` parts, or Docker lines without their newline) are joined.
  Indented lines (stack trace frames) join the line before them when both
  are read in the same pass. A line neither format reads is sent whole and
  counted.
- **Rotation and restarts.** A file is followed by its inode, so the rest
  of a rotated (renamed) file is read before the new one. A container
  restart starts `<restart+1>.log`, a resource of its own. A truncated
  file, or another file under a reused inode, is read again from its
  start. Compressed rotations (`.gz`) are never read.
- **Bounded and back-pressured.** One pass reads at most
  `IH_PODLOGS_MAX_BATCH_BYTES` (1 MiB) of new lines into one export, and
  the next pass waits until a Poet took it. A busy Poet's
  `429 Retry-After` is waited out; while no Poet takes the export, the
  unread lines stay in the files, not in memory. Bytes kubelet deleted
  before they were read are counted in the Muse's log.
- **Checkpoint.** The acknowledged offsets are saved to
  `IH_PODLOGS_CHECKPOINT_FILE`; a restart neither repeats nor skips a line
  (a line resent after a crash is stored once).

Settings (flag, environment variable, default):

| Setting | Default | Meaning |
|---|---|---|
| `--pod-logs`, `IH_PODLOGS` | `auto` | `on`, `off`, or `auto` (on when the directory exists) |
| `--pod-logs-dir`, `IH_PODLOGS_DIR` | `/var/log/pods` | kubelet's pod logs as mounted (read-only) |
| `--pod-logs-containers-dir`, `IH_PODLOGS_CONTAINERS_DIR` | `containers` beside it | kubelet's container links (container ids) |
| `--pod-logs-checkpoint-file`, `IH_PODLOGS_CHECKPOINT_FILE` | `/var/lib/ih/kernel-muse/pod-logs.json` | acknowledged offsets |
| `--pod-logs-start-at`, `IH_PODLOGS_START_AT` | `end` | first start without a checkpoint: `end` (new lines only) or `beginning` |
| `--pod-logs-include-namespaces`, `IH_PODLOGS_INCLUDE_NAMESPACES` | all | comma-separated patterns (`*`, `?`) |
| `--pod-logs-exclude-namespaces`, `IH_PODLOGS_EXCLUDE_NAMESPACES` | none | an exclusion wins |
| `--pod-logs-include-pods`, `IH_PODLOGS_INCLUDE_PODS` | none | `<namespace>/<pod>` patterns, added to the namespaces |
| `--pod-logs-exclude-pods`, `IH_PODLOGS_EXCLUDE_PODS` | none | `<namespace>/<pod>` patterns |
| `--pod-logs-interval-ms`, `IH_PODLOGS_INTERVAL_MS` | `2000` | how often the files are read |
| `--pod-logs-max-batch-bytes`, `IH_PODLOGS_MAX_BATCH_BYTES` | 1 MiB | one export (at most 3 MiB) |
| `--kernel-faults`, `IH_KERNEL_FAULTS` | `on` | `off` runs pod logs only |

The DaemonSet needs, besides what the kernel Muse already has:

- the host's `/var/log` mounted read-only (it is, for the journal), so
  `--pod-logs-dir /host/var/log/pods`;
- read access to the log files: kubelet writes them `root`-owned, mode
  `0640` on most distributions, so the container runs as user 0 with every
  capability dropped and a read-only root file system (it reads as the
  files' owner and can change nothing);
- the checkpoint directory writable (the kernel Muse's cursor volume).

## The OpenTelemetry Collector

For a Collector (the `otel/opentelemetry-collector-contrib` distribution,
0.100 or newer) running as a DaemonSet with `/var/log/pods` mounted
read-only. Replace the values in angle brackets; the token is a Poet
intake token with the `logs` scope, the CA the one that signs the Poet's
intake certificate.

```yaml
extensions:
  file_storage:                       # offsets survive restarts
    directory: /var/lib/otelcol/file_storage

receivers:
  filelog:
    include: [/var/log/pods/*/*/*.log]
    # exclude: [/var/log/pods/kube-system_*/*/*.log]
    start_at: end
    include_file_path: true           # the container parser reads the pod from it
    include_file_name: true           # log.file.name
    storage: file_storage
    operators:
      - type: container               # CRI and Docker JSON, partial lines,
        id: container-parser          # k8s.* from the path, log.iostream

processors:
  memory_limiter:
    check_interval: 1s
    limit_percentage: 80
  k8sattributes:                      # needs get/list/watch on pods and replicasets
    filter:
      node_from_env_var: K8S_NODE_NAME
    extract:
      metadata: [k8s.deployment.name, k8s.statefulset.name, k8s.daemonset.name,
                 k8s.job.name, k8s.node.name, container.id]
    pod_association:
      - sources: [{from: resource_attribute, name: k8s.pod.uid}]
  resource:
    attributes:
      - {key: k8s.cluster.uid, value: "<the Kubernetes Muse's --cluster-uid>", action: upsert}
  transform/service-name:             # service.name: the workload, as the node Muse names it
    log_statements:
      - context: resource
        statements:
          - set(attributes["service.name"], attributes["k8s.deployment.name"]) where attributes["service.name"] == nil
          - set(attributes["service.name"], attributes["k8s.statefulset.name"]) where attributes["service.name"] == nil
          - set(attributes["service.name"], attributes["k8s.daemonset.name"]) where attributes["service.name"] == nil
          - set(attributes["service.name"], attributes["k8s.job.name"]) where attributes["service.name"] == nil
          - set(attributes["service.name"], attributes["k8s.pod.name"]) where attributes["service.name"] == nil
  batch:
    send_batch_size: 1024
    send_batch_max_size: 2048         # well under the intake's 4 MiB per export

exporters:
  otlphttp/infinite-haiku:
    endpoint: https://<poet intake host>:<port>   # the Collector appends /v1/logs
    encoding: proto                   # the intake reads protobuf only
    compression: gzip
    headers:
      Authorization: "Bearer ${env:IH_POET_TOKEN}"
    tls:
      ca_file: /etc/ih/ca.crt
    retry_on_failure:                 # honours 429 Retry-After
      enabled: true
    sending_queue:
      enabled: true
      storage: file_storage

service:
  extensions: [file_storage]
  pipelines:
    logs:
      receivers: [filelog]
      processors: [memory_limiter, k8sattributes, resource, transform/service-name, batch]
      exporters: [otlphttp/infinite-haiku]
```

Tested on 2026-10-16 with `otel/opentelemetry-collector-contrib:0.115.1`
against a local Poet (without `k8sattributes`, which needs a Kubernetes
API): the lines arrive with the same resource as the node Muse's. The
Collector sets no severity by itself; add a `severity_parser` operator
after `container` when your lines state a level the same way.

With several Poets, list them behind one Service, or add one exporter per
Poet with the `failover` connector: any Poet of a cluster takes the logs
and the Poets replicate among themselves.
