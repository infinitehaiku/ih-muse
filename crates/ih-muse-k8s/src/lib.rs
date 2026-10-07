//! The Kubernetes Muse: reads the Kubernetes API in a pod and sends the
//! cluster, its nodes, the pods of one namespace or of every namespace,
//! their containers and workloads, with usage from metrics-server, to Poet
//! as `ih.graph.v1` batches, with the problems and changes it detects
//! between collections as events (`events`).
//!
//! A Rust port of `ih-infra/scripts/k8s_muse.py` built on ih-muse, in the
//! shape of the macOS Muse: deterministic identities, graph intake with
//! failover across the Poets of a cluster, and a dashboard definition
//! (`k8s.cluster`) attached until a Poet acknowledges it.

pub mod api;
pub mod backlog;
pub mod dashboards;
pub mod events;
pub mod graph;
pub mod identity;
pub mod model;

use std::collections::BTreeMap;

use ih_muse::dashboards::{DashboardDelivery, DashboardSetError};
use ih_muse_proto::{
    AttributeValue, EntityIdentity, EntityKey, Event, EventKind, GraphBatch, GraphIntakeRequest,
    InstrumentationScope, JoinStatus, OrganizationId, Provenance, TimeRange,
    GRAPH_INTAKE_CONTRACT_REVISION, GRAPH_INTAKE_SCHEMA_VERSION,
};

use crate::api::{ApiError, KubeApi};
use crate::backlog::{Backlog, BacklogConfig, Loss};
use crate::events::Detector;
use crate::graph::{GraphConfig, Snapshot};
use crate::identity::MuseIdentity;

/// `event.name` of the event that reports what the backlog thinned or
/// dropped while no Poet answered.
pub const BACKLOG_EVENT_NAME: &str = "k8s.muse.backlog.reduced";

/// The Muse between collections: who it is, what it sends, what is unsent.
pub struct K8sMuse {
    identity: MuseIdentity,
    graph: GraphConfig,
    delivery: DashboardDelivery,
    /// Unacknowledged batches, bounded by bytes.
    backlog: Backlog,
    namespace_uid: Option<String>,
    /// What the previous collection saw, to detect problems and changes.
    detector: Detector,
}

impl K8sMuse {
    /// `cluster_uid` keys the cluster root (the Python Muse's `--cluster-uid`)
    /// and names it until [`Self::with_cluster_name`] gives a name.
    pub fn new(
        identity: MuseIdentity,
        organization: &str,
        cluster_uid: &str,
        namespace: &str,
        interval_ns: u64,
    ) -> Result<Self, DashboardSetError> {
        Ok(Self {
            graph: GraphConfig {
                organization: organization.into(),
                cluster_uid: cluster_uid.into(),
                cluster_name: None,
                namespace: namespace.into(),
                all_namespaces: false,
                expected_interval_ns: interval_ns,
                source_id: identity.source_id(),
                source_revision: identity.source_revision(),
            },
            identity,
            delivery: DashboardDelivery::new(dashboards::dashboard_definitions())?,
            backlog: Backlog::new(BacklogConfig::default(), interval_ns),
            namespace_uid: None,
            detector: Detector::new(),
        })
    }

    /// Bounds the unsent batches kept while no Poet answers.
    #[must_use]
    pub fn with_backlog(mut self, config: BacklogConfig) -> Self {
        self.backlog = Backlog::new(config, self.graph.expected_interval_ns);
        self
    }

    /// Observes every namespace (pods, workloads, warnings), not only its
    /// own; needs the cluster-wide read grants (README, RBAC).
    #[must_use]
    pub fn with_all_namespaces(mut self, all: bool) -> Self {
        self.graph.all_namespaces = all;
        self
    }

    /// Names the cluster root (`k8s.cluster.name`) for people: `k-lab`
    /// instead of its UID. A blank name keeps the UID.
    #[must_use]
    pub fn with_cluster_name(mut self, name: Option<&str>) -> Self {
        self.graph.cluster_name = name
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned);
        self
    }

    pub fn graph_config(&self) -> &GraphConfig {
        &self.graph
    }

    /// The intake request for one snapshot observed at `now`, with the
    /// problems and changes since the previous snapshot as events. It
    /// carries the dashboard definitions until a Poet acknowledges a batch
    /// that did.
    pub fn intake(&mut self, snapshot: &Snapshot, now: u64) -> GraphIntakeRequest {
        let events = self.detector.detect(snapshot, now);
        let mut batch: GraphBatch =
            graph::collect_graph_with(snapshot, &self.graph, now, Some(&self.detector), &events);
        self.delivery.attach(&mut batch);
        GraphIntakeRequest {
            schema_version: GRAPH_INTAKE_SCHEMA_VERSION,
            contract_revision: GRAPH_INTAKE_CONTRACT_REVISION.into(),
            delivery_id: self.identity.delivery_id(now),
            organization: self.graph.organization.clone(),
            owner_id: self.graph.organization.clone(),
            batch,
        }
    }

    /// Records that a Poet acknowledged `request`.
    pub fn acknowledge(&mut self, request: &GraphIntakeRequest) {
        self.delivery.acknowledge(&request.batch);
    }

    /// Queues a request collected at `now`. Returns what queueing it
    /// thinned or dropped to stay within the backlog's byte bound.
    pub fn enqueue(&mut self, request: GraphIntakeRequest, now: u64) -> Loss {
        self.backlog.push(&request, now)
    }

    /// Batches queued.
    pub fn pending(&self) -> usize {
        self.backlog.len()
    }

    /// Compressed bytes queued.
    pub fn pending_bytes(&self) -> usize {
        self.backlog.bytes()
    }

    /// Everything thinned, dropped or rejected since start.
    pub fn lost(&self) -> &Loss {
        self.backlog.total()
    }

    /// Sends up to `max` queued requests oldest first through `send`,
    /// stopping at the first unavailable Poet (that request stays queued).
    /// Pass the replay pace as `max` so a backlog does not flood Poet. What
    /// the backlog thinned or dropped and no Poet has acknowledged yet rides
    /// along as an event in the request sent. A request a Poet rejects (4xx)
    /// is dropped and counted: any Poet would reject it again.
    pub async fn send_pending<F, Fut>(
        &mut self,
        max: usize,
        mut send: F,
    ) -> Result<usize, ih_muse_core::MuseError>
    where
        F: FnMut(GraphIntakeRequest) -> Fut,
        Fut: std::future::Future<Output = Result<(), ih_muse_core::MuseError>>,
    {
        let mut sent = 0;
        while sent < max {
            let Some(mut request) = self.backlog.front() else {
                break;
            };
            let report = self.backlog.take_unreported();
            if let Some(loss) = &report {
                request.batch.events.push(self.loss_event(loss));
            }
            let result = send(request.clone()).await;
            if let (Err(_), Some(loss)) = (&result, &report) {
                self.backlog.restore_unreported(loss);
            }
            match result {
                Ok(()) => {
                    self.acknowledge(&request);
                    self.backlog.pop_acknowledged();
                    sent += 1;
                }
                Err(ih_muse_core::MuseError::Validation(message)) => {
                    self.backlog.pop_rejected();
                    return Err(ih_muse_core::MuseError::Validation(format!(
                        "dropped rejected batch {}: {message}",
                        request.delivery_id
                    )));
                }
                Err(error) => return Err(error),
            }
        }
        Ok(sent)
    }

    /// The event on the cluster that tells Poet, over the affected window,
    /// how many collection intervals were thinned or dropped and how many
    /// events were lost, with the totals since start.
    fn loss_event(&self, loss: &Loss) -> Event {
        let total = self.backlog.total();
        let summary = format!(
            "No Poet answered and the unsent backlog reached its {} byte bound: \
             {} collection interval(s) thinned (kept at a coarser resolution) \
             and {} dropped, with {} event(s)",
            self.backlog.config().max_bytes,
            loss.thinned_intervals,
            loss.dropped_intervals,
            loss.dropped_events
        );
        let attributes = BTreeMap::from([
            (
                "event.name".to_string(),
                AttributeValue::String(BACKLOG_EVENT_NAME.into()),
            ),
            (
                "ih.event.severity".into(),
                AttributeValue::String("warning".into()),
            ),
            (
                "k8s.cluster.name".into(),
                AttributeValue::String(self.cluster_label().into()),
            ),
            (
                "ih.muse.backlog.thinned_intervals".into(),
                AttributeValue::U64(loss.thinned_intervals),
            ),
            (
                "ih.muse.backlog.dropped_intervals".into(),
                AttributeValue::U64(loss.dropped_intervals),
            ),
            (
                "ih.muse.backlog.dropped_events".into(),
                AttributeValue::U64(loss.dropped_events),
            ),
            (
                "ih.muse.backlog.thinned_intervals_total".into(),
                AttributeValue::U64(total.thinned_intervals),
            ),
            (
                "ih.muse.backlog.dropped_intervals_total".into(),
                AttributeValue::U64(total.dropped_intervals),
            ),
            (
                "ih.muse.backlog.dropped_events_total".into(),
                AttributeValue::U64(total.dropped_events),
            ),
            (
                "ih.muse.backlog.max_bytes".into(),
                AttributeValue::U64(self.backlog.config().max_bytes as u64),
            ),
        ]);
        let organization = OrganizationId(self.graph.organization.clone());
        Event {
            entity: EntityKey {
                organization,
                identity: EntityIdentity::Cluster {
                    cluster_uid: self.graph.cluster_uid.clone(),
                },
            },
            kind: EventKind::Domain,
            event_id: format!(
                "k8s:{}:backlog:{}-{}",
                self.graph.cluster_uid, loss.from_unix_nano, loss.to_unix_nano
            ),
            time: TimeRange {
                from_unix_nano: loss.from_unix_nano.max(1),
                to_unix_nano: loss.to_unix_nano.max(loss.from_unix_nano.max(1) + 1),
            },
            scope: InstrumentationScope {
                name: graph::EVENT_SCOPE_NAME.into(),
                version: Some(graph::EVENT_SCOPE_VERSION.into()),
                schema_url: None,
                attributes: BTreeMap::new(),
            },
            attributes,
            body: Some(AttributeValue::String(summary)),
            provenance: Provenance {
                source_id: self.graph.source_id.clone(),
                source_revision: self.graph.source_revision.clone(),
                observed_at_unix_nano: loss.to_unix_nano,
                join_status: JoinStatus::Resolved,
            },
        }
    }

    fn cluster_label(&self) -> &str {
        self.graph
            .cluster_name
            .as_deref()
            .unwrap_or(&self.graph.cluster_uid)
    }

    /// Reads one snapshot. Nodes and pods are required; metrics-server,
    /// namespaces, workloads, ReplicaSets, warnings and the namespace UID
    /// are optional (their absence becomes gaps or fewer elements and
    /// events, and is reported through `warn`). The namespace UID is read
    /// until it succeeds once.
    pub async fn collect(
        &mut self,
        api: &KubeApi,
        warn: impl Fn(String),
    ) -> Result<Snapshot, ApiError> {
        if self.graph.all_namespaces {
            return self.collect_all(api, warn).await;
        }
        let namespace = self.graph.namespace.clone();
        if self.namespace_uid.is_none() {
            match api.namespace_uid(&namespace).await {
                Ok(uid) => self.namespace_uid = Some(uid),
                Err(error) => warn(format!("namespace {namespace} UID unavailable: {error}")),
            }
        }
        let nodes = api.nodes().await?;
        let node_metrics = api
            .node_metrics()
            .await
            .map_err(|error| warn(format!("node metrics unavailable: {error}")))
            .ok();
        let pods = api.pods(&namespace).await?;
        let pod_metrics = api
            .pod_metrics(&namespace)
            .await
            .map_err(|error| warn(format!("pod metrics unavailable: {error}")))
            .ok();
        let scope = Some(namespace.as_str());
        let optional = |what: &str, error: ApiError| warn(format!("{what} unavailable: {error}"));
        Ok(Snapshot {
            nodes,
            node_metrics,
            pods,
            pod_metrics,
            namespace_uid: self.namespace_uid.clone(),
            namespaces: Vec::new(),
            deployments: api
                .deployments(scope)
                .await
                .map_err(|error| optional("deployments", error))
                .unwrap_or_default(),
            statefulsets: api
                .statefulsets(scope)
                .await
                .map_err(|error| optional("statefulsets", error))
                .unwrap_or_default(),
            replicasets: api
                .replicasets(scope)
                .await
                .map_err(|error| optional("replicasets", error))
                .unwrap_or_default(),
            warnings: api
                .warning_events(scope)
                .await
                .map_err(|error| optional("warning events", error))
                .ok(),
        })
    }

    /// [`Self::collect`] over every namespace.
    async fn collect_all(
        &mut self,
        api: &KubeApi,
        warn: impl Fn(String),
    ) -> Result<Snapshot, ApiError> {
        let optional = |what: &str, error: ApiError| warn(format!("{what} unavailable: {error}"));
        let nodes = api.nodes().await?;
        let pods = api.all_pods().await?;
        let namespaces = api
            .namespaces()
            .await
            .map_err(|error| optional("namespaces", error))
            .unwrap_or_default();
        Ok(Snapshot {
            nodes,
            node_metrics: api
                .node_metrics()
                .await
                .map_err(|error| optional("node metrics", error))
                .ok(),
            pods,
            pod_metrics: api
                .all_pod_metrics()
                .await
                .map_err(|error| optional("pod metrics", error))
                .ok(),
            namespace_uid: None,
            namespaces,
            deployments: api
                .deployments(None)
                .await
                .map_err(|error| optional("deployments", error))
                .unwrap_or_default(),
            statefulsets: api
                .statefulsets(None)
                .await
                .map_err(|error| optional("statefulsets", error))
                .unwrap_or_default(),
            replicasets: api
                .replicasets(None)
                .await
                .map_err(|error| optional("replicasets", error))
                .unwrap_or_default(),
            warnings: api
                .warning_events(None)
                .await
                .map_err(|error| optional("warning events", error))
                .ok(),
        })
    }
}
