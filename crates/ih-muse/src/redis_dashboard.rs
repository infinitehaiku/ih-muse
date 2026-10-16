//! The dashboard of a Redis or Valkey server, defined in code by the Muse
//! that reads its `INFO` ([`crate::graph_muse::RedisInfoAdapter`]).
//!
//! It travels to Poet in `GraphBatch.dashboards` until a Poet acknowledges
//! a batch that carried it ([`crate::dashboards::DashboardDelivery`]). It is
//! recognised by the service's role (`ih.database.role = redis`), so it is
//! drawn for every Redis-protocol server this Muse observes; every panel
//! names a metric the Muse sends ([`REDIS_METRICS`]), so a renamed metric
//! breaks the tests, not the dashboard.

use ih_muse_proto::dashboard::{
    AttributeRule, GoldenSignal, PanelAggregation, PanelKind, PanelSize, PanelSpec,
    PanelThresholds, ProfileRecognition,
};
use ih_muse_proto::{DashboardAppliesTo, DashboardBlock, DashboardDefinition};

/// The Muse kind owning the definition.
pub const REDIS_MUSE_KIND: &str = "redis";
/// Id of the Redis/Valkey server dashboard.
pub const REDIS_DASHBOARD_ID: &str = "redis.server";
/// The role attribute and value the dashboard is recognised by.
pub const ROLE_ATTRIBUTE: &str = "ih.database.role";
pub const REDIS_ROLE: &str = "redis";

/// Every metric the Redis observation sends (`append_database_service`).
pub const REDIS_METRICS: [&str; 18] = [
    "database.commands",
    "database.connections",
    "database.waits",
    "database.cache.hits",
    "database.cache.misses",
    "database.cache.evictions",
    "database.storage.bytes",
    "database.persistence.bytes",
    "database.keys",
    "database.memory.resident.bytes",
    "database.memory.limit.bytes",
    "database.connections.rejected",
    "database.keys.expired",
    "database.network.received.bytes",
    "database.network.sent.bytes",
    "database.persistence.unsaved_changes",
    "database.persistence.ok",
    "database.replication.replicas",
];

/// Every dashboard the Redis observation defines.
pub fn redis_dashboard_definitions() -> Vec<DashboardDefinition> {
    vec![redis_dashboard()]
}

/// Commands, memory, clients, keyspace, persistence and replication of
/// one Redis or Valkey server, as the old Redis Muse's built-in profile
/// showed them, with what `INFO` adds.
pub fn redis_dashboard() -> DashboardDefinition {
    use PanelAggregation as A;
    DashboardDefinition {
        id: REDIS_DASHBOARD_ID.into(),
        revision: 1,
        title: "Valkey / Redis".into(),
        description: "Commands, memory, clients, keys, persistence and replication of a Valkey or Redis server, read from its INFO beside the server (never its keys or values).".into(),
        muse_kind: REDIS_MUSE_KIND.into(),
        muse_versions: Some(concat!(">=", env!("CARGO_PKG_VERSION")).into()),
        applies_to: DashboardAppliesTo::Recognition(ProfileRecognition {
            root_attributes: vec![AttributeRule {
                key: ROLE_ATTRIBUTE.into(),
                value: Some(REDIS_ROLE.into()),
            }],
            min_metrics: 1,
            ..ProfileRecognition::default()
        }),
        panels: vec![
            panel("commands", "Commands per second", "database.commands", A::Rate).signal(GoldenSignal::Traffic),
            panel("misses", "Keyspace misses per second", "database.cache.misses", A::Rate).signal(GoldenSignal::Errors),
            panel("hits", "Keyspace hits per second", "database.cache.hits", A::Rate),
            panel("memory", "Memory used", "database.storage.bytes", A::Last).signal(GoldenSignal::Saturation),
            panel("resident", "Memory held (resident)", "database.memory.resident.bytes", A::Last),
            panel("limit", "Memory limit (0: none)", "database.memory.limit.bytes", A::Last)
                .kind(PanelKind::Stat)
                .size(3, 2),
            panel("evictions", "Keys evicted per second", "database.cache.evictions", A::Rate),
            panel("clients", "Connected clients", "database.connections", A::Last),
            panel("blocked", "Blocked clients", "database.waits", A::Last),
            panel("rejected", "Connections refused per second", "database.connections.rejected", A::Rate),
            panel("keys", "Keys", "database.keys", A::Last),
            panel("expired", "Keys expired per second", "database.keys.expired", A::Rate),
            panel("received", "Bytes received per second", "database.network.received.bytes", A::Rate),
            panel("sent", "Bytes sent per second", "database.network.sent.bytes", A::Rate),
            panel("persistence_ok", "Last save and append succeeded (1: yes)", "database.persistence.ok", A::Min)
                .kind(PanelKind::Stat)
                .size(3, 2)
                .thresholds(PanelThresholds { warning: 0.5, critical: 0.5, higher_is_worse: false }),
            panel("unsaved", "Writes since the last snapshot", "database.persistence.unsaved_changes", A::Last),
            panel("persistence", "Append-only file size", "database.persistence.bytes", A::Last),
            panel("replicas", "Connected replicas", "database.replication.replicas", A::Last)
                .kind(PanelKind::Stat)
                .size(3, 2),
        ],
        blocks: vec![
            block("Commands", "What clients ask, and how often a key they ask for is missing.", &["commands", "hits", "misses"]),
            block("Memory", "Used is what the data takes; resident is what the system gives the process. Evictions start when used reaches the limit.", &["memory", "resident", "evictions", "limit"]),
            block("Clients", "Connected and blocked clients; refused ones mean the client limit was reached.", &["clients", "blocked", "rejected", "received", "sent"]),
            block("Keys", "Keys stored across every database, and those removed when their time to live ran out.", &["keys", "expired"]),
            block("Persistence and replication", "A failed save or append shows 0. Unsaved writes are lost if the server stops before the next snapshot.", &["persistence_ok", "replicas", "unsaved", "persistence"]),
        ],
        columns: Some(3),
    }
}

fn block(label: &str, text: &str, panels: &[&str]) -> DashboardBlock {
    DashboardBlock {
        label: label.into(),
        text: (!text.is_empty()).then(|| text.into()),
        panels: panels.iter().map(|panel| panel.to_string()).collect(),
        ..Default::default()
    }
}

fn panel(id: &str, title: &str, metric: &str, aggregation: PanelAggregation) -> PanelSpec {
    PanelSpec {
        id: id.into(),
        title: title.into(),
        metric: metric.into(),
        aggregation,
        ..Default::default()
    }
}

/// Small builder steps that keep the panel list readable.
trait PanelSpecExt {
    fn signal(self, signal: GoldenSignal) -> Self;
    fn kind(self, kind: PanelKind) -> Self;
    fn size(self, w: u8, h: u8) -> Self;
    fn thresholds(self, thresholds: PanelThresholds) -> Self;
}

impl PanelSpecExt for PanelSpec {
    fn signal(mut self, signal: GoldenSignal) -> Self {
        self.signal = Some(signal);
        self
    }

    fn kind(mut self, kind: PanelKind) -> Self {
        self.kind = kind;
        self
    }

    fn size(mut self, w: u8, h: u8) -> Self {
        self.size = Some(PanelSize { w, h });
        self
    }

    fn thresholds(mut self, thresholds: PanelThresholds) -> Self {
        self.thresholds = Some(thresholds);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph_muse::{DatabaseService, GraphMuse, GraphMuseConfig, RedisInfoAdapter};
    use std::collections::BTreeSet;

    /// A Valkey 8 `INFO ALL` excerpt (synthetic numbers).
    const INFO: &[u8] = b"# Server\r\nredis_version:7.2.4\r\nvalkey_version:8.0.1\r\n# Clients\r\nconnected_clients:4\r\nblocked_clients:1\r\n# Memory\r\nused_memory:1048576\r\nused_memory_rss:4194304\r\nmaxmemory:0\r\n# Persistence\r\nrdb_changes_since_last_save:12\r\nrdb_last_bgsave_status:ok\r\naof_enabled:1\r\naof_last_write_status:ok\r\naof_current_size:2048\r\n# Stats\r\ntotal_commands_processed:900\r\nrejected_connections:0\r\nexpired_keys:5\r\nevicted_keys:0\r\nkeyspace_hits:80\r\nkeyspace_misses:20\r\ntotal_net_input_bytes:10000\r\ntotal_net_output_bytes:30000\r\n# Replication\r\nrole:master\r\nconnected_slaves:0\r\n# Keyspace\r\ndb0:keys=3,expires=1,avg_ttl=0\r\ndb1:keys=2,expires=0,avg_ttl=0\r\n";

    #[test]
    fn the_dashboard_is_valid_and_reads_only_what_the_muse_sends() {
        let definition = redis_dashboard();
        crate::dashboards::check_definitions(&redis_dashboard_definitions()).unwrap();
        let config = GraphMuseConfig {
            organization: "org".into(),
            owner_id: "org".into(),
            source_id: "valkey-muse".into(),
            environment_id: "env".into(),
            host_id: "host".into(),
            kubernetes: Default::default(),
            rustvello_runner: None,
            max_processes: 1,
        };
        let muse = GraphMuse::new(config).unwrap();
        let mut batch = ih_muse_proto::GraphBatch {
            schema_version: ih_muse_proto::GRAPH_SCHEMA_VERSION,
            contract_revision: ih_muse_proto::GRAPH_CONTRACT_REVISION.into(),
            entities: Vec::new(),
            relations: Vec::new(),
            observations: Vec::new(),
            events: Vec::new(),
            derivations: Vec::new(),
            availability: Vec::new(),
            dashboards: Vec::new(),
        };
        let snapshot = RedisInfoAdapter::parse(INFO).unwrap();
        muse.append_database_service(
            &mut batch,
            DatabaseService {
                role: REDIS_ROLE,
                service_name: "valkey",
                instance_id: "valkey-0",
                process_pid: None,
            },
            &snapshot,
        )
        .unwrap();
        batch.validate().unwrap();
        let measured: BTreeSet<&str> = batch
            .observations
            .iter()
            .filter(|o| matches!(o.value, ih_muse_proto::MetricObservation::Measured { .. }))
            .map(|o| o.descriptor.name.as_str())
            .collect();
        // Every metric the Muse lists is measured from a full INFO reply
        // (a primary has no link to a primary).
        assert_eq!(measured, REDIS_METRICS.iter().copied().collect());
        for panel in &definition.panels {
            assert!(
                REDIS_METRICS.contains(&panel.metric.as_str()),
                "panel {} reads {}, which the Muse does not send",
                panel.id,
                panel.metric
            );
        }
        // The service carries the role the dashboard is recognised by and
        // the name people read.
        let service = &batch.entities[0];
        assert_eq!(
            service.attributes.get(ROLE_ATTRIBUTE),
            Some(&ih_muse_proto::AttributeValue::String(REDIS_ROLE.into()))
        );
        assert!(matches!(
            &service.key.identity,
            ih_muse_proto::EntityIdentity::Service { service_name, .. } if service_name == "valkey"
        ));
    }
}
