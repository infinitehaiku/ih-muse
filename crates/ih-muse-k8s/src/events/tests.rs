//! Detection: two observed states in, the expected events out, with ids
//! that stay the same when the same change is seen again.

use serde_json::json;

use super::*;
use crate::graph::{collect_graph_with, GraphConfig};
use crate::model::{Node, StatefulSet};

const S: u64 = 1_000_000_000;
/// 2026-10-04T16:25:00Z, the hour of the k-lab crash loop.
const NOW: u64 = 1_791_131_100 * S;

fn pod(restarts: i64, state: serde_json::Value, last: serde_json::Value) -> Pod {
    serde_json::from_value(json!({
        "metadata": {"name": "piceli-gitops-7f9c", "namespace": "piceli-system",
                     "uid": "pod-gitops"},
        "spec": {"nodeName": "amedes",
                 "containers": [{"name": "gitops", "image": "ghcr.io/pynenc/piceli-controller:0.14",
                                 "resources": {"limits": {"memory": "256Mi"}}}]},
        "status": {"phase": "Running", "containerStatuses": [{
            "name": "gitops", "ready": false, "restartCount": restarts,
            "image": "ghcr.io/pynenc/piceli-controller:0.14",
            "imageID": "ghcr.io/pynenc/piceli-controller@sha256:870e4251aaaa",
            "state": state, "lastState": last}]}
    }))
    .unwrap()
}

fn crashed(exit: i64, reason: &str) -> serde_json::Value {
    json!({"terminated": {"exitCode": exit, "reason": reason,
                          "finishedAt": "2026-10-04T16:24:30Z"}})
}

fn backoff() -> serde_json::Value {
    json!({"waiting": {"reason": "CrashLoopBackOff",
                       "message": "back-off 5m0s restarting failed container=gitops"}})
}

fn snapshot(pods: Vec<Pod>) -> Snapshot {
    Snapshot {
        pods,
        ..Snapshot::default()
    }
}

fn ids(events: &[DetectedEvent]) -> Vec<&str> {
    events.iter().map(|event| event.id.as_str()).collect()
}

#[test]
fn a_crash_loop_is_a_restart_with_its_reason_and_one_waiting_event() {
    let mut detector = Detector::new();
    let before = snapshot(vec![pod(
        36,
        json!({"running": {"startedAt": "2026-10-04T16:20:00Z"}}),
        crashed(1, "Error"),
    )]);
    detector.detect(&before, NOW - 60 * S);
    let after = snapshot(vec![pod(37, backoff(), crashed(1, "Error"))]);
    let events = detector.detect(&after, NOW);
    assert_eq!(
        ids(&events),
        [
            "restart/pod-gitops/gitops/37",
            "waiting/pod-gitops/gitops/CrashLoopBackOff/37"
        ]
    );
    let restart = &events[0];
    assert_eq!(restart.name, RESTART);
    assert_eq!(restart.severity, Severity::Warning);
    assert_eq!(
        restart.time_unix_nano,
        NOW - 30 * S,
        "the restart is placed when the container ended"
    );
    assert_eq!(
        restart.attributes["k8s.container.last_terminated.reason"],
        "Error"
    );
    assert_eq!(
        restart.attributes["k8s.container.last_terminated.exit_code"],
        "1"
    );
    assert_eq!(restart.attributes["k8s.node.name"], "amedes");
    assert_eq!(
        restart.summary,
        "piceli-system/piceli-gitops-7f9c gitops restarted: Error (exit 1), 37 restarts"
    );
    let waiting = &events[1];
    assert_eq!(waiting.severity, Severity::Error);
    assert_eq!(
        waiting.attributes["k8s.container.status.reason"],
        "CrashLoopBackOff"
    );
    assert_eq!(
        waiting.summary,
        "piceli-system/piceli-gitops-7f9c gitops CrashLoopBackOff, 37 restarts"
    );
    assert_eq!(
        waiting.target,
        Target::Pod {
            uid: "pod-gitops".into(),
            namespace: "piceli-system".into()
        }
    );

    // Seen again: nothing new. The next loop: one restart, no second
    // waiting event (it is still the same crash loop).
    assert!(detector.detect(&after, NOW + 5 * S).is_empty());
    let running = snapshot(vec![pod(
        37,
        json!({"running": {"startedAt": "2026-10-04T16:30:00Z"}}),
        crashed(1, "Error"),
    )]);
    assert!(detector.detect(&running, NOW + 300 * S).is_empty());
    let again = snapshot(vec![pod(38, backoff(), crashed(1, "Error"))]);
    assert_eq!(
        ids(&detector.detect(&again, NOW + 310 * S)),
        ["restart/pod-gitops/gitops/38"]
    );
}

#[test]
fn a_restarted_muse_names_the_same_changes_with_the_same_ids() {
    let after = snapshot(vec![pod(37, backoff(), crashed(1, "Error"))]);
    let first = Detector::new().detect(&after, NOW);
    let second = Detector::new().detect(&after, NOW + 20 * S);
    assert_eq!(ids(&first), ids(&second));
    assert_eq!(
        first[0].time_unix_nano, second[0].time_unix_nano,
        "a restart's time comes from the API, not from the collection"
    );
    // A restart that ended long ago is history, not news, on a first look.
    let old = snapshot(vec![pod(37, json!({"running": {}}), crashed(1, "Error"))]);
    assert!(Detector::new().detect(&old, NOW + 3 * 3600 * S).is_empty());
}

#[test]
fn oomkills_and_segfaults_are_errors_and_oomkills_are_counted() {
    let mut detector = Detector::new();
    detector.detect(
        &snapshot(vec![pod(2, json!({"running": {}}), json!({}))]),
        NOW - 60 * S,
    );
    let events = detector.detect(
        &snapshot(vec![pod(
            3,
            json!({"running": {}}),
            crashed(137, "OOMKilled"),
        )]),
        NOW,
    );
    assert_eq!(events[0].severity, Severity::Error);
    assert_eq!(
        events[0].attributes["k8s.container.last_terminated.signal"],
        "SIGKILL"
    );
    assert_eq!(detector.oom_kills("pod-gitops", "gitops").unwrap().kills, 1);

    let events = detector.detect(
        &snapshot(vec![pod(4, json!({"running": {}}), crashed(139, "Error"))]),
        NOW + 60 * S,
    );
    assert_eq!(
        events[0].severity,
        Severity::Error,
        "a segfault may be the node's memory"
    );
    assert!(
        events[0].summary.contains("Error (exit 139, SIGSEGV)"),
        "{}",
        events[0].summary
    );
    assert_eq!(detector.oom_kills("pod-gitops", "gitops").unwrap().kills, 1);
}

#[test]
fn image_pull_and_config_errors_are_reported_but_a_normal_start_is_not() {
    let waiting = |reason: &str| json!({"waiting": {"reason": reason, "message": "m"}});
    let mut detector = Detector::new();
    assert!(detector
        .detect(
            &snapshot(vec![pod(0, waiting("ContainerCreating"), json!({}))]),
            NOW
        )
        .is_empty());
    let events = detector.detect(
        &snapshot(vec![pod(0, waiting("ImagePullBackOff"), json!({}))]),
        NOW + S,
    );
    assert_eq!(
        ids(&events),
        ["waiting/pod-gitops/gitops/ImagePullBackOff/0"]
    );
    let events = detector.detect(
        &snapshot(vec![pod(
            0,
            waiting("CreateContainerConfigError"),
            json!({}),
        )]),
        NOW + 2 * S,
    );
    assert_eq!(
        ids(&events),
        ["waiting/pod-gitops/gitops/CreateContainerConfigError/0"]
    );
}

#[test]
fn a_new_image_digest_in_the_same_pod_is_a_change() {
    let mut detector = Detector::new();
    let before = pod(0, json!({"running": {}}), json!({}));
    detector.detect(&snapshot(vec![before.clone()]), NOW);
    let mut after = before;
    after.status.container_statuses[0].image_id =
        "ghcr.io/pynenc/piceli-controller@sha256:9f12c973bbbb".into();
    let events = detector.detect(&snapshot(vec![after.clone()]), NOW + S);
    assert_eq!(
        ids(&events),
        ["image/pod-gitops/gitops/ghcr.io/pynenc/piceli-controller@sha256:9f12c973bbbb"]
    );
    assert_eq!(events[0].name, IMAGE_CHANGE);
    assert!(
        events[0]
            .summary
            .contains("@870e4251aaaa → ghcr.io/pynenc/piceli-controller:0.14@9f12c973bbbb"),
        "{}",
        events[0].summary
    );
    assert!(detector
        .detect(&snapshot(vec![after]), NOW + 2 * S)
        .is_empty());
}

#[test]
fn an_unschedulable_pod_is_reported_once() {
    let mut pending: Pod = serde_json::from_value(json!({
        "metadata": {"name": "big", "namespace": "ih-x", "uid": "pod-big"},
        "status": {"phase": "Pending", "conditions": [{"type": "PodScheduled", "status": "False",
            "reason": "Unschedulable", "lastTransitionTime": "2026-10-04T16:00:00Z",
            "message": "0/4 nodes are available: 4 Insufficient memory."}]}
    }))
    .unwrap();
    let mut detector = Detector::new();
    let events = detector.detect(&snapshot(vec![pending.clone()]), NOW);
    assert_eq!(ids(&events), ["unschedulable/pod-big/2026-10-04T16:00:00Z"]);
    assert_eq!(
        events[0].attributes["k8s.event.message"],
        "0/4 nodes are available: 4 Insufficient memory."
    );
    assert!(detector
        .detect(&snapshot(vec![pending.clone()]), NOW + S)
        .is_empty());
    pending.status.conditions.clear();
    assert!(detector
        .detect(&snapshot(vec![pending]), NOW + 2 * S)
        .is_empty());
}

fn node(ready: &str, memory_pressure: &str, since: &str) -> Node {
    serde_json::from_value(json!({
        "metadata": {"name": "amedes", "uid": "node-amedes"},
        "status": {"conditions": [
            {"type": "Ready", "status": ready, "reason": "KubeletReady", "lastTransitionTime": since},
            {"type": "MemoryPressure", "status": memory_pressure, "lastTransitionTime": since},
            {"type": "DiskPressure", "status": "False", "lastTransitionTime": "2026-09-01T00:00:00Z"}]}
    }))
    .unwrap()
}

#[test]
fn node_condition_changes_are_events_and_a_healthy_node_is_quiet() {
    let mut detector = Detector::new();
    let healthy = Snapshot {
        nodes: vec![node("True", "False", "2026-09-01T00:00:00Z")],
        ..Snapshot::default()
    };
    assert!(detector.detect(&healthy, NOW).is_empty());
    let sick = Snapshot {
        nodes: vec![node("False", "True", "2026-10-04T16:20:00Z")],
        ..Snapshot::default()
    };
    let events = detector.detect(&sick, NOW + S);
    assert_eq!(
        ids(&events),
        [
            "node/node-amedes/MemoryPressure/True/2026-10-04T16:20:00Z",
            "node/node-amedes/Ready/False/2026-10-04T16:20:00Z"
        ]
    );
    let ready = events
        .iter()
        .find(|event| event.attributes["k8s.node.condition.type"] == "Ready")
        .unwrap();
    assert_eq!(ready.severity, Severity::Error);
    assert_eq!(ready.attributes["k8s.node.condition.previous"], "True");
    assert_eq!(ready.time_unix_nano, NOW - 300 * S);
    // Only the standard conditions become the condition metric (k3s adds
    // `EtcdIsVoter`, True on every etcd member).
    let mut voter = node("True", "False", "2026-09-01T00:00:00Z");
    voter
        .status
        .conditions
        .push(serde_json::from_value(json!({"type": "EtcdIsVoter", "status": "True"})).unwrap());
    let config = GraphConfig {
        organization: "o".into(),
        cluster_uid: "c".into(),
        cluster_name: None,
        namespace: "n".into(),
        all_namespaces: true,
        expected_interval_ns: 5 * S,
        source_id: "s".into(),
        source_revision: "r".into(),
    };
    let batch = collect_graph_with(
        &Snapshot {
            nodes: vec![voter],
            ..Snapshot::default()
        },
        &config,
        NOW,
        None,
        &[],
    );
    assert!(batch.observations.iter().all(|observation| observation
        .attributes
        .get("k8s.node.condition.type")
        != Some(&ih_muse_proto::AttributeValue::String("EtcdIsVoter".into()))));
    // A fresh Muse that finds the node sick reports the same ids.
    assert_eq!(
        ids(&Detector::new().detect(&sick, NOW + 9 * S)),
        ids(&events)
    );
}

fn deployment(revision: &str, image: &str, env: &str, status: serde_json::Value) -> Deployment {
    serde_json::from_value(json!({
        "metadata": {"name": "ih-kabuki", "namespace": "ih-main", "uid": "dep-kabuki",
                     "generation": 7,
                     "annotations": {"deployment.kubernetes.io/revision": revision}},
        "spec": {"replicas": 1, "template": template(image, env)},
        "status": status
    }))
    .unwrap()
}

fn template(image: &str, env: &str) -> serde_json::Value {
    json!({"metadata": {"labels": {"app": "kabuki"}},
           "spec": {"containers": [{"name": "kabuki", "image": image,
                                    "env": [{"name": "MODE", "value": env},
                                            {"name": "TOKEN", "valueFrom": {"secretKeyRef": {"name": "ih-private", "key": "t"}}}],
                                    "resources": {"limits": {"memory": "256Mi"}}}],
                    "volumes": [{"name": "c", "configMap": {"name": "kabuki-config-a1"}}]}})
}

fn replicaset(revision: &str, created: &str, template: serde_json::Value) -> ReplicaSet {
    serde_json::from_value(json!({
        "metadata": {"name": format!("ih-kabuki-{revision}"), "namespace": "ih-main",
                     "uid": format!("rs-{revision}"), "creationTimestamp": created,
                     "annotations": {"deployment.kubernetes.io/revision": revision},
                     "ownerReferences": [{"kind": "Deployment", "name": "ih-kabuki", "uid": "dep-kabuki"}]},
        "spec": {"template": template}
    }))
    .unwrap()
}

#[test]
fn a_deployment_rollout_starts_with_what_changed_and_finishes_once() {
    let complete = json!({"observedGeneration": 7, "replicas": 1, "updatedReplicas": 1,
                          "readyReplicas": 1, "availableReplicas": 1,
                          "conditions": [{"type": "Progressing", "status": "True",
                                          "reason": "NewReplicaSetAvailable",
                                          "lastUpdateTime": "2026-10-04T16:26:40Z"}]});
    let rolling = json!({"observedGeneration": 7, "replicas": 2, "updatedReplicas": 1,
                         "readyReplicas": 1, "availableReplicas": 1});
    let old = template("ih/kabuki:abc", "a");
    let mut detector = Detector::new();
    let before = Snapshot {
        deployments: vec![deployment("3", "ih/kabuki:abc", "a", complete.clone())],
        replicasets: vec![replicaset("3", "2026-10-04T10:00:00Z", old.clone())],
        ..Snapshot::default()
    };
    assert!(
        detector.detect(&before, NOW - 60 * S).is_empty(),
        "a settled deployment is quiet"
    );
    let replicasets = vec![
        replicaset("3", "2026-10-04T10:00:00Z", old),
        replicaset("4", "2026-10-04T16:24:00Z", template("ih/kabuki:def", "b")),
    ];
    let during = Snapshot {
        deployments: vec![deployment("4", "ih/kabuki:def", "b", rolling)],
        replicasets: replicasets.clone(),
        ..Snapshot::default()
    };
    let events = detector.detect(&during, NOW);
    assert_eq!(ids(&events), ["rollout/dep-kabuki/4/start"]);
    let start = &events[0];
    assert_eq!(
        start.time_unix_nano,
        NOW - 60 * S,
        "started when its ReplicaSet was made"
    );
    assert_eq!(start.attributes["k8s.rollout.changes"], "env,image");
    assert_eq!(start.attributes["k8s.rollout.previous_revision"], "3");
    assert_eq!(
        start.attributes["k8s.rollout.details"],
        "kabuki: image ih/kabuki:abc → ih/kabuki:def; kabuki: env MODE"
    );
    assert!(
        !start.summary.contains("\"b\""),
        "env values never leave the Muse"
    );
    assert!(detector.detect(&during, NOW + 5 * S).is_empty());

    let after = Snapshot {
        deployments: vec![deployment("4", "ih/kabuki:def", "b", complete)],
        replicasets,
        ..Snapshot::default()
    };
    let events = detector.detect(&after, NOW + 100 * S);
    assert_eq!(ids(&events), ["rollout/dep-kabuki/4/finish"]);
    assert_eq!(events[0].time_unix_nano, NOW + 100 * S);
    assert!(detector.detect(&after, NOW + 105 * S).is_empty());
}

#[test]
fn template_changes_name_config_and_resources_without_values() {
    let before = template("i:1", "a");
    let mut after = template("i:1", "a");
    after["spec"]["volumes"][0]["configMap"]["name"] = json!("kabuki-config-b2");
    after["spec"]["containers"][0]["resources"]["limits"]["memory"] = json!("512Mi");
    after["metadata"]["annotations"] = json!({"checksum/config": "x"});
    let changes = template_changes(&before, &after);
    let details: Vec<&str> = changes
        .iter()
        .map(|change| change.detail.as_str())
        .collect();
    assert_eq!(
        details,
        [
            "kabuki: resources limits.memory=256Mi → limits.memory=512Mi",
            "config configmap kabuki-config-a1 → configmap kabuki-config-b2",
            "annotations checksum/config"
        ]
    );
    assert!(template_changes(&before, &before).is_empty());
}

#[test]
fn a_statefulset_rollout_follows_its_update_revision() {
    let statefulset = |current: &str, update: &str, ready: i64| -> StatefulSet {
        serde_json::from_value(json!({
            "metadata": {"name": "ih-poet", "namespace": "ih-main", "uid": "sts-poet", "generation": 3},
            "spec": {"replicas": 2, "template": template("ih/poet:1", "a")},
            "status": {"observedGeneration": 3, "replicas": 2, "readyReplicas": ready,
                       "updatedReplicas": if current == update { 2 } else { 1 },
                       "currentRevision": current, "updateRevision": update}
        }))
        .unwrap()
    };
    let mut detector = Detector::new();
    let at = |set| Snapshot {
        statefulsets: vec![set],
        ..Snapshot::default()
    };
    assert!(detector
        .detect(&at(statefulset("ih-poet-a1", "ih-poet-a1", 2)), NOW)
        .is_empty());
    let events = detector.detect(&at(statefulset("ih-poet-a1", "ih-poet-b2", 1)), NOW + S);
    assert_eq!(ids(&events), ["rollout/sts-poet/ih-poet-b2/start"]);
    assert!(events[0]
        .summary
        .starts_with("statefulset ih-main/ih-poet rollout to revision b2"));
    let events = detector.detect(&at(statefulset("ih-poet-b2", "ih-poet-b2", 2)), NOW + 9 * S);
    assert_eq!(ids(&events), ["rollout/sts-poet/ih-poet-b2/finish"]);
}

fn warning(uid: &str, last: &str, kind: &str, name: &str) -> KubeEvent {
    serde_json::from_value(json!({
        "metadata": {"name": format!("{name}.1"), "namespace": "piceli-system", "uid": uid},
        "involvedObject": {"kind": kind, "namespace": if kind == "Node" { "" } else { "piceli-system" },
                           "name": name, "uid": format!("{name}-uid")},
        "reason": "BackOff", "message": "Back-off restarting failed container gitops",
        "type": "Warning", "count": 37, "lastTimestamp": last
    }))
    .unwrap()
}

#[test]
fn warnings_are_reported_once_per_quarter_hour_while_they_repeat() {
    let mut detector = Detector::new();
    let at = |last: &str| Snapshot {
        warnings: Some(vec![warning("ev-1", last, "Pod", "piceli-gitops-7f9c")]),
        ..Snapshot::default()
    };
    let first = detector.detect(&at("2026-10-04T16:20:00Z"), NOW);
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].name, WARNING);
    assert_eq!(first[0].attributes["k8s.event.count"], "37");
    assert_eq!(
        first[0].summary,
        "piceli-system/piceli-gitops-7f9c BackOff: Back-off restarting failed container gitops (×37)"
    );
    // The same Kubernetes Event, updated within the quarter hour: once.
    assert!(detector
        .detect(&at("2026-10-04T16:24:00Z"), NOW + 60 * S)
        .is_empty());
    // Still repeating later: a new marker.
    assert_eq!(
        detector
            .detect(&at("2026-10-04T16:40:00Z"), NOW + 900 * S)
            .len(),
        1
    );
    // Another Muse names it the same.
    assert_eq!(
        ids(&Detector::new().detect(&at("2026-10-04T16:20:00Z"), NOW)),
        ids(&first)
    );
}

#[test]
fn a_node_warning_lands_on_the_node() {
    let snapshot = Snapshot {
        nodes: vec![node("True", "False", "2026-09-01T00:00:00Z")],
        warnings: Some(vec![warning(
            "ev-2",
            "2026-10-04T16:20:00Z",
            "Node",
            "amedes",
        )]),
        ..Snapshot::default()
    };
    let events = Detector::new().detect(&snapshot, NOW);
    assert_eq!(
        events[0].target,
        Target::Node {
            uid: "node-amedes".into()
        }
    );
    assert_eq!(events[0].attributes["k8s.node.name"], "amedes");
}

#[test]
fn events_reach_the_batch_on_their_elements_with_cluster_scoped_ids() {
    let config = GraphConfig {
        organization: "ih-local".into(),
        cluster_uid: "k-lab-uid".into(),
        cluster_name: Some("k-lab".into()),
        namespace: "ih-main".into(),
        all_namespaces: true,
        expected_interval_ns: 5 * S,
        source_id: "ih-k8s-muse:k-lab".into(),
        source_revision: "test".into(),
    };
    let namespace: crate::model::Namespace =
        serde_json::from_value(json!({"metadata": {"name": "piceli-system", "uid": "ns-piceli"}}))
            .unwrap();
    let snapshot = Snapshot {
        pods: vec![pod(37, backoff(), crashed(137, "OOMKilled"))],
        namespaces: vec![namespace],
        warnings: Some(vec![warning(
            "ev-3",
            "2026-10-04T16:20:00Z",
            "ReplicaSet",
            "gone",
        )]),
        ..Snapshot::default()
    };
    let mut detector = Detector::new();
    let events = detector.detect(&snapshot, NOW);
    let batch = collect_graph_with(&snapshot, &config, NOW, Some(&detector), &events);
    batch
        .validate()
        .expect("events point at elements of the batch");
    let restart = batch
        .events
        .iter()
        .find(|event| event.event_id == "k8s:k-lab-uid:restart/pod-gitops/gitops/37")
        .expect("the restart, its id scoped to the cluster");
    assert!(matches!(
        &restart.entity.identity,
        ih_muse_proto::EntityIdentity::Kubernetes { resource_kind, resource_uid, .. }
            if resource_kind == "pod" && resource_uid == "pod-gitops"
    ));
    assert_eq!(
        restart.attributes["ih.event.severity"],
        ih_muse_proto::AttributeValue::String("error".into())
    );
    let replicaset = batch
        .events
        .iter()
        .find(|event| event.event_id.contains("warning/ev-3"))
        .unwrap();
    assert!(
        matches!(
            &replicaset.entity.identity,
            ih_muse_proto::EntityIdentity::Kubernetes { resource_kind, .. } if resource_kind == "namespace"
        ),
        "an object the Muse does not model lands on its namespace"
    );
    // The waiting gauge carries the reason; the OOMKill counter counts one.
    let gauge = |name: &str| {
        batch
            .observations
            .iter()
            .find(|observation| observation.descriptor.name == name)
            .unwrap()
            .clone()
    };
    let waiting = gauge(crate::graph::CONTAINER_WAITING_METRIC);
    assert_eq!(
        waiting.attributes[crate::graph::WAITING_REASON_ATTRIBUTE],
        ih_muse_proto::AttributeValue::String("CrashLoopBackOff".into())
    );
    let oom = gauge(crate::graph::CONTAINER_OOM_KILLS_METRIC);
    assert!(matches!(
        oom.value,
        ih_muse_proto::MetricObservation::Measured {
            value: ih_muse_proto::TypedMetricValue::Sum(ih_muse_proto::Number::I64(1))
        }
    ));
    // The pod lands in its own namespace, not the Muse's.
    let pod = batch
        .entities
        .iter()
        .find(|entity| matches!(&entity.key.identity, ih_muse_proto::EntityIdentity::Kubernetes { resource_kind, .. } if resource_kind == "pod"))
        .unwrap();
    assert_eq!(
        pod.attributes["k8s.namespace.name"],
        ih_muse_proto::AttributeValue::String("piceli-system".into())
    );
}
