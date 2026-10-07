//! The backlog while no Poet answers, at k-lab scale (the fixtures' pods
//! repeated to about the 465 observations a k-lab batch carries): a 10
//! minute outage replays paced and with no gap, a 30 minute one stays
//! within the byte bound with its thinned windows marked, and everything
//! thinned or dropped is reported to Poet.

use ih_muse_core::MuseError;
use ih_muse_k8s::backlog::{BacklogConfig, DEFAULT_MAX_THINNING, THINNED_PRESSURE_POLICY};
use ih_muse_k8s::graph::Snapshot;
use ih_muse_k8s::identity::MuseIdentity;
use ih_muse_k8s::model::{List, Node, NodeMetrics, Pod, PodMetrics};
use ih_muse_k8s::{K8sMuse, BACKLOG_EVENT_NAME};
use ih_muse_proto::{AttributeValue, GraphIntakeRequest};

const INTERVAL: u64 = 5_000_000_000;
const START: u64 = 1_791_108_000_000_000_000;
const REPLAY: usize = 4;

fn parse<T: serde::de::DeserializeOwned>(name: &str) -> T {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// The fixtures with their pods repeated eight times.
fn klab_scale_snapshot() -> Snapshot {
    let pods: Vec<Pod> = parse::<List<Pod>>("pods.json").items;
    let metrics: Vec<PodMetrics> = parse::<List<PodMetrics>>("pod-metrics.json").items;
    let mut snapshot = Snapshot {
        nodes: parse::<List<Node>>("nodes.json").items,
        node_metrics: Some(parse::<List<NodeMetrics>>("node-metrics.json").items),
        pod_metrics: Some(Vec::new()),
        warnings: Some(Vec::new()),
        ..Snapshot::default()
    };
    for copy in 0..8 {
        for pod in &pods {
            let mut pod = pod.clone();
            pod.metadata.name = format!("{}-{copy}", pod.metadata.name);
            pod.metadata.uid = format!("{}-{copy}", pod.metadata.uid);
            snapshot.pods.push(pod);
        }
        for item in &metrics {
            let mut item = item.clone();
            item.metadata.name = format!("{}-{copy}", item.metadata.name);
            snapshot.pod_metrics.as_mut().unwrap().push(item);
        }
    }
    snapshot
}

fn muse(config: BacklogConfig) -> K8sMuse {
    K8sMuse::new(
        MuseIdentity::new(Some("k-lab"), "api"),
        "org",
        "k-lab",
        "ns",
        INTERVAL,
    )
    .unwrap()
    .with_cluster_name(Some("k-lab"))
    .with_backlog(config)
}

/// Collects and queues one interval; returns the bytes then held.
fn collect(muse: &mut K8sMuse, snapshot: &Snapshot, tick: u64) -> usize {
    let now = START + tick * INTERVAL;
    let request = muse.intake(snapshot, now);
    muse.enqueue(request, now);
    muse.pending_bytes()
}

async fn unreachable(muse: &mut K8sMuse) {
    let result = muse
        .send_pending(REPLAY, |_| async {
            Err(MuseError::Unavailable("down".into()))
        })
        .await;
    assert!(result.is_err());
}

/// One paced send to an answering Poet; returns what it accepted.
async fn reachable(muse: &mut K8sMuse) -> Vec<GraphIntakeRequest> {
    let mut accepted = Vec::new();
    let sent = muse
        .send_pending(REPLAY, |request| {
            accepted.push(request);
            async { Ok(()) }
        })
        .await
        .unwrap();
    assert_eq!(sent, accepted.len());
    assert!(sent <= REPLAY, "replay is paced");
    accepted
}

/// The window a delivered batch's availability states.
fn window(request: &GraphIntakeRequest) -> (u64, u64) {
    let availability = &request.batch.availability;
    assert!(!availability.is_empty());
    (
        availability
            .iter()
            .map(|a| a.available_time.from_unix_nano)
            .min()
            .unwrap(),
        availability
            .iter()
            .map(|a| a.available_time.to_unix_nano)
            .max()
            .unwrap(),
    )
}

fn loss_events(delivered: &[GraphIntakeRequest]) -> Vec<&ih_muse_proto::Event> {
    delivered
        .iter()
        .flat_map(|request| &request.batch.events)
        .filter(|event| {
            event.attributes.get("event.name")
                == Some(&AttributeValue::String(BACKLOG_EVENT_NAME.into()))
        })
        .collect()
}

fn reported(events: &[&ih_muse_proto::Event], name: &str) -> u64 {
    events
        .iter()
        .map(|event| match event.attributes.get(name) {
            Some(AttributeValue::U64(count)) => *count,
            other => panic!("{name}: {other:?}"),
        })
        .sum()
}

/// Delivers what is queued at the replay pace while collecting on; returns
/// everything delivered, oldest first.
async fn replay(muse: &mut K8sMuse, snapshot: &Snapshot, mut tick: u64) -> Vec<GraphIntakeRequest> {
    let mut delivered = Vec::new();
    let mut sends = 0;
    while muse.pending() > 0 {
        tick += 1;
        collect(muse, snapshot, tick);
        delivered.extend(reachable(muse).await);
        sends += 1;
        assert!(sends < 1_000, "the backlog drains");
    }
    delivered
}

#[tokio::test]
async fn a_10_minute_outage_replays_paced_with_no_gap() {
    let snapshot = klab_scale_snapshot();
    let mut muse = muse(BacklogConfig::default());
    let mut peak = 0;
    for tick in 0..120 {
        peak = peak.max(collect(&mut muse, &snapshot, tick));
        unreachable(&mut muse).await;
    }
    let observations = muse.intake(&snapshot, START).batch.observations.len();
    eprintln!(
        "{observations} observations per batch; 10 min backlog: {} batches, {peak} bytes",
        muse.pending()
    );
    assert!(observations >= 400, "k-lab scale: {observations}");
    assert_eq!(muse.pending(), 120, "the whole outage is kept");
    assert!(
        peak <= BacklogConfig::default().max_bytes / 2,
        "{peak} bytes"
    );

    let delivered = replay(&mut muse, &snapshot, 119).await;
    assert!(muse.lost().is_empty(), "nothing thinned or dropped");
    assert!(loss_events(&delivered).is_empty());
    for pair in delivered.windows(2) {
        let (previous, next) = (window(&pair[0]), window(&pair[1]));
        assert!(
            next.0 <= previous.1,
            "gap between {previous:?} and {next:?}"
        );
        assert_eq!(next.0, previous.0 + INTERVAL, "one batch per interval");
    }
    assert!(delivered
        .iter()
        .flat_map(|request| &request.batch.availability)
        .all(|a| a.pressure_policy.is_none() && a.coverage_fraction == 1.0));
}

#[tokio::test]
async fn a_30_minute_outage_stays_within_the_bound_and_marks_what_it_thinned() {
    let snapshot = klab_scale_snapshot();
    // A bound that holds 10 minutes unthinned.
    let one = {
        let mut probe = muse(BacklogConfig::default());
        collect(&mut probe, &snapshot, 1);
        collect(&mut probe, &snapshot, 2) - probe.pending_bytes() / 2
    };
    let bound = one * 120;
    let mut muse = muse(BacklogConfig {
        max_bytes: bound,
        max_thinning: DEFAULT_MAX_THINNING,
    });
    for tick in 0..360 {
        let held = collect(&mut muse, &snapshot, tick);
        assert!(
            held <= bound,
            "{held} bytes over the {bound} byte bound at {tick}"
        );
        unreachable(&mut muse).await;
    }
    let lost = muse.lost().clone();
    eprintln!(
        "30 min within {bound} bytes: {} batches, {} bytes, {} intervals thinned, {} dropped",
        muse.pending(),
        muse.pending_bytes(),
        lost.thinned_intervals,
        lost.dropped_intervals
    );
    assert!(lost.thinned_intervals > 0);
    assert_eq!(lost.dropped_intervals, 0, "thinning sufficed");

    let delivered = replay(&mut muse, &snapshot, 359).await;
    // The whole outage is covered: thinned windows are marked, not holes.
    let (first, _) = window(&delivered[0]);
    assert_eq!(first, START - INTERVAL);
    for pair in delivered.windows(2) {
        let (previous, next) = (window(&pair[0]), window(&pair[1]));
        assert!(
            next.0 <= previous.1,
            "unmarked gap between {previous:?} and {next:?}"
        );
    }
    let thinned: Vec<_> = delivered
        .iter()
        .filter(|request| {
            request
                .batch
                .availability
                .iter()
                .any(|a| a.pressure_policy.as_deref() == Some(THINNED_PRESSURE_POLICY))
        })
        .collect();
    assert!(!thinned.is_empty());
    for request in thinned {
        let (from, to) = window(request);
        for availability in &request.batch.availability {
            assert!(availability.coverage_fraction < 1.0);
            assert_eq!(availability.resolution_unix_nano, to - from);
        }
    }
    // Every thinned interval, also those thinned while replaying, is
    // reported to Poet, once.
    let lost = muse.lost().clone();
    assert_eq!(lost.dropped_intervals, 0);
    let events = loss_events(&delivered);
    assert!(!events.is_empty());
    assert_eq!(
        reported(&events, "ih.muse.backlog.thinned_intervals"),
        lost.thinned_intervals
    );
    assert_eq!(reported(&events, "ih.muse.backlog.dropped_intervals"), 0);
}

#[tokio::test]
async fn beyond_the_thinning_limit_the_oldest_is_dropped_and_reported() {
    let snapshot = klab_scale_snapshot();
    let mut muse = muse(BacklogConfig {
        max_bytes: 1,
        max_thinning: 2,
    });
    for tick in 0..50 {
        collect(&mut muse, &snapshot, tick);
        assert_eq!(muse.pending(), 1, "only the newest batch is kept");
        unreachable(&mut muse).await;
    }
    let lost = muse.lost().clone();
    assert_eq!(lost.dropped_intervals, 49);
    assert_eq!(lost.from_unix_nano, START - INTERVAL);

    let delivered = replay(&mut muse, &snapshot, 49).await;
    let events = loss_events(&delivered);
    let lost = muse.lost().clone();
    let total = |name: &str| match events.last().unwrap().attributes.get(name) {
        Some(AttributeValue::U64(count)) => *count,
        other => panic!("{name}: {other:?}"),
    };
    // Reports in dropped batches went back to unreported: the delivered
    // ones still account for every dropped interval and the whole window.
    assert_eq!(
        reported(&events, "ih.muse.backlog.dropped_intervals")
            + reported(&events, "ih.muse.backlog.thinned_intervals"),
        lost.dropped_intervals + lost.thinned_intervals
    );
    assert_eq!(
        total("ih.muse.backlog.dropped_intervals_total"),
        lost.dropped_intervals
    );
    let from = events.iter().map(|e| e.time.from_unix_nano).min().unwrap();
    assert_eq!(
        from,
        START - INTERVAL,
        "the gap starts where the outage did"
    );
}
