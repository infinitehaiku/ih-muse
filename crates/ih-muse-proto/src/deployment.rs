//! Deployment vocabulary: what a deployment system (a GitOps controller,
//! a CD pipeline) did, as graph entities, relations, events and metrics.
//!
//! One vocabulary for every deployment Muse (Piceli first; Argo CD, Flux and
//! others later), so Poet, Kabuki and AGS read every deployment system the
//! same way. Names follow the OpenTelemetry semantic conventions where they
//! exist (`deployment.*`, `cicd.*`, `vcs.*`, `k8s.*`, `oci.*`); the rest are
//! `deployment.*` names of our own, listed here.
//!
//! # Entities
//!
//! Every entity is an [`EntityIdentity::Other`] whose `namespace` is its kind
//! and whose `id` is built by [`DeploymentScope`], so two Muses observing the
//! same system name the same entities:
//!
//! | Kind | Id | Is |
//! | --- | --- | --- |
//! | [`SYSTEM_KIND`] | `{system}:{instance}` | One installed deployment system (`piceli:k-lab`). |
//! | [`ENVIRONMENT_KIND`] | `{system}:{instance}/{environment}` | An environment it deploys (`main`, `rc`, a branch). |
//! | [`RELEASE_KIND`] | `{system}:{instance}/{environment}/{release id}` | One deploy attempt of an environment: its sources and commits, components, checks, approver. |
//! | [`COMPONENT_KIND`] | `{system}:{instance}/{environment}/{component}` | A deployable part (an image) of an environment. |
//!
//! Each carries `display.name` (what people read) and the attributes below.
//!
//! # Relations
//!
//! - system `contains` environment; environment `contains` release and
//!   component (an additive tree: nothing is counted twice);
//! - environment `represents` the Kubernetes namespace it deploys into, and
//!   a Kubernetes workload (Deployment, StatefulSet) `depends_on` the
//!   component whose image it runs;
//! - system `depends_on` the workload the controller itself runs as, so its
//!   pod's restarts (seen by the Kubernetes Muse) and the system's outage
//!   (seen by the deployment Muse) are one click apart and never reported
//!   twice.
//!
//! Kubernetes entities are keyed exactly as the Kubernetes Muse keys them
//! ([`kubernetes_key`]: the cluster UID and the object's UID), so both Muses
//! meet on the same elements. Every relation carries [`RELATION_ROLE`].
//!
//! # Events
//!
//! [`EventKind::Domain`] events with `event.name` ([`EVENT_NAME`]) one of the
//! `*_EVENT` constants, `ih.event.severity` ([`SEVERITY`]), a one-line
//! summary as body, and a stable id from [`DeploymentScope::event_id`]
//! (`piceli:k-lab:main/<run id>:rolled`): the system, the environment, the
//! release and the event, never the time it was observed. The names share
//! the `deployment.` prefix whatever the system, so one marker query
//! (`kind_prefix=deployment.`) lists every system's deploys and
//! [`SYSTEM_NAME`] tells them apart. Provenance is stamped with the event's own time, so a resent
//! event is byte-identical and Poet stores it once.
//!
//! Rollouts and rollbacks are also sent on every Kubernetes workload they
//! changed, so each affected graph shows the marker.
//!
//! # Metrics
//!
//! On the environment: [`DURATION_METRIC`], [`BUILD_DURATION_METRIC`],
//! [`CHECKS_PASSED_METRIC`] and [`CHECKS_FAILED_METRIC`] at each finished
//! release's end, and [`SINCE_SUCCESS_METRIC`] on every collection. On the
//! system: [`SYSTEM_UP_METRIC`].

use std::collections::BTreeMap;

use crate::graph::{
    Entity, EntityIdentity, EntityKey, Event, EventKind, JoinStatus, OrganizationId, Provenance,
    RelationKind, TemporalRelation,
};
use crate::telemetry::{AttributeValue, InstrumentationScope, TimeRange};

// ------------------------------------------------------------ entity kinds

/// Entity kind of an installed deployment system.
pub const SYSTEM_KIND: &str = "deployment.system";
/// Entity kind of an environment a system deploys.
pub const ENVIRONMENT_KIND: &str = "deployment.environment";
/// Entity kind of one deploy attempt (a release) of an environment.
pub const RELEASE_KIND: &str = "deployment.release";
/// Entity kind of a deployable component (an image) of an environment.
pub const COMPONENT_KIND: &str = "deployment.component";

// -------------------------------------------------------------- attributes

/// `event.name` (OpenTelemetry): which event this is.
pub const EVENT_NAME: &str = "event.name";
/// How serious an event is: `info`, `warning` or `error` (shared with the
/// Kubernetes Muse's events).
pub const SEVERITY: &str = "ih.event.severity";
/// What people read for an entity (Poet's `entity_label` takes it first).
pub const DISPLAY_NAME: &str = "display.name";
/// The deployment system's product: `piceli`, `argocd`, `flux`.
pub const SYSTEM_NAME: &str = "deployment.system.name";
/// The installation of the system (the cluster or host it runs on).
pub const SYSTEM_INSTANCE: &str = "deployment.system.instance";
/// The system's version.
pub const SYSTEM_VERSION: &str = "deployment.system.version";
/// The environment (OpenTelemetry `deployment.environment.name`).
pub const ENVIRONMENT_NAME: &str = "deployment.environment.name";
/// The release's id (OpenTelemetry `deployment.id`): Piceli's run id, an
/// Argo CD sync's history id.
pub const DEPLOYMENT_ID: &str = "deployment.id";
/// The release's name (OpenTelemetry `deployment.name`), when the system
/// names releases apart from their id.
pub const DEPLOYMENT_NAME: &str = "deployment.name";
/// `succeeded` or `failed` (OpenTelemetry `deployment.status`), once finished.
pub const DEPLOYMENT_STATUS: &str = "deployment.status";
/// The system's own state word for the release (`deployed`, `failed`,
/// `rolled-back`, `approval-required`, `running`).
pub const DEPLOYMENT_STATE: &str = "deployment.state";
/// What the release did: `deployed` (applied), `unchanged`, `verified`.
pub const DEPLOYMENT_ACTION: &str = "deployment.action";
/// Why it did not succeed: the system's error code.
pub const DEPLOYMENT_REASON: &str = "deployment.reason";
/// What started it: `push <source>/<branch>`, `tag v1.2`, `sync`, `promote`.
pub const DEPLOYMENT_TRIGGER: &str = "deployment.trigger";
/// Who approved it: `policy`, `cli`, `ui`, `request` (an approval whose
/// channel is unknown) or `none`.
pub const APPROVED_VIA: &str = "deployment.approval.via";
/// The approved plan's hash.
pub const PLAN_HASH: &str = "deployment.plan.hash";
/// Components a release rolled (comma-separated, sorted).
pub const ROLLED: &str = "deployment.components.rolled";
/// Components a release built (comma-separated, sorted).
pub const BUILT: &str = "deployment.components.built";
/// A component's name (on component entities and per-component events).
pub const COMPONENT_NAME: &str = "deployment.component.name";
/// The release a rollback restored.
pub const ROLLBACK_TO: &str = "deployment.rollback.release";
/// Checks run, passed and failed by a release.
pub const CHECKS_TOTAL: &str = "deployment.checks.total";
pub const CHECKS_PASSED: &str = "deployment.checks.passed";
pub const CHECKS_FAILED: &str = "deployment.checks.failed";
/// The failing checks: `name: detail` lines.
pub const CHECKS_FAILED_DETAIL: &str = "deployment.checks.failed.detail";
/// The last lines of a failed build's log (scrubbed by the system).
pub const LOG_TAIL: &str = "deployment.log.tail";
/// Objects a release deleted (`Kind/name`, comma-separated).
pub const PRUNED: &str = "deployment.pruned";
/// Wall time of the release, in seconds.
pub const DURATION_SECONDS: &str = "deployment.duration.seconds";
/// `true` when an event's time is computed (a run's start plus the
/// durations of its earlier stages) rather than recorded by the system.
pub const TIME_DERIVED: &str = "deployment.time.derived";
/// The pipeline stage of a stage event (OpenTelemetry `cicd.pipeline.task.name`).
pub const STAGE: &str = "cicd.pipeline.task.name";
/// The release's run id (OpenTelemetry `cicd.pipeline.run.id`).
pub const PIPELINE_RUN_ID: &str = "cicd.pipeline.run.id";
/// `success`, `failure`, `error` or `cancellation` (OpenTelemetry `cicd.pipeline.result`).
pub const PIPELINE_RESULT: &str = "cicd.pipeline.result";
/// The source repository's name (OpenTelemetry `vcs.repository.name`).
pub const VCS_REPOSITORY_NAME: &str = "vcs.repository.name";
/// The source repository's URL (OpenTelemetry `vcs.repository.url.full`).
pub const VCS_REPOSITORY_URL: &str = "vcs.repository.url.full";
/// The followed ref (OpenTelemetry `vcs.ref.head.name`).
pub const VCS_REF: &str = "vcs.ref.head.name";
/// The deployed commit (OpenTelemetry `vcs.ref.head.revision`).
pub const VCS_REVISION: &str = "vcs.ref.head.revision";
/// Every source of a release: `name=commit` (comma-separated, sorted).
pub const SOURCES: &str = "deployment.sources";
/// A component's image digest (OpenTelemetry `oci.manifest.digest`).
pub const IMAGE_DIGEST: &str = "oci.manifest.digest";
/// A component's image reference (OpenTelemetry `container.image.name`).
pub const IMAGE_NAME: &str = "container.image.name";
/// Kubernetes names, as the Kubernetes Muse writes them.
pub const K8S_CLUSTER_NAME: &str = "k8s.cluster.name";
pub const K8S_NAMESPACE_NAME: &str = "k8s.namespace.name";
pub const K8S_DEPLOYMENT_NAME: &str = "k8s.deployment.name";
pub const K8S_STATEFULSET_NAME: &str = "k8s.statefulset.name";
/// What a relation means for deployments: `deploys` (environment to its
/// namespace), `runs` (workload to the component it runs), `hosts`
/// (system to the workload it runs as).
pub const RELATION_ROLE: &str = "deployment.relation.role";

// ------------------------------------------------------------------ events

/// A component build began.
pub const BUILD_STARTED_EVENT: &str = "deployment.build.started";
/// A component build finished and its image was delivered.
pub const BUILD_FINISHED_EVENT: &str = "deployment.build.finished";
/// A component build failed; [`LOG_TAIL`] holds its log's last lines.
pub const BUILD_FAILED_EVENT: &str = "deployment.build.failed";
/// A plan was computed and waits for approval ([`PLAN_HASH`]).
pub const APPROVAL_REQUIRED_EVENT: &str = "deployment.approval.required";
/// A release was approved ([`APPROVED_VIA`]: policy, cli or ui).
pub const APPROVED_EVENT: &str = "deployment.approved";
/// The apply of a release began.
pub const APPLY_STARTED_EVENT: &str = "deployment.apply.started";
/// A release rolled components ([`ROLLED`]); also sent on each workload.
pub const ROLLED_EVENT: &str = "deployment.rolled";
/// A release finished ([`DEPLOYMENT_STATUS`], [`DEPLOYMENT_STATE`]).
pub const FINISHED_EVENT: &str = "deployment.finished";
/// A release's checks passed.
pub const CHECKS_PASSED_EVENT: &str = "deployment.checks.passed";
/// A release's checks failed ([`CHECKS_FAILED_DETAIL`]).
pub const CHECKS_FAILED_EVENT: &str = "deployment.checks.failed";
/// A release was rolled back ([`ROLLBACK_TO`]); also sent on each workload.
pub const ROLLBACK_EVENT: &str = "deployment.rollback";
/// An environment was stopped (scaled to zero, kept).
pub const ENVIRONMENT_STOPPED_EVENT: &str = "deployment.environment.stopped";
/// An environment was started again.
pub const ENVIRONMENT_STARTED_EVENT: &str = "deployment.environment.started";
/// A release deleted objects it no longer declares ([`PRUNED`]).
pub const PRUNED_EVENT: &str = "deployment.pruned";
/// The deployment system stopped working (its status went stale, or its
/// workload has no available replica).
pub const SYSTEM_DOWN_EVENT: &str = "deployment.system.down";
/// The deployment system works again.
pub const SYSTEM_UP_EVENT: &str = "deployment.system.up";

/// Every event name of the vocabulary.
pub const EVENT_NAMES: [&str; 16] = [
    BUILD_STARTED_EVENT,
    BUILD_FINISHED_EVENT,
    BUILD_FAILED_EVENT,
    APPROVAL_REQUIRED_EVENT,
    APPROVED_EVENT,
    APPLY_STARTED_EVENT,
    ROLLED_EVENT,
    FINISHED_EVENT,
    CHECKS_PASSED_EVENT,
    CHECKS_FAILED_EVENT,
    ROLLBACK_EVENT,
    ENVIRONMENT_STOPPED_EVENT,
    ENVIRONMENT_STARTED_EVENT,
    PRUNED_EVENT,
    SYSTEM_DOWN_EVENT,
    SYSTEM_UP_EVENT,
];

/// The events drawn as deploy markers on the graphs of what they changed.
pub const MARKER_EVENT_NAMES: [&str; 4] = [
    ROLLED_EVENT,
    ROLLBACK_EVENT,
    CHECKS_FAILED_EVENT,
    SYSTEM_DOWN_EVENT,
];

// ----------------------------------------------------------------- metrics

/// Wall time of a finished release, seconds (gauge on the environment at
/// the release's end; attribute [`PIPELINE_RESULT`]).
pub const DURATION_METRIC: &str = "deployment.duration";
/// Build stage time of a finished release, seconds.
pub const BUILD_DURATION_METRIC: &str = "deployment.build.duration";
/// Checks a finished release passed (gauge at its end).
pub const CHECKS_PASSED_METRIC: &str = "deployment.checks.passed";
/// Checks a finished release failed (gauge at its end).
pub const CHECKS_FAILED_METRIC: &str = "deployment.checks.failed";
/// Seconds since the environment's last successful release ended.
pub const SINCE_SUCCESS_METRIC: &str = "deployment.since_last_success";
/// 1 while the deployment system works, 0 while it is down.
pub const SYSTEM_UP_METRIC: &str = "deployment.system.up";

/// Finished releases: a delta counter of 1 on the environment at each
/// release's end, with [`PIPELINE_RESULT`], so a dashboard counts deploys by
/// result (moved here from the Piceli Muse).
pub const RELEASES_METRIC: &str = "deployment.releases";

/// Where a marker came from when it was not sent by a deployment Muse:
/// `otlp` for the markers Poet derives from a system's own OpenTelemetry
/// ([`crate::event_mapping`]). A reader that sees both for one environment
/// keeps the OTLP ones from their first release on (a switch from a
/// polling Muse to the system's own OTLP lists each release once).
pub const FEED: &str = "deployment.feed";
/// [`FEED`] of markers derived from OTLP.
pub const FEED_OTLP: &str = "otlp";

/// Instrumentation scope of the vocabulary's events and metrics.
pub const SCOPE_NAME: &str = "ih.deployment";
/// Version of the vocabulary.
pub const SCOPE_VERSION: &str = "1.0.0";

/// How serious a deployment event is.
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

/// One installed deployment system as a Muse reports it: who owns the data
/// (organization), which system and installation, and which Muse speaks.
/// Builds every key, event id and provenance of the vocabulary, so ids stay
/// identical across Muse restarts and between Muses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeploymentScope {
    /// Tenant the Muse's Poet token is scoped to.
    pub organization: String,
    /// The product: `piceli`, `argocd`, `flux`.
    pub system: String,
    /// The installation: a cluster name, or a host.
    pub instance: String,
    /// The Muse's provenance `source_id`.
    pub source_id: String,
    /// The Muse's provenance `source_revision` (its version).
    pub source_revision: String,
}

impl DeploymentScope {
    fn key(&self, kind: &str, id: String) -> EntityKey {
        EntityKey {
            organization: OrganizationId(self.organization.clone()),
            identity: EntityIdentity::Other {
                namespace: kind.into(),
                id,
            },
        }
    }

    /// `{system}:{instance}`.
    pub fn system_id(&self) -> String {
        format!("{}:{}", self.system, self.instance)
    }

    pub fn system_key(&self) -> EntityKey {
        self.key(SYSTEM_KIND, self.system_id())
    }

    pub fn environment_key(&self, environment: &str) -> EntityKey {
        self.key(
            ENVIRONMENT_KIND,
            format!("{}/{environment}", self.system_id()),
        )
    }

    pub fn release_key(&self, environment: &str, release: &str) -> EntityKey {
        self.key(
            RELEASE_KIND,
            format!("{}/{environment}/{release}", self.system_id()),
        )
    }

    pub fn component_key(&self, environment: &str, component: &str) -> EntityKey {
        self.key(
            COMPONENT_KIND,
            format!("{}/{environment}/{component}", self.system_id()),
        )
    }

    /// The stable id of one event, scoped by its source as Poet's marker
    /// contract asks: `{system}:{instance}:{subject}:{what}`, where `subject`
    /// names what it is about (`{environment}/{release}`, or `system`) and
    /// `what` the event (`rolled`, `checks`, `rolled/{workload uid}`).
    pub fn event_id(&self, subject: &str, what: &str) -> String {
        format!("{}:{subject}:{what}", self.system_id())
    }

    /// Provenance stamped at `at`: the event's or observation's own time,
    /// never the collection's, so a resend is identical.
    pub fn provenance(&self, at_unix_nano: u64) -> Provenance {
        Provenance {
            source_id: self.source_id.clone(),
            source_revision: self.source_revision.clone(),
            observed_at_unix_nano: at_unix_nano.max(1),
            join_status: JoinStatus::Resolved,
        }
    }

    /// The vocabulary's instrumentation scope.
    pub fn scope(&self) -> InstrumentationScope {
        InstrumentationScope {
            name: SCOPE_NAME.into(),
            version: Some(SCOPE_VERSION.into()),
            schema_url: None,
            attributes: BTreeMap::new(),
        }
    }

    /// An entity alive from `from` on (open-ended while the Muse sees it).
    pub fn entity(
        &self,
        key: EntityKey,
        from_unix_nano: u64,
        to_unix_nano: u64,
        attributes: BTreeMap<String, AttributeValue>,
    ) -> Entity {
        let from = from_unix_nano.max(1);
        Entity {
            key,
            lifetime: TimeRange {
                from_unix_nano: from,
                to_unix_nano: to_unix_nano.max(from + 1),
            },
            attributes,
        }
    }

    /// A relation valid from `from` to `to`, with its [`RELATION_ROLE`].
    pub fn relation(
        &self,
        subject: &EntityKey,
        object: &EntityKey,
        kind: RelationKind,
        role: &str,
        from_unix_nano: u64,
        to_unix_nano: u64,
    ) -> TemporalRelation {
        let from = from_unix_nano.max(1);
        TemporalRelation {
            subject: subject.clone(),
            object: object.clone(),
            kind,
            valid_time: TimeRange {
                from_unix_nano: from,
                to_unix_nano: to_unix_nano.max(from + 1),
            },
            provenance: self.provenance(from),
            attributes: BTreeMap::from([(RELATION_ROLE.into(), string(role))]),
        }
    }

    /// The graph event of `spec` (empty attribute values are dropped).
    pub fn event(&self, spec: DeploymentEvent) -> Event {
        let at = spec.at_unix_nano.max(1);
        let mut values: BTreeMap<String, AttributeValue> = spec
            .attributes
            .into_iter()
            .filter(|(_, value)| !value.is_empty())
            .map(|(key, value)| (key, AttributeValue::String(value)))
            .collect();
        values.insert(EVENT_NAME.into(), string(spec.name));
        values.insert(SEVERITY.into(), string(spec.severity.as_str()));
        values.insert(SYSTEM_NAME.into(), string(&self.system));
        values.insert(SYSTEM_INSTANCE.into(), string(&self.instance));
        Event {
            entity: spec.entity,
            kind: EventKind::Domain,
            event_id: spec.id,
            time: TimeRange {
                from_unix_nano: at,
                to_unix_nano: at + 1,
            },
            scope: self.scope(),
            attributes: values,
            body: Some(AttributeValue::String(spec.summary)),
            provenance: self.provenance(at),
        }
    }
}

/// One deployment event before it becomes a graph [`Event`]: `name` (one of
/// the `*_EVENT` constants) on `entity` at `at_unix_nano`, with its stable
/// `id` ([`DeploymentScope::event_id`]) and a one-line `summary`.
#[derive(Clone, Debug, PartialEq)]
pub struct DeploymentEvent {
    pub entity: EntityKey,
    pub id: String,
    pub name: &'static str,
    pub severity: Severity,
    pub at_unix_nano: u64,
    pub summary: String,
    pub attributes: BTreeMap<String, String>,
}

/// The key the Kubernetes Muse gives a Kubernetes object: `resource_kind`
/// is `namespace`, `deployment`, `statefulset`, `pod` or `node`.
pub fn kubernetes_key(
    organization: &str,
    cluster_uid: &str,
    resource_kind: &str,
    resource_uid: &str,
) -> EntityKey {
    EntityKey {
        organization: OrganizationId(organization.into()),
        identity: EntityIdentity::Kubernetes {
            cluster_uid: cluster_uid.into(),
            resource_kind: resource_kind.into(),
            resource_uid: resource_uid.into(),
        },
    }
}

/// A string attribute value.
pub fn string(value: impl Into<String>) -> AttributeValue {
    AttributeValue::String(value.into())
}

/// `true` when `name` is one of the vocabulary's events.
pub fn is_deployment_event(name: &str) -> bool {
    EVENT_NAMES.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{GraphBatch, GRAPH_CONTRACT_REVISION, GRAPH_SCHEMA_VERSION};

    fn scope() -> DeploymentScope {
        DeploymentScope {
            organization: "org".into(),
            system: "piceli".into(),
            instance: "lab".into(),
            source_id: "ih-muse-piceli:lab".into(),
            source_revision: "0.1.0".into(),
        }
    }

    #[test]
    fn keys_and_event_ids_are_stable_and_distinct() {
        let scope = scope();
        assert_eq!(scope.system_id(), "piceli:lab");
        let env = scope.environment_key("main");
        assert_eq!(
            env.identity,
            EntityIdentity::Other {
                namespace: ENVIRONMENT_KIND.into(),
                id: "piceli:lab/main".into()
            }
        );
        assert_ne!(
            scope.release_key("main", "r1"),
            scope.release_key("rc", "r1")
        );
        assert_ne!(
            scope.component_key("main", "web"),
            scope.release_key("main", "web")
        );
        assert_eq!(
            scope.event_id("main/r1", "rolled"),
            "piceli:lab:main/r1:rolled"
        );
    }

    #[test]
    fn an_event_is_identical_when_built_twice_and_validates_in_a_batch() {
        let scope = scope();
        let env = scope.environment_key("main");
        let build = || {
            scope.event(DeploymentEvent {
                entity: env.clone(),
                id: scope.event_id("main/r1", "rolled"),
                name: ROLLED_EVENT,
                severity: Severity::Info,
                at_unix_nano: 42,
                summary: "main: rolled web".into(),
                attributes: BTreeMap::from([
                    (ROLLED.into(), "web".into()),
                    ("empty".into(), String::new()),
                ]),
            })
        };
        let event = build();
        assert_eq!(event, build());
        assert_eq!(event.provenance.observed_at_unix_nano, 42);
        assert!(!event.attributes.contains_key("empty"));
        assert_eq!(event.attributes[EVENT_NAME], string(ROLLED_EVENT));
        assert_eq!(event.attributes[SEVERITY], string("info"));
        let namespace = kubernetes_key("org", "c1", "namespace", "n1");
        let batch = GraphBatch {
            schema_version: GRAPH_SCHEMA_VERSION,
            contract_revision: GRAPH_CONTRACT_REVISION.into(),
            entities: vec![
                scope.entity(env.clone(), 1, 100, BTreeMap::new()),
                scope.entity(namespace.clone(), 1, 100, BTreeMap::new()),
            ],
            relations: vec![scope.relation(
                &env,
                &namespace,
                RelationKind::Represents,
                "deploys",
                1,
                100,
            )],
            observations: vec![],
            events: vec![event],
            derivations: vec![],
            availability: vec![],
            dashboards: vec![],
        };
        batch.validate().expect("valid batch");
    }

    #[test]
    fn every_marker_event_is_a_vocabulary_event() {
        assert!(MARKER_EVENT_NAMES
            .iter()
            .all(|name| is_deployment_event(name)));
        assert!(!is_deployment_event("k8s.rollout.start"));
        let mut names = EVENT_NAMES.to_vec();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), EVENT_NAMES.len());
    }
}
