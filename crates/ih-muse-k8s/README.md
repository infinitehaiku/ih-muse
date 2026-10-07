# ih-muse-k8s

The Kubernetes Muse. It runs as one pod in a cluster, reads the Kubernetes
API with its service account, and sends Poet one `ih.graph.v1` batch per
interval:

```text
Cluster -> Node                  (contains)
Cluster -> Namespace -> Pod      (contains; the namespace keyed by its UID)
Pod -> Node                      (scheduled_on)
Pod -> Container                 (contains)
```

Metrics (scope `ih.k8s.metrics` 2.0.0):

| Metric | Unit | On | Meaning |
| --- | --- | --- | --- |
| `k8s.cpu.usage` | cores (`1`) | node, pod, container (`k8s.level`) | metrics-server usage |
| `k8s.memory.usage` | bytes (`By`) | node, pod, container (`k8s.level`) | metrics-server usage |
| `k8s.pod.phase.running` | `1` | pod | 1 while Running |
| `k8s.pod.not_ready` | `1` | pod | 1 while a Running or Pending pod is not ready |
| `k8s.container.restarts` | `{restart}` | container | cumulative restarts (monotonic sum) |

A container that is not running, or any usage when metrics-server does not
answer, is sent as *missing* (`source_unavailable`), never as 0.

Phase A metrics (OpenTelemetry semantic-convention names; ratios in `1`):
`k8s.node.condition.status` (attributes `k8s.node.condition.type` and
`.status`, 1 for the current status), `k8s.node.cpu|memory.allocatable`,
`k8s.node.cpu|memory.utilization` (usage / allocatable),
`k8s.node.cpu|memory.request_utilization`,
`k8s.container.cpu|memory.request|limit`,
`k8s.container.cpu|memory.limit.utilization` (only with a limit),
`k8s.container.waiting` (attribute `k8s.container.status.reason`),
`k8s.container.oom_kills` (counted since the Muse first saw the container),
`k8s.pod.unschedulable`, `k8s.deployment.pod.desired|available`,
`k8s.statefulset.pod.desired|ready`. Workloads are entities too:
`Namespace -> Deployment/StatefulSet` (contains).

## Events

`src/events.rs` compares each collection with the previous one and sends
the problems and changes as `Domain` events with stable ids (restarts with
reason, exit code and signal, waiting reasons, image changes, unschedulable
pods, node conditions, rollouts with what changed, scaling, Kubernetes
Warning events). The kinds, ids and attributes, and how Poet serves them as
markers: `infinite-haiku` `docs/components/events.md`.

## Every namespace

`--all-namespaces` (`IH_K8S_ALL_NAMESPACES=true`) observes pods,
namespaces, Deployments, StatefulSets, ReplicaSets and Warning events in
every namespace (a ClusterRole with get, list and watch on pods, nodes,
namespaces, events, deployments, statefulsets and replicasets, plus
metrics-server's pods and nodes). Without it, the Muse reads its own
namespace as before.

It is a port of `ih-infra/scripts/k8s_muse.py`: the same identities, names,
units, attributes and availability, checked by
`tests/fixtures_parity.rs` against batches that script builds from the same
recorded API answers (`tests/fixtures/gen_expected.py` regenerates them).
Two differences: the producer id (below), and it lists nodes and pods every
interval instead of keeping a watch cache.

The Muse attaches its dashboard, `k8s.cluster` (`src/dashboards.rs`, same as
`examples/dashboards/k8s-cluster.json`), to every batch until a Poet
acknowledges one that carried it.

## Identity

- provenance `source_id`: `ih-k8s-muse:<instance>`, where `<instance>` is
  `--cluster-name` if set, else the API host;
- provenance `source_revision`: the crate version;
- delivery id: `ih-k8s-muse:<instance>:<observed-at ns>` (a retry to any
  Poet is stored once);
- the cluster root's identity is `{"kind": "cluster", "cluster_uid": ...}`
  from `--cluster-uid` (default: the instance).

Poet lists the producer as `ih-k8s-muse:<instance>@<version>`.

## Run in a cluster

```sh
ih-muse-k8s \
  --poet-url https://ih-poet:18443 --poet-ca-file /run/private/ca.crt \
  --poet-token-path /run/private/target-token \
  --organization ih-local --cluster-uid <cluster uid> --cluster-name k-lab \
  --interval-seconds 5
```

with `POD_NAMESPACE` from the downward API (`metadata.namespace`). The API
host comes from `KUBERNETES_SERVICE_HOST`/`KUBERNETES_SERVICE_PORT`, the
token and CA from `/var/run/secrets/kubernetes.io/serviceaccount` (the token
is read again on every request, so rotation works). Every flag also has an
environment variable (`--help`). `--poet-url` takes every Poet of the
cluster, comma-separated; the Muse fails over in that order. Plain HTTP is
accepted only for loopback; in a cluster use Poet's TLS port and its CA.
The Poet token file must not be world-readable.

`--api-url`, `--api-token-file` and `--api-ca-file` point it at another API
(tests use a loopback HTTP server). `--once` or `--samples N` stop after N
batches.

### While no Poet answers

The Muse keeps collecting and queues what it could not send, compressed
(an all-namespace k-lab batch is about 0.75 MB of JSON and 30 to 40 KB
queued). `--backlog-max-bytes` (`IH_MUSE_BACKLOG_MAX_BYTES`, default
16 MiB) bounds the queue: about 37 minutes of k-lab at 5 s before anything
is reduced. Beyond it the oldest data is thinned first (every second, then
every fourth, up to every 16th sample kept), then the oldest batches are
dropped. Batches carrying events are never thinned. Nothing is reduced
silently: a thinned batch's availability states the longer window it
stands for, its coarser resolution and its lower coverage (pressure policy
`ih.muse.backlog.thinned`); a dropped window has no availability, so Poet
shows it as unavailable, not as zero; and the next batch a Poet accepts
carries a `k8s.muse.backlog.reduced` warning event on the cluster with the
window and the intervals thinned and dropped (and their totals since
start).

When a Poet answers again the queue is replayed oldest first at
`--replay-batches-per-interval` (`IH_MUSE_REPLAY_BATCHES_PER_INTERVAL`,
default 4) batches per interval, so a 10 minute backlog drains in about
4 minutes without flooding Poet.

### RBAC

Read-only, the same grants as the Python Muse:

```yaml
# Role in the observed namespace
- apiGroups: [""]
  resources: ["pods"]
  verbs: ["get", "list", "watch"]
- apiGroups: ["metrics.k8s.io"]
  resources: ["pods"]
  verbs: ["get", "list"]
# ClusterRole
- apiGroups: [""]
  resources: ["nodes"]
  verbs: ["get", "list", "watch"]
- apiGroups: [""]
  resources: ["namespaces"]
  verbs: ["get"]
- apiGroups: ["metrics.k8s.io"]
  resources: ["nodes"]
  verbs: ["get", "list"]
```

(`watch` is not used by this Muse yet; it is kept so the same account serves
a later watch cache.)

## Test

```sh
cargo test -p ih-muse-k8s
```

No cluster is needed: the tests use the recorded answers in
`tests/fixtures/` and a loopback server.
