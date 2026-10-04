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
pub mod dashboards;
pub mod events;
pub mod graph;
pub mod identity;
pub mod model;

use std::collections::VecDeque;

use ih_muse::dashboards::{DashboardDelivery, DashboardSetError};
use ih_muse_proto::{
    GraphBatch, GraphIntakeRequest, GRAPH_INTAKE_CONTRACT_REVISION, GRAPH_INTAKE_SCHEMA_VERSION,
};

use crate::api::{ApiError, KubeApi};
use crate::events::Detector;
use crate::graph::{GraphConfig, Snapshot};
use crate::identity::MuseIdentity;

/// Batches kept while no Poet answers: 10 minutes at the 5 s cadence.
pub const MAX_PENDING_BATCHES: usize = 120;

/// The Muse between collections: who it is, what it sends, what is unsent.
pub struct K8sMuse {
    identity: MuseIdentity,
    graph: GraphConfig,
    delivery: DashboardDelivery,
    pending: VecDeque<GraphIntakeRequest>,
    dropped: u64,
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
            pending: VecDeque::new(),
            dropped: 0,
            namespace_uid: None,
            detector: Detector::new(),
        })
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

    /// Queues a request; returns how many oldest ones were dropped to stay
    /// within [`MAX_PENDING_BATCHES`].
    pub fn enqueue(&mut self, request: GraphIntakeRequest) -> usize {
        self.pending.push_back(request);
        let mut dropped = 0;
        while self.pending.len() > MAX_PENDING_BATCHES {
            self.pending.pop_front();
            dropped += 1;
        }
        self.dropped += dropped as u64;
        dropped
    }

    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Sends queued requests oldest first through `send`, stopping at the
    /// first unavailable Poet (that request stays queued). A request a Poet
    /// rejects (4xx) is dropped: any Poet would reject it again.
    pub async fn send_pending<F, Fut>(
        &mut self,
        mut send: F,
    ) -> Result<usize, ih_muse_core::MuseError>
    where
        F: FnMut(GraphIntakeRequest) -> Fut,
        Fut: std::future::Future<Output = Result<(), ih_muse_core::MuseError>>,
    {
        let mut sent = 0;
        while let Some(request) = self.pending.front().cloned() {
            match send(request.clone()).await {
                Ok(()) => {
                    self.acknowledge(&request);
                    self.pending.pop_front();
                    sent += 1;
                }
                Err(ih_muse_core::MuseError::Validation(message)) => {
                    self.pending.pop_front();
                    self.dropped += 1;
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
