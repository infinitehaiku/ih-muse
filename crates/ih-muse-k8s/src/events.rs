//! Problems and changes, detected between two observations of the cluster.
//!
//! [`Detector`] keeps what the previous collection saw and turns each new
//! [`Snapshot`] into [`DetectedEvent`]s: container restarts (with the last
//! termination reason and exit code), waiting reasons appearing, image
//! changes, unschedulable pods, node condition changes, rollouts of
//! Deployments and StatefulSets (start, finish, scale) and Kubernetes
//! `Warning` events.
//!
//! Every event id is derived from what the API says, never from the
//! collection time, so a resent batch, a second Muse or a restarted Muse
//! names the same change with the same id and Poet stores it once:
//!
//! | Event | Id |
//! | --- | --- |
//! | restart | `restart/{pod uid}/{container}/{restart count}` |
//! | waiting | `waiting/{pod uid}/{container}/{reason}/{restart count}` |
//! | image change | `image/{pod uid}/{container}/{image id or image}` |
//! | unschedulable | `unschedulable/{pod uid}/{transition time}` |
//! | node condition | `node/{node uid}/{type}/{status}/{transition time}` |
//! | rollout | `rollout/{workload uid}/{revision}/start` or `/finish` |
//! | scale | `scale/{workload uid}/{generation}` |
//! | warning | `warning/{event uid}/{15-minute bucket of its last time}` |
//!
//! The graph prefixes each id with the cluster UID.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde_json::Value;

use crate::graph::Snapshot;
use crate::model::{ContainerStatus, Deployment, KubeEvent, Pod, ReplicaSet};

/// `event.name` of a container restart.
pub const RESTART: &str = "k8s.container.restart";
/// `event.name` of a container starting to wait for a problem reason.
pub const WAITING: &str = "k8s.container.waiting";
/// `event.name` of a container's image or digest changing.
pub const IMAGE_CHANGE: &str = "k8s.container.image.change";
/// `event.name` of a pod that cannot be scheduled.
pub const UNSCHEDULABLE: &str = "k8s.pod.unschedulable";
/// `event.name` of a node condition changing.
pub const NODE_CONDITION: &str = "k8s.node.condition.change";
/// `event.name` of a Deployment or StatefulSet starting a new revision.
pub const ROLLOUT_START: &str = "k8s.rollout.start";
/// `event.name` of a rollout whose pods all run the new revision.
pub const ROLLOUT_FINISH: &str = "k8s.rollout.finish";
/// `event.name` of a workload's desired replicas changing.
pub const SCALE: &str = "k8s.workload.scale";
/// `event.name` of a Kubernetes `Warning` event (the Events API).
pub const WARNING: &str = "k8s.warning";

/// Waiting reasons that are part of a normal start, never reported.
const BENIGN_WAITING: [&str; 2] = ["ContainerCreating", "PodInitializing"];
/// Node conditions reported; `Ready` is healthy when `True`, the others
/// when `False`.
pub(crate) const NODE_CONDITIONS: [&str; 5] = [
    "Ready",
    "MemoryPressure",
    "DiskPressure",
    "PIDPressure",
    "NetworkUnavailable",
];
/// A container running this long has left any crash loop (the kubelet's
/// back-off tops out at 5 minutes), so a new waiting reason is news again.
const STABLE_RUN_NS: u64 = 10 * 60 * 1_000_000_000;
/// On the first look, a restart that ended earlier than this is history,
/// not news.
const RECENT_NS: u64 = 60 * 60 * 1_000_000_000;
/// Warning events of one Kubernetes Event are reported at most once per
/// bucket while it keeps repeating.
const WARNING_BUCKET_NS: u64 = 15 * 60 * 1_000_000_000;
/// Warning ids remembered (to send each once) for this long.
const WARNING_MEMORY_NS: u64 = 3 * 60 * 60 * 1_000_000_000;
/// Longest message kept from the API.
const MAX_MESSAGE_CHARS: usize = 1024;

/// How serious an event is (`ih.event.severity`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

/// The element an event is about. The graph keys it; a target that is not
/// in the batch (a deleted pod) falls back to its namespace, then the
/// cluster.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Target {
    Pod {
        uid: String,
        namespace: String,
    },
    Node {
        uid: String,
    },
    /// A Deployment or StatefulSet (`kind` is `deployment` or `statefulset`).
    Workload {
        kind: &'static str,
        uid: String,
        namespace: String,
    },
    Namespace {
        name: String,
    },
    Cluster,
}

/// One problem or change, ready to become a GraphBatch event.
#[derive(Clone, Debug, PartialEq)]
pub struct DetectedEvent {
    /// Stable id (module docs); the graph prefixes the cluster UID.
    pub id: String,
    /// `event.name` (`k8s.container.restart`, ...).
    pub name: &'static str,
    pub target: Target,
    /// When it happened per the API, else when it was first seen.
    pub time_unix_nano: u64,
    pub severity: Severity,
    /// `k8s.*` names of the elements and the details (reason, exit code...).
    pub attributes: BTreeMap<String, String>,
    /// One line for people: `piceli-system/piceli-gitops-7f9c gitops restarted: Error (exit 1), 37 restarts`.
    pub summary: String,
}

/// What the previous collection saw of one container.
#[derive(Clone, Debug, Default)]
struct ContainerSeen {
    restart_count: i64,
    image: String,
    image_id: String,
    /// The waiting reason already reported, until the container runs stably.
    waiting_reported: Option<String>,
}

/// What the previous collection saw of one workload.
#[derive(Clone, Debug, Default)]
struct WorkloadSeen {
    revision: String,
    desired: i64,
    /// The revision whose rollout was seen in progress (a finish follows).
    rolling: Option<String>,
    template: Value,
}

/// OOMKills counted for one container since the Muse first saw it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OomCounter {
    pub kills: i64,
    pub since_unix_nano: u64,
}

/// The state kept between collections. A fresh detector (a restarted Muse)
/// reports what is wrong now and recent restarts; the stable ids make the
/// repeats harmless.
#[derive(Debug, Default)]
pub struct Detector {
    containers: HashMap<(String, String), ContainerSeen>,
    unschedulable: HashSet<String>,
    nodes: HashMap<(String, String), String>,
    workloads: HashMap<String, WorkloadSeen>,
    warnings: HashMap<String, u64>,
    oom_kills: HashMap<(String, String), OomCounter>,
}

fn unix_nano(text: Option<&str>) -> Option<u64> {
    let time = chrono::DateTime::parse_from_rfc3339(text?).ok()?;
    u64::try_from(time.timestamp_nanos_opt()?).ok()
}

fn clip(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= MAX_MESSAGE_CHARS {
        return text.to_string();
    }
    let mut clipped: String = text.chars().take(MAX_MESSAGE_CHARS).collect();
    clipped.push('…');
    clipped
}

fn attrs<const N: usize>(pairs: [(&str, String); N]) -> BTreeMap<String, String> {
    pairs
        .into_iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(name, value)| (name.to_string(), value))
        .collect()
}

impl Detector {
    pub fn new() -> Self {
        Self::default()
    }

    /// OOMKills counted per `(pod uid, container)` (the
    /// `k8s.container.oom_kills` counter).
    pub fn oom_kills(&self, pod_uid: &str, container: &str) -> Option<OomCounter> {
        self.oom_kills
            .get(&(pod_uid.to_string(), container.to_string()))
            .copied()
    }

    /// The events between the previous snapshot and `snapshot`, observed at
    /// `now`; remembers `snapshot` for the next call.
    pub fn detect(&mut self, snapshot: &Snapshot, now: u64) -> Vec<DetectedEvent> {
        let mut events = Vec::new();
        let mut live_containers = HashSet::new();
        let mut live_pods = HashSet::new();
        for pod in &snapshot.pods {
            live_pods.insert(pod.metadata.uid.clone());
            self.pod(pod, now, &mut events);
            for container in &pod.status.container_statuses {
                live_containers.insert((pod.metadata.uid.clone(), container.name.clone()));
                self.container(pod, container, now, &mut events);
            }
        }
        // Forget what is gone, so a long-running Muse stays small.
        self.containers
            .retain(|key, _| live_containers.contains(key));
        self.oom_kills
            .retain(|key, _| live_containers.contains(key));
        self.unschedulable.retain(|uid| live_pods.contains(uid));

        let mut live_nodes = HashSet::new();
        for node in &snapshot.nodes {
            for condition in &node.status.conditions {
                if !NODE_CONDITIONS.contains(&condition.kind.as_str()) {
                    continue;
                }
                let key = (node.metadata.uid.clone(), condition.kind.clone());
                live_nodes.insert(key.clone());
                let healthy = if condition.kind == "Ready" {
                    condition.status == "True"
                } else {
                    condition.status == "False"
                };
                let previous = self.nodes.insert(key, condition.status.clone());
                let report = match &previous {
                    Some(previous) => *previous != condition.status,
                    None => !healthy,
                };
                if !report {
                    continue;
                }
                let since = condition.last_transition_time.clone().unwrap_or_default();
                let severity = match (healthy, condition.kind.as_str()) {
                    (true, _) => Severity::Info,
                    (false, "Ready") => Severity::Error,
                    (false, _) => Severity::Warning,
                };
                let node_name = &node.metadata.name;
                events.push(DetectedEvent {
                    id: format!(
                        "node/{}/{}/{}/{since}",
                        node.metadata.uid, condition.kind, condition.status
                    ),
                    name: NODE_CONDITION,
                    target: Target::Node {
                        uid: node.metadata.uid.clone(),
                    },
                    time_unix_nano: unix_nano(Some(&since)).unwrap_or(now),
                    severity,
                    attributes: attrs([
                        ("k8s.node.name", node_name.clone()),
                        ("k8s.node.condition.type", condition.kind.clone()),
                        ("k8s.node.condition.status", condition.status.clone()),
                        ("k8s.node.condition.previous", previous.unwrap_or_default()),
                        (
                            "k8s.event.reason",
                            condition.reason.clone().unwrap_or_default(),
                        ),
                        (
                            "k8s.event.message",
                            clip(condition.message.as_deref().unwrap_or_default()),
                        ),
                    ]),
                    summary: format!(
                        "node {node_name} {}={}{}",
                        condition.kind,
                        condition.status,
                        condition
                            .reason
                            .as_deref()
                            .map(|reason| format!(" ({reason})"))
                            .unwrap_or_default()
                    ),
                });
            }
        }
        self.nodes.retain(|key, _| live_nodes.contains(key));

        let mut live_workloads = HashSet::new();
        for deployment in &snapshot.deployments {
            live_workloads.insert(deployment.metadata.uid.clone());
            self.workload(
                "deployment",
                deployment,
                &snapshot.replicasets,
                now,
                &mut events,
            );
        }
        for statefulset in &snapshot.statefulsets {
            live_workloads.insert(statefulset.metadata.uid.clone());
            self.workload("statefulset", statefulset, &[], now, &mut events);
        }
        self.workloads.retain(|uid, _| live_workloads.contains(uid));

        for warning in snapshot.warnings.iter().flatten() {
            self.warning(snapshot, warning, now, &mut events);
        }
        self.warnings
            .retain(|_, seen| now.saturating_sub(*seen) < WARNING_MEMORY_NS);

        events.sort_by(|left, right| {
            (left.time_unix_nano, &left.id).cmp(&(right.time_unix_nano, &right.id))
        });
        events
    }

    fn pod(&mut self, pod: &Pod, now: u64, events: &mut Vec<DetectedEvent>) {
        let uid = &pod.metadata.uid;
        let scheduling = pod.status.conditions.iter().find(|condition| {
            condition.kind == "PodScheduled"
                && condition.status == "False"
                && condition.reason.as_deref() == Some("Unschedulable")
        });
        let Some(condition) = scheduling else {
            self.unschedulable.remove(uid);
            return;
        };
        if !self.unschedulable.insert(uid.clone()) {
            return;
        }
        let since = condition.last_transition_time.clone().unwrap_or_default();
        let message = clip(condition.message.as_deref().unwrap_or_default());
        events.push(DetectedEvent {
            id: format!("unschedulable/{uid}/{since}"),
            name: UNSCHEDULABLE,
            target: pod_target(pod),
            time_unix_nano: unix_nano(Some(&since)).unwrap_or(now),
            severity: Severity::Warning,
            attributes: attrs([
                ("k8s.namespace.name", pod.metadata.namespace.clone()),
                ("k8s.pod.name", pod.metadata.name.clone()),
                ("k8s.event.reason", "Unschedulable".to_string()),
                ("k8s.event.message", message.clone()),
            ]),
            summary: format!("{} unschedulable: {message}", pod_label(pod)),
        });
    }

    fn container(
        &mut self,
        pod: &Pod,
        container: &ContainerStatus,
        now: u64,
        events: &mut Vec<DetectedEvent>,
    ) {
        let uid = &pod.metadata.uid;
        let name = &container.name;
        let key = (uid.clone(), name.clone());
        let previous = self.containers.get(&key).cloned();
        let mut seen = previous.clone().unwrap_or_default();
        let base = || {
            vec![
                ("k8s.namespace.name", pod.metadata.namespace.clone()),
                ("k8s.pod.name", pod.metadata.name.clone()),
                ("k8s.container.name", name.clone()),
                (
                    "k8s.container.restart_count",
                    container.restart_count.to_string(),
                ),
                (
                    "k8s.node.name",
                    pod.spec.node_name.clone().unwrap_or_default(),
                ),
            ]
            .into_iter()
            .filter(|(_, value)| !value.is_empty())
            .collect::<Vec<_>>()
        };

        // Restarts, with how the container last ended.
        let termination = container.last_termination();
        let finished = termination
            .as_ref()
            .and_then(|termination| unix_nano(termination.finished_at.as_deref()));
        let restarted = match &previous {
            Some(previous) => container.restart_count > previous.restart_count,
            None => {
                container.restart_count > 0
                    && finished.is_some_and(|finished| now.saturating_sub(finished) < RECENT_NS)
            }
        };
        if restarted {
            let termination = termination.clone().unwrap_or_default();
            let reason = termination.reason.clone().unwrap_or_default();
            let oom = reason == "OOMKilled";
            let signal = termination.exit_code.and_then(exit_signal);
            // A crash by memory or instruction fault hints at the node's
            // hardware as much as at the program: always an error.
            let fault = matches!(signal, Some("SIGSEGV" | "SIGBUS" | "SIGILL"));
            if oom {
                let counter = self.oom_kills.entry(key.clone()).or_insert(OomCounter {
                    kills: 0,
                    since_unix_nano: now,
                });
                counter.kills += 1;
            }
            let exit = termination
                .exit_code
                .map(|code| code.to_string())
                .unwrap_or_default();
            let mut attributes: BTreeMap<String, String> = base()
                .into_iter()
                .map(|(name, value)| (name.to_string(), value))
                .collect();
            attributes.extend(attrs([
                ("k8s.container.last_terminated.reason", reason.clone()),
                ("k8s.container.last_terminated.exit_code", exit.clone()),
                (
                    "k8s.container.last_terminated.signal",
                    signal.unwrap_or_default().to_string(),
                ),
                ("k8s.event.reason", reason.clone()),
            ]));
            let exit = match signal {
                Some(signal) => format!("{exit}, {signal}"),
                None => exit,
            };
            let how = match (reason.is_empty(), exit.is_empty()) {
                (false, false) => format!(": {reason} (exit {exit})"),
                (false, true) => format!(": {reason}"),
                (true, false) => format!(": exit {exit}"),
                (true, true) => String::new(),
            };
            events.push(DetectedEvent {
                id: format!("restart/{uid}/{name}/{}", container.restart_count),
                name: RESTART,
                target: pod_target(pod),
                time_unix_nano: finished.unwrap_or(now).min(now),
                severity: if oom || fault {
                    Severity::Error
                } else {
                    Severity::Warning
                },
                attributes,
                summary: format!(
                    "{} {name} restarted{how}, {} restart{}",
                    pod_label(pod),
                    container.restart_count,
                    if container.restart_count == 1 {
                        ""
                    } else {
                        "s"
                    }
                ),
            });
        }
        self.oom_kills.entry(key.clone()).or_insert(OomCounter {
            kills: 0,
            since_unix_nano: now,
        });

        // A problem waiting reason, once until the container runs stably.
        match container
            .waiting_reason()
            .filter(|reason| !BENIGN_WAITING.contains(reason))
        {
            Some(reason) if seen.waiting_reported.as_deref() != Some(reason) => {
                let message = clip(container.waiting_message().unwrap_or_default());
                let mut attributes: BTreeMap<String, String> = base()
                    .into_iter()
                    .map(|(name, value)| (name.to_string(), value))
                    .collect();
                attributes.extend(attrs([
                    ("k8s.container.status.reason", reason.to_string()),
                    ("k8s.event.reason", reason.to_string()),
                    ("k8s.event.message", message.clone()),
                ]));
                if let Some(termination) = &termination {
                    attributes.extend(attrs([
                        (
                            "k8s.container.last_terminated.reason",
                            termination.reason.clone().unwrap_or_default(),
                        ),
                        (
                            "k8s.container.last_terminated.exit_code",
                            termination
                                .exit_code
                                .map(|code| code.to_string())
                                .unwrap_or_default(),
                        ),
                    ]));
                }
                events.push(DetectedEvent {
                    id: format!("waiting/{uid}/{name}/{reason}/{}", container.restart_count),
                    name: WAITING,
                    target: pod_target(pod),
                    time_unix_nano: now,
                    severity: if reason == "CrashLoopBackOff" {
                        Severity::Error
                    } else {
                        Severity::Warning
                    },
                    attributes,
                    summary: format!(
                        "{} {name} {reason}, {} restart{}",
                        pod_label(pod),
                        container.restart_count,
                        if container.restart_count == 1 {
                            ""
                        } else {
                            "s"
                        }
                    ),
                });
                seen.waiting_reported = Some(reason.to_string());
            }
            Some(_) => {}
            None => {
                let started = container
                    .state
                    .as_ref()
                    .and_then(|state| state.get("running"))
                    .and_then(|running| running.get("startedAt"))
                    .and_then(Value::as_str);
                let stable = container.ready
                    && unix_nano(started)
                        .is_some_and(|started| now.saturating_sub(started) >= STABLE_RUN_NS);
                if stable {
                    seen.waiting_reported = None;
                }
            }
        }

        // The image digest changed in place (a new pod is a rollout). The
        // status `image` alone is not compared: the runtime rewrites it once
        // the image is pulled (a tag becomes `sha256:...`).
        if let Some(previous) = &previous {
            let digest_changed = !container.image_id.is_empty()
                && !previous.image_id.is_empty()
                && container.image_id != previous.image_id;
            if digest_changed {
                let mut attributes: BTreeMap<String, String> = base()
                    .into_iter()
                    .map(|(name, value)| (name.to_string(), value))
                    .collect();
                attributes.extend(attrs([
                    ("container.image.name", container.image.clone()),
                    ("container.image.id", container.image_id.clone()),
                    ("container.image.previous.name", previous.image.clone()),
                    ("container.image.previous.id", previous.image_id.clone()),
                ]));
                let identity = if container.image_id.is_empty() {
                    &container.image
                } else {
                    &container.image_id
                };
                events.push(DetectedEvent {
                    id: format!("image/{uid}/{name}/{identity}"),
                    name: IMAGE_CHANGE,
                    target: pod_target(pod),
                    time_unix_nano: now,
                    severity: Severity::Info,
                    attributes,
                    summary: format!(
                        "{} {name} image {} → {}",
                        pod_label(pod),
                        short_image(&previous.image, &previous.image_id),
                        short_image(&container.image, &container.image_id)
                    ),
                });
            }
        }

        seen.restart_count = container.restart_count;
        if !container.image.is_empty() {
            seen.image = container.image.clone();
        }
        if !container.image_id.is_empty() {
            seen.image_id = container.image_id.clone();
        }
        self.containers.insert(key, seen);
    }

    fn workload(
        &mut self,
        kind: &'static str,
        workload: &Deployment,
        replicasets: &[ReplicaSet],
        now: u64,
        events: &mut Vec<DetectedEvent>,
    ) {
        let meta = &workload.metadata;
        let status = &workload.status;
        let desired = workload.spec.replicas.unwrap_or(1);
        let revision = match kind {
            "deployment" => meta
                .annotations
                .get("deployment.kubernetes.io/revision")
                .cloned()
                .unwrap_or_default(),
            _ => status.update_revision.clone().unwrap_or_default(),
        };
        let observed = status.observed_generation >= meta.generation;
        let complete = observed
            && match kind {
                "deployment" => {
                    status.updated_replicas >= desired
                        && status.replicas <= status.updated_replicas
                        && status.available_replicas >= desired
                }
                _ => {
                    status.current_revision == status.update_revision
                        && status.updated_replicas >= desired
                        && status.ready_replicas >= desired
                }
            };
        let target = Target::Workload {
            kind,
            uid: meta.uid.clone(),
            namespace: meta.namespace.clone(),
        };
        let label = format!("{}/{}", meta.namespace, meta.name);
        let name_attribute = if kind == "deployment" {
            "k8s.deployment.name"
        } else {
            "k8s.statefulset.name"
        };
        let previous = self.workloads.get(&meta.uid).cloned();
        let mut seen = previous.clone().unwrap_or_default();

        let new_revision = match &previous {
            Some(previous) => !revision.is_empty() && revision != previous.revision,
            None => !complete && !revision.is_empty(),
        };
        if new_revision {
            let old_template = old_template(kind, workload, replicasets, &revision)
                .or_else(|| previous.as_ref().map(|previous| previous.template.clone()))
                .filter(|template| !template.is_null());
            let changes = old_template
                .as_ref()
                .map(|old| template_changes(old, &workload.spec.template))
                .unwrap_or_default();
            let categories: BTreeSet<&str> = changes.iter().map(|change| change.category).collect();
            let started = (kind == "deployment")
                .then(|| revision_replicaset(workload, replicasets, &revision))
                .flatten()
                .and_then(|replicaset| {
                    unix_nano(replicaset.metadata.creation_timestamp.as_deref())
                });
            let details: Vec<String> = changes.iter().map(|change| change.detail.clone()).collect();
            events.push(DetectedEvent {
                id: format!("rollout/{}/{revision}/start", meta.uid),
                name: ROLLOUT_START,
                target: target.clone(),
                time_unix_nano: started.unwrap_or(now).min(now),
                severity: Severity::Info,
                attributes: attrs([
                    ("k8s.namespace.name", meta.namespace.clone()),
                    (name_attribute, meta.name.clone()),
                    ("k8s.workload.kind", kind.to_string()),
                    ("k8s.rollout.revision", revision.clone()),
                    (
                        "k8s.rollout.previous_revision",
                        previous
                            .as_ref()
                            .map(|previous| previous.revision.clone())
                            .unwrap_or_default(),
                    ),
                    (
                        "k8s.rollout.changes",
                        categories.iter().copied().collect::<Vec<_>>().join(","),
                    ),
                    ("k8s.rollout.details", clip(&details.join("; "))),
                    (
                        "container.image.name",
                        images(&workload.spec.template).join(","),
                    ),
                ]),
                summary: format!(
                    "{kind} {label} rollout to revision {}{}",
                    short_revision(&revision),
                    if details.is_empty() {
                        String::new()
                    } else {
                        format!(": {}", details.join("; "))
                    }
                ),
            });
            seen.rolling = Some(revision.clone());
        }
        if complete && seen.rolling.as_deref() == Some(revision.as_str()) {
            let finished = workload
                .status
                .conditions
                .iter()
                .find(|condition| {
                    condition.kind == "Progressing"
                        && condition.reason.as_deref() == Some("NewReplicaSetAvailable")
                })
                .and_then(|condition| unix_nano(condition.last_update_time.as_deref()));
            events.push(DetectedEvent {
                id: format!("rollout/{}/{revision}/finish", meta.uid),
                name: ROLLOUT_FINISH,
                target: target.clone(),
                time_unix_nano: finished.unwrap_or(now).min(now),
                severity: Severity::Info,
                attributes: attrs([
                    ("k8s.namespace.name", meta.namespace.clone()),
                    (name_attribute, meta.name.clone()),
                    ("k8s.workload.kind", kind.to_string()),
                    ("k8s.rollout.revision", revision.clone()),
                ]),
                summary: format!(
                    "{kind} {label} revision {} rolled out ({desired} desired, {} available)",
                    short_revision(&revision),
                    if kind == "deployment" {
                        status.available_replicas
                    } else {
                        status.ready_replicas
                    }
                ),
            });
            seen.rolling = None;
        }
        if let Some(previous) = &previous {
            if previous.desired != desired {
                events.push(DetectedEvent {
                    id: format!("scale/{}/{}", meta.uid, meta.generation),
                    name: SCALE,
                    target,
                    time_unix_nano: now,
                    severity: Severity::Info,
                    attributes: attrs([
                        ("k8s.namespace.name", meta.namespace.clone()),
                        (name_attribute, meta.name.clone()),
                        ("k8s.workload.kind", kind.to_string()),
                        ("k8s.workload.desired", desired.to_string()),
                        (
                            "k8s.workload.previous_desired",
                            previous.desired.to_string(),
                        ),
                    ]),
                    summary: format!("{kind} {label} scaled {} → {desired}", previous.desired),
                });
            }
        }
        seen.revision = revision;
        seen.desired = desired;
        seen.template = workload.spec.template.clone();
        self.workloads.insert(meta.uid.clone(), seen);
    }

    fn warning(
        &mut self,
        snapshot: &Snapshot,
        warning: &KubeEvent,
        now: u64,
        events: &mut Vec<DetectedEvent>,
    ) {
        if warning.kind != "Warning" || warning.metadata.uid.is_empty() {
            return;
        }
        let last = warning
            .series
            .as_ref()
            .and_then(|series| unix_nano(series.last_observed_time.as_deref()))
            .or_else(|| unix_nano(warning.last_timestamp.as_deref()))
            .or_else(|| unix_nano(warning.event_time.as_deref()))
            .or_else(|| unix_nano(warning.first_timestamp.as_deref()))
            .unwrap_or(now)
            .min(now);
        let id = format!(
            "warning/{}/{}",
            warning.metadata.uid,
            last / WARNING_BUCKET_NS
        );
        if self.warnings.contains_key(&id) {
            return;
        }
        self.warnings.insert(id.clone(), now);
        let object = &warning.involved_object;
        let namespace = if object.namespace.is_empty() {
            warning.metadata.namespace.clone()
        } else {
            object.namespace.clone()
        };
        let target = match object.kind.as_str() {
            "Pod" => Target::Pod {
                uid: object.uid.clone(),
                namespace: namespace.clone(),
            },
            "Node" => snapshot
                .nodes
                .iter()
                .find(|node| node.metadata.uid == object.uid || node.metadata.name == object.name)
                .map_or(Target::Cluster, |node| Target::Node {
                    uid: node.metadata.uid.clone(),
                }),
            "Deployment" => Target::Workload {
                kind: "deployment",
                uid: object.uid.clone(),
                namespace: namespace.clone(),
            },
            "StatefulSet" => Target::Workload {
                kind: "statefulset",
                uid: object.uid.clone(),
                namespace: namespace.clone(),
            },
            _ if !namespace.is_empty() => Target::Namespace {
                name: namespace.clone(),
            },
            _ => Target::Cluster,
        };
        let count = warning
            .series
            .as_ref()
            .map(|series| series.count)
            .or(warning.count)
            .unwrap_or(1);
        let message = clip(&warning.message);
        let element = if namespace.is_empty() {
            format!("{} {}", object.kind.to_lowercase(), object.name)
        } else {
            format!("{namespace}/{}", object.name)
        };
        let mut attributes = attrs([
            ("k8s.namespace.name", namespace.clone()),
            ("k8s.event.reason", warning.reason.clone()),
            ("k8s.event.message", message.clone()),
            ("k8s.event.count", count.to_string()),
            ("k8s.object.kind", object.kind.clone()),
            ("k8s.object.name", object.name.clone()),
        ]);
        match object.kind.as_str() {
            "Pod" => {
                attributes.insert("k8s.pod.name".into(), object.name.clone());
            }
            "Node" => {
                attributes.insert("k8s.node.name".into(), object.name.clone());
            }
            "Deployment" => {
                attributes.insert("k8s.deployment.name".into(), object.name.clone());
            }
            "StatefulSet" => {
                attributes.insert("k8s.statefulset.name".into(), object.name.clone());
            }
            _ => {}
        }
        events.push(DetectedEvent {
            id,
            name: WARNING,
            target,
            time_unix_nano: last,
            severity: Severity::Warning,
            attributes,
            summary: format!(
                "{element} {}: {message}{}",
                warning.reason,
                if count > 1 {
                    format!(" (×{count})")
                } else {
                    String::new()
                }
            ),
        });
    }
}

/// The signal a container exit code `128 + n` reports (`139` is `SIGSEGV`).
pub fn exit_signal(code: i64) -> Option<&'static str> {
    Some(match code {
        130 => "SIGINT",
        132 => "SIGILL",
        133 => "SIGTRAP",
        134 => "SIGABRT",
        135 => "SIGBUS",
        136 => "SIGFPE",
        137 => "SIGKILL",
        139 => "SIGSEGV",
        141 => "SIGPIPE",
        143 => "SIGTERM",
        _ => return None,
    })
}

fn pod_target(pod: &Pod) -> Target {
    Target::Pod {
        uid: pod.metadata.uid.clone(),
        namespace: pod.metadata.namespace.clone(),
    }
}

fn pod_label(pod: &Pod) -> String {
    if pod.metadata.namespace.is_empty() {
        pod.metadata.name.clone()
    } else {
        format!("{}/{}", pod.metadata.namespace, pod.metadata.name)
    }
}

/// `registry/repo:tag` and a short digest, for one line of text.
fn short_image(image: &str, image_id: &str) -> String {
    let digest = image_id
        .rsplit_once("sha256:")
        .map(|(_, digest)| &digest[..digest.len().min(12)]);
    match digest {
        Some(digest) if !image.contains(digest) => format!("{image}@{digest}"),
        _ => image.to_string(),
    }
}

/// A StatefulSet revision is `name-<hash>`; the hash is what people compare.
fn short_revision(revision: &str) -> &str {
    revision.rsplit('-').next().unwrap_or(revision)
}

/// The ReplicaSet of `workload` at `revision`.
fn revision_replicaset<'a>(
    workload: &'a Deployment,
    replicasets: &'a [ReplicaSet],
    revision: &str,
) -> Option<&'a ReplicaSet> {
    owned(workload, replicasets).find(|replicaset| replica_revision(replicaset) == Some(revision))
}

fn owned<'a>(
    workload: &'a Deployment,
    replicasets: &'a [ReplicaSet],
) -> impl Iterator<Item = &'a ReplicaSet> {
    replicasets.iter().filter(move |replicaset| {
        replicaset
            .metadata
            .owner_references
            .iter()
            .any(|owner| owner.uid == workload.metadata.uid)
    })
}

fn replica_revision(replicaset: &ReplicaSet) -> Option<&str> {
    replicaset
        .metadata
        .annotations
        .get("deployment.kubernetes.io/revision")
        .map(String::as_str)
}

/// The template a Deployment ran before `revision`: its ReplicaSet with the
/// highest lower revision.
fn old_template(
    kind: &str,
    workload: &Deployment,
    replicasets: &[ReplicaSet],
    revision: &str,
) -> Option<Value> {
    if kind != "deployment" {
        return None;
    }
    let current: i64 = revision.parse().ok()?;
    owned(workload, replicasets)
        .filter_map(|replicaset| {
            let number: i64 = replica_revision(replicaset)?.parse().ok()?;
            (number < current).then_some((number, replicaset))
        })
        .max_by_key(|(number, _)| *number)
        .map(|(_, replicaset)| replicaset.spec.template.clone())
}

/// One difference between two pod templates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateChange {
    /// `image`, `env`, `resources`, `config` (ConfigMap or Secret
    /// references, template annotations) or `spec` (anything else).
    pub category: &'static str,
    /// `redis: image redis:7.2 → redis:7.4`. Env values are never shown,
    /// only the variables' names (they may hold secrets).
    pub detail: String,
}

fn containers(template: &Value) -> BTreeMap<String, &Value> {
    template
        .pointer("/spec/containers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|container| Some((container.get("name")?.as_str()?.to_string(), container)))
        .collect()
}

fn images(template: &Value) -> Vec<String> {
    containers(template)
        .values()
        .filter_map(|container| container.get("image")?.as_str().map(str::to_owned))
        .collect()
}

/// Names of ConfigMaps and Secrets a pod template references.
fn config_refs(template: &Value) -> BTreeSet<String> {
    let mut refs = BTreeSet::new();
    let mut walk = |value: &Value| collect_refs(value, &mut refs);
    if let Some(spec) = template.get("spec") {
        walk(spec);
    }
    refs
}

fn collect_refs(value: &Value, refs: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                let kind = match key.as_str() {
                    "configMap" | "configMapRef" | "configMapKeyRef" => Some("configmap"),
                    "secret" | "secretRef" | "secretKeyRef" => Some("secret"),
                    _ => None,
                };
                if let Some(kind) = kind {
                    let name = value
                        .get("name")
                        .or_else(|| value.get("secretName"))
                        .and_then(Value::as_str);
                    if let Some(name) = name {
                        refs.insert(format!("{kind} {name}"));
                    }
                }
                collect_refs(value, refs);
            }
        }
        Value::Array(items) => items.iter().for_each(|item| collect_refs(item, refs)),
        _ => {}
    }
}

/// What changed from `old` to `new`, in a stable order.
pub fn template_changes(old: &Value, new: &Value) -> Vec<TemplateChange> {
    let mut changes = Vec::new();
    let (old_containers, new_containers) = (containers(old), containers(new));
    let names: BTreeSet<&String> = old_containers.keys().chain(new_containers.keys()).collect();
    for name in names {
        match (old_containers.get(name), new_containers.get(name)) {
            (Some(before), Some(after)) => {
                let field = |container: &Value, field: &str| container.get(field).cloned();
                let image = |container: &Value| {
                    container
                        .get("image")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string()
                };
                if image(before) != image(after) {
                    changes.push(TemplateChange {
                        category: "image",
                        detail: format!("{name}: image {} → {}", image(before), image(after)),
                    });
                }
                let env_names = |container: &Value| -> BTreeMap<String, Value> {
                    container
                        .get("env")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|item| {
                            Some((item.get("name")?.as_str()?.to_string(), item.clone()))
                        })
                        .collect()
                };
                let (before_env, after_env) = (env_names(before), env_names(after));
                let changed_env: Vec<&String> = before_env
                    .keys()
                    .chain(after_env.keys())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .filter(|key| before_env.get(*key) != after_env.get(*key))
                    .collect();
                if !changed_env.is_empty() || field(before, "envFrom") != field(after, "envFrom") {
                    let list: Vec<&str> = changed_env.iter().map(|name| name.as_str()).collect();
                    changes.push(TemplateChange {
                        category: "env",
                        detail: if list.is_empty() {
                            format!("{name}: envFrom")
                        } else {
                            format!("{name}: env {}", list.join(", "))
                        },
                    });
                }
                if field(before, "resources") != field(after, "resources") {
                    changes.push(TemplateChange {
                        category: "resources",
                        detail: format!(
                            "{name}: resources {} → {}",
                            compact_resources(before),
                            compact_resources(after)
                        ),
                    });
                }
            }
            (None, Some(_)) => changes.push(TemplateChange {
                category: "spec",
                detail: format!("container {name} added"),
            }),
            (Some(_), None) => changes.push(TemplateChange {
                category: "spec",
                detail: format!("container {name} removed"),
            }),
            (None, None) => {}
        }
    }
    let (before_refs, after_refs) = (config_refs(old), config_refs(new));
    if before_refs != after_refs {
        let removed: Vec<&String> = before_refs.difference(&after_refs).collect();
        let added: Vec<&String> = after_refs.difference(&before_refs).collect();
        changes.push(TemplateChange {
            category: "config",
            detail: format!(
                "config {} → {}",
                if removed.is_empty() {
                    "-".to_string()
                } else {
                    removed
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                },
                if added.is_empty() {
                    "-".to_string()
                } else {
                    added
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                }
            ),
        });
    }
    let annotations = |template: &Value| -> BTreeMap<String, Value> {
        template
            .pointer("/metadata/annotations")
            .and_then(Value::as_object)
            .map(|map| map.clone().into_iter().collect())
            .unwrap_or_default()
    };
    let (before_notes, after_notes) = (annotations(old), annotations(new));
    let changed_notes: Vec<&String> = before_notes
        .keys()
        .chain(after_notes.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|key| before_notes.get(*key) != after_notes.get(*key))
        .collect();
    if !changed_notes.is_empty() {
        changes.push(TemplateChange {
            category: "config",
            detail: format!(
                "annotations {}",
                changed_notes
                    .iter()
                    .map(|key| key.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        });
    }
    // Anything else in the pod spec (volumes, probes, node selector...).
    let strip = |template: &Value| {
        let mut spec = template.get("spec").cloned().unwrap_or(Value::Null);
        if let Some(map) = spec.as_object_mut() {
            map.remove("containers");
        }
        spec
    };
    let other_container_fields = |template: &Value| -> Vec<Value> {
        containers(template)
            .values()
            .map(|container| {
                let mut container = (*container).clone();
                if let Some(map) = container.as_object_mut() {
                    for field in ["image", "env", "envFrom", "resources"] {
                        map.remove(field);
                    }
                }
                container
            })
            .collect()
    };
    let config_only = before_refs != after_refs;
    if (strip(old) != strip(new) && !config_only)
        || other_container_fields(old) != other_container_fields(new)
    {
        changes.push(TemplateChange {
            category: "spec",
            detail: "pod spec".into(),
        });
    }
    changes
}

fn compact_resources(container: &Value) -> String {
    let mut parts = Vec::new();
    for section in ["requests", "limits"] {
        if let Some(map) = container
            .pointer(&format!("/resources/{section}"))
            .and_then(Value::as_object)
        {
            for (name, value) in map {
                parts.push(format!(
                    "{section}.{name}={}",
                    value
                        .as_str()
                        .map_or_else(|| value.to_string(), str::to_owned)
                ));
            }
        }
    }
    if parts.is_empty() {
        "none".into()
    } else {
        parts.join(" ")
    }
}

#[cfg(test)]
mod tests;
