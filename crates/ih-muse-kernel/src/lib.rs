//! The kernel-faults Muse: one per node (a DaemonSet), it reads the
//! kernel's lines from the host's journal files (read-only), finds the
//! faults among them (segfaults, general protection faults and other
//! traps, machine-check and EDAC errors, I/O and filesystem errors, OOM
//! kills: [`fault`]) and sends each to the Poets as an event on the node's
//! Kubernetes entity, with cumulative counters per kind and per CPU so a
//! dashboard shows faults per core over time.
//!
//! ```text
//! Cluster -> Node   (contains; the node is the k8s Muse's, by its UID)
//! ```
//!
//! Truthful by construction: an event or a counter point is sent only for a
//! line the kernel wrote, at the line's own time; with no faults nothing is
//! sent but the entities and the dashboard definition, once. The counters
//! count from the start of the current boot: at start the Muse reads the
//! whole boot (the backfill window) to know the totals, and sends only what
//! its [`Cursor`] says no Poet has acknowledged yet, so a restart sends no
//! duplicates. Event ids are derived from the journal's own line ids, so a
//! lost cursor resends the same events, which Poet stores once.

pub mod dashboards;
pub mod fault;
pub mod journal;

use std::collections::BTreeMap;
use std::path::Path;

use ih_muse::dashboards::{DashboardDelivery, DashboardSetError};
use ih_muse_proto::{
    AggregationTemporality, AttributeValue, Entity, EntityIdentity, EntityKey, Event, EventKind,
    GraphBatch, GraphIntakeAnswer, GraphIntakeRequest, InstrumentationScope, JoinStatus,
    MetricDescriptor, MetricInstrument, MetricObservation, MetricUnit, Number, Observation,
    OrganizationId, Provenance, RelationKind, SpatialAggregation, TemporalRelation, TimeRange,
    TypedMetricValue, UnitDisplay, ValueDomain, GRAPH_CONTRACT_REVISION,
    GRAPH_INTAKE_CONTRACT_REVISION, GRAPH_INTAKE_SCHEMA_VERSION, GRAPH_SCHEMA_VERSION,
};
use serde::{Deserialize, Serialize};

use crate::fault::{FaultKind, KernelFault, Parsed};
use crate::journal::KernelRecord;

/// The Muse kind: owner of the `kernel.*` dashboard definitions.
pub const MUSE_KIND: &str = "kernel";
/// The source id prefix: `ih-kernel-muse:<node>`.
pub const SOURCE_PREFIX: &str = "ih-kernel-muse";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Faults since boot, per kind (attribute [`KIND_ATTRIBUTE`]).
pub const FAULTS_METRIC: &str = "ih.kernel.faults";
/// Faults since boot per logical CPU and kind (attributes
/// [`CPU_ATTRIBUTE`], [`KIND_ATTRIBUTE`]), for the lines that name a CPU.
pub const CPU_FAULTS_METRIC: &str = "ih.kernel.cpu.faults";
/// Lines since boot that had a fault's marker but could not be read,
/// per kind: counted, never dropped silently.
pub const SKIPPED_METRIC: &str = "ih.kernel.lines.skipped";
/// Every metric this Muse sends.
pub const METRICS: [&str; 3] = [FAULTS_METRIC, CPU_FAULTS_METRIC, SKIPPED_METRIC];

pub const KIND_ATTRIBUTE: &str = "ih.kernel.fault.kind";
/// OpenTelemetry's logical CPU number attribute.
pub const CPU_ATTRIBUTE: &str = "cpu.logical_number";
/// Scope of observations and events.
pub const SCOPE_NAME: &str = "ih.kernel.faults";
pub const SCOPE_VERSION: &str = "1.0.0";
/// `event.name` of a fault event: `kernel.<kind>`.
pub const EVENT_NAME_PREFIX: &str = "kernel.";

const FOREVER: TimeRange = TimeRange {
    from_unix_nano: 0,
    to_unix_nano: u64::MAX,
};

/// Who this Muse speaks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeConfig {
    pub organization: String,
    /// The cluster root's identity, as the k8s Muse sends it.
    pub cluster_uid: String,
    pub cluster_name: Option<String>,
    /// `k8s.node.name` (the pod's `spec.nodeName`).
    pub node_name: String,
    /// The node's UID: the k8s Muse's node entity is keyed by it.
    pub node_uid: String,
    /// `host.name`: the kernel's host name (usually the node name).
    pub host_name: String,
}

impl NodeConfig {
    pub fn source_id(&self) -> String {
        format!("{SOURCE_PREFIX}:{}", self.node_name)
    }

    fn key(&self, identity: EntityIdentity) -> EntityKey {
        EntityKey {
            organization: OrganizationId(self.organization.clone()),
            identity,
        }
    }

    pub fn cluster_key(&self) -> EntityKey {
        self.key(EntityIdentity::Cluster {
            cluster_uid: self.cluster_uid.clone(),
        })
    }

    pub fn node_key(&self) -> EntityKey {
        self.key(EntityIdentity::Kubernetes {
            cluster_uid: self.cluster_uid.clone(),
            resource_kind: "node".into(),
            resource_uid: self.node_uid.clone(),
        })
    }
}

/// What the Poets have acknowledged: per boot, the newest journal line
/// (per sequence-number space) whose fault was sent. Kept in a file so a
/// restarted Muse sends no line twice.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    pub boot_id: String,
    /// Newest sent `seqnum` per journal `seqnum_id`.
    pub sent: BTreeMap<String, u64>,
}

impl Cursor {
    /// True when `record` was sent before.
    pub fn covers(&self, record: &KernelRecord) -> bool {
        record.boot_id == self.boot_id
            && self
                .sent
                .get(&record.seqnum_id)
                .is_some_and(|seqnum| record.seqnum <= *seqnum)
    }

    fn advance(&mut self, record: &KernelRecord) {
        if self.boot_id != record.boot_id {
            self.boot_id = record.boot_id.clone();
            self.sent.clear();
        }
        let sent = self.sent.entry(record.seqnum_id.clone()).or_insert(0);
        *sent = (*sent).max(record.seqnum);
    }

    /// The cursor in `path`; a missing or unreadable file is an empty cursor
    /// (everything of the boot is sent again, under the same event ids).
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Writes the cursor through a temporary file and a rename.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let temporary = path.with_extension("tmp");
        std::fs::write(&temporary, serde_json::to_vec(self)?)?;
        std::fs::rename(temporary, path)
    }
}

/// One fault line read past the cursor, with the counter values it set.
#[derive(Clone, Debug, PartialEq)]
pub struct NewFault {
    pub record: KernelRecord,
    pub fault: KernelFault,
    /// [`FAULTS_METRIC`] for its kind after it.
    pub kind_total: u64,
    /// [`CPU_FAULTS_METRIC`] for its kind and CPU after it.
    pub cpu_total: Option<(u32, u64)>,
}

/// One line with a fault's marker that could not be read, past the cursor.
#[derive(Clone, Debug, PartialEq)]
pub struct NewSkip {
    pub record: KernelRecord,
    pub kind: FaultKind,
    pub total: u64,
}

/// The Muse between polls: what it counted this boot and what it sent.
pub struct KernelMuse {
    config: NodeConfig,
    delivery: DashboardDelivery,
    /// Acknowledged by a Poet (loaded from and saved to the cursor file).
    acknowledged: Cursor,
    /// Put into a request (queued or sent).
    emitted: Cursor,
    boot_id: Option<String>,
    /// Wall-clock start of the boot (ns): the counters' start time.
    boot_start_unix_nano: Option<u64>,
    kind_totals: BTreeMap<FaultKind, u64>,
    cpu_totals: BTreeMap<(FaultKind, u32), u64>,
    skipped_totals: BTreeMap<FaultKind, u64>,
    /// Entities and definitions were sent and acknowledged once.
    introduced: bool,
}

impl KernelMuse {
    pub fn new(config: NodeConfig, cursor: Cursor) -> Result<Self, DashboardSetError> {
        Ok(Self {
            config,
            delivery: DashboardDelivery::new(dashboards::dashboard_definitions())?,
            emitted: cursor.clone(),
            acknowledged: cursor,
            boot_id: None,
            boot_start_unix_nano: None,
            kind_totals: BTreeMap::new(),
            cpu_totals: BTreeMap::new(),
            skipped_totals: BTreeMap::new(),
            introduced: false,
        })
    }

    pub fn config(&self) -> &NodeConfig {
        &self.config
    }

    /// The cursor every Poet-acknowledged request has advanced.
    pub fn acknowledged_cursor(&self) -> &Cursor {
        &self.acknowledged
    }

    /// Faults counted since boot, per kind.
    pub fn totals(&self) -> &BTreeMap<FaultKind, u64> {
        &self.kind_totals
    }

    /// Faults counted since boot, per kind and CPU.
    pub fn cpu_totals(&self) -> &BTreeMap<(FaultKind, u32), u64> {
        &self.cpu_totals
    }

    /// Counts `records` (one boot's new kernel lines, oldest first) and
    /// returns the faults and unreadable fault lines no request carried yet.
    pub fn ingest(&mut self, records: Vec<KernelRecord>) -> (Vec<NewFault>, Vec<NewSkip>) {
        let mut faults = Vec::new();
        let mut skips = Vec::new();
        for record in records {
            if self.boot_id.as_deref() != Some(record.boot_id.as_str()) {
                self.boot_id = Some(record.boot_id.clone());
                self.boot_start_unix_nano = None;
                self.kind_totals.clear();
                self.cpu_totals.clear();
                self.skipped_totals.clear();
            }
            let boot_start = record
                .realtime_usec
                .saturating_sub(record.monotonic_usec)
                .saturating_mul(1_000);
            self.boot_start_unix_nano = Some(
                self.boot_start_unix_nano
                    .map_or(boot_start, |start| start.min(boot_start)),
            );
            let fresh = !self.emitted.covers(&record);
            match fault::parse_line(&record.message) {
                Parsed::Fault(found) => {
                    let kind_total = {
                        let total = self.kind_totals.entry(found.kind).or_insert(0);
                        *total += 1;
                        *total
                    };
                    let cpu_total = found.fields.cpu.map(|cpu| {
                        let total = self.cpu_totals.entry((found.kind, cpu)).or_insert(0);
                        *total += 1;
                        (cpu, *total)
                    });
                    if fresh {
                        self.emitted.advance(&record);
                        faults.push(NewFault {
                            record,
                            fault: found,
                            kind_total,
                            cpu_total,
                        });
                    }
                }
                Parsed::Malformed(kind) => {
                    let total = self.skipped_totals.entry(kind).or_insert(0);
                    *total += 1;
                    if fresh {
                        self.emitted.advance(&record);
                        skips.push(NewSkip {
                            record,
                            kind,
                            total: *total,
                        });
                    }
                }
                Parsed::NotAFault => {}
            }
        }
        (faults, skips)
    }

    /// The request for what [`Self::ingest`] returned, observed at `now`;
    /// `None` when there is nothing to send (no new fault, no skipped line,
    /// and the entities and dashboard definition were acknowledged).
    pub fn intake(
        &mut self,
        faults: &[NewFault],
        skips: &[NewSkip],
        now: u64,
    ) -> Option<GraphIntakeRequest> {
        if faults.is_empty() && skips.is_empty() && self.introduced && self.delivery.is_delivered()
        {
            return None;
        }
        let mut batch = self.batch(faults, skips, now);
        self.delivery.attach(&mut batch);
        Some(GraphIntakeRequest {
            schema_version: GRAPH_INTAKE_SCHEMA_VERSION,
            contract_revision: GRAPH_INTAKE_CONTRACT_REVISION.into(),
            delivery_id: format!("{}:{now}", self.config.source_id()),
            organization: self.config.organization.clone(),
            owner_id: self.config.organization.clone(),
            batch,
        })
    }

    /// Records that a Poet acknowledged `request`: its lines are sent for
    /// good (the cursor advances) and its definitions delivered.
    pub fn acknowledge(&mut self, request: &GraphIntakeRequest, answer: &GraphIntakeAnswer) {
        self.delivery.acknowledge(&request.batch, answer);
        self.introduced = true;
        for event in &request.batch.events {
            if let Some((boot_id, seqnum_id, seqnum)) = parse_event_id(&event.event_id) {
                let record = KernelRecord {
                    realtime_usec: 0,
                    monotonic_usec: 0,
                    boot_id,
                    seqnum_id,
                    seqnum,
                    message: String::new(),
                };
                self.acknowledged.advance(&record);
            }
        }
    }

    fn provenance(&self, observed: u64) -> Provenance {
        Provenance {
            source_id: self.config.source_id(),
            source_revision: VERSION.into(),
            observed_at_unix_nano: observed,
            join_status: JoinStatus::Resolved,
        }
    }

    fn scope() -> InstrumentationScope {
        InstrumentationScope {
            name: SCOPE_NAME.into(),
            version: Some(SCOPE_VERSION.into()),
            schema_url: None,
            attributes: BTreeMap::new(),
        }
    }

    fn batch(&self, faults: &[NewFault], skips: &[NewSkip], now: u64) -> GraphBatch {
        let cluster = self.config.cluster_key();
        let node = self.config.node_key();
        let mut cluster_attributes = BTreeMap::new();
        if let Some(name) = &self.config.cluster_name {
            cluster_attributes.insert("k8s.cluster.name".into(), string(name));
        }
        let entities = vec![
            Entity {
                key: cluster.clone(),
                lifetime: FOREVER,
                attributes: cluster_attributes,
            },
            Entity {
                key: node.clone(),
                lifetime: FOREVER,
                attributes: BTreeMap::from([
                    ("k8s.node.name".into(), string(&self.config.node_name)),
                    ("host.name".into(), string(&self.config.host_name)),
                ]),
            },
        ];
        let relations = vec![TemporalRelation {
            subject: cluster,
            object: node.clone(),
            kind: RelationKind::Contains,
            valid_time: FOREVER,
            provenance: self.provenance(now),
            attributes: BTreeMap::new(),
        }];
        let mut observations = Vec::new();
        let mut events = Vec::new();
        let start = self.boot_start_unix_nano;
        for new in faults {
            let time = record_time(&new.record);
            let kind = new.fault.kind.as_str();
            observations.push(self.counter(
                &node,
                FAULTS_METRIC,
                new.kind_total,
                &[(KIND_ATTRIBUTE, kind.to_string())],
                start,
                time,
            ));
            if let Some((cpu, total)) = new.cpu_total {
                observations.push(self.counter(
                    &node,
                    CPU_FAULTS_METRIC,
                    total,
                    &[
                        (KIND_ATTRIBUTE, kind.to_string()),
                        (CPU_ATTRIBUTE, cpu.to_string()),
                    ],
                    start,
                    time,
                ));
            }
            events.push(self.event(&node, new, time));
        }
        for skip in skips {
            observations.push(self.counter(
                &node,
                SKIPPED_METRIC,
                skip.total,
                &[(KIND_ATTRIBUTE, skip.kind.as_str().to_string())],
                start,
                record_time(&skip.record),
            ));
        }
        GraphBatch {
            schema_version: GRAPH_SCHEMA_VERSION,
            contract_revision: GRAPH_CONTRACT_REVISION.into(),
            entities,
            relations,
            observations,
            events,
            derivations: Vec::new(),
            availability: Vec::new(),
            dashboards: Vec::new(),
        }
    }

    fn counter(
        &self,
        entity: &EntityKey,
        name: &str,
        value: u64,
        attributes: &[(&str, String)],
        start: Option<u64>,
        time: u64,
    ) -> Observation {
        let description = match name {
            FAULTS_METRIC => "Kernel faults since boot",
            CPU_FAULTS_METRIC => "Kernel faults since boot on one logical CPU",
            _ => "Kernel fault lines since boot that could not be read",
        };
        let unit = if name == SKIPPED_METRIC {
            "{line}"
        } else {
            "{fault}"
        };
        Observation {
            entity: entity.clone(),
            scope: Self::scope(),
            descriptor: MetricDescriptor {
                name: name.into(),
                description: description.into(),
                unit: MetricUnit {
                    ucum: unit.into(),
                    display: UnitDisplay::Number,
                },
                value_domain: ValueDomain::Unbounded,
                instrument: MetricInstrument::Sum {
                    monotonic: true,
                    temporality: AggregationTemporality::Cumulative,
                },
                spatial_aggregation: SpatialAggregation::Sum,
            },
            ratio_denominator: None,
            attributes: attributes
                .iter()
                .map(|(key, value)| ((*key).to_string(), string(value)))
                .collect(),
            start_time_unix_nano: start.filter(|start| *start > 0 && *start <= time),
            time_unix_nano: time,
            value: MetricObservation::Measured {
                value: TypedMetricValue::Sum(Number::I64(i64::try_from(value).unwrap_or(i64::MAX))),
            },
            provenance: self.provenance(time),
        }
    }

    fn event(&self, node: &EntityKey, new: &NewFault, time: u64) -> Event {
        let fields = &new.fault.fields;
        let kind = new.fault.kind;
        let mut attributes = BTreeMap::from([
            (
                "event.name".to_string(),
                string(&format!("{EVENT_NAME_PREFIX}{kind}")),
            ),
            ("ih.event.severity".into(), string(kind.severity())),
            (KIND_ATTRIBUTE.into(), string(kind.as_str())),
            ("k8s.node.name".into(), string(&self.config.node_name)),
            ("host.name".into(), string(&self.config.host_name)),
            ("ih.boot.id".into(), string(&new.record.boot_id)),
        ]);
        let mut put = |key: &str, value: Option<AttributeValue>| {
            if let Some(value) = value {
                attributes.insert(key.to_string(), value);
            }
        };
        put(
            "process.executable.name",
            fields.process.as_deref().map(string),
        );
        put(
            "process.pid",
            fields.pid.map(|pid| AttributeValue::I64(i64::from(pid))),
        );
        put(
            "ih.kernel.fault.object",
            fields.object.as_deref().map(string),
        );
        put(
            "ih.kernel.fault.ip",
            fields.ip.map(|ip| string(&format!("0x{ip:x}"))),
        );
        put(
            "ih.kernel.fault.address",
            fields
                .address
                .map(|address| string(&format!("0x{address:x}"))),
        );
        put(
            "ih.kernel.fault.error_code",
            fields.error_code.as_deref().map(string),
        );
        put(
            CPU_ATTRIBUTE,
            fields.cpu.map(|cpu| AttributeValue::I64(i64::from(cpu))),
        );
        put(
            "ih.cpu.core",
            fields.core.map(|core| AttributeValue::I64(i64::from(core))),
        );
        put(
            "ih.cpu.socket",
            fields
                .socket
                .map(|socket| AttributeValue::I64(i64::from(socket))),
        );
        put("ih.kernel.trap", fields.trap.as_deref().map(string));
        put("system.device", fields.device.as_deref().map(string));
        put(
            "system.filesystem.type",
            fields.filesystem.as_deref().map(string),
        );
        put(
            "ih.kernel.oom.scope",
            fields.oom_scope.as_deref().map(string),
        );
        Event {
            entity: node.clone(),
            kind: EventKind::Domain,
            event_id: event_id(&self.config.node_name, &new.record),
            time: TimeRange {
                from_unix_nano: time,
                to_unix_nano: time + 1,
            },
            scope: Self::scope(),
            attributes,
            body: Some(string(&new.record.message)),
            provenance: self.provenance(time),
        }
    }
}

fn string(value: &str) -> AttributeValue {
    AttributeValue::String(value.to_string())
}

/// The line's own time in ns (never the read time); at least 1.
fn record_time(record: &KernelRecord) -> u64 {
    record.realtime_usec.saturating_mul(1_000).max(1)
}

/// `kernel:<node>:<boot>:<seqnum_id>:<seqnum>`: the journal's identity of
/// the line, so a resent line keeps its id.
pub fn event_id(node: &str, record: &KernelRecord) -> String {
    format!(
        "kernel:{node}:{}:{}:{}",
        record.boot_id, record.seqnum_id, record.seqnum
    )
}

fn parse_event_id(id: &str) -> Option<(String, String, u64)> {
    let mut parts = id.rsplitn(4, ':');
    let seqnum = parts.next()?.parse().ok()?;
    let seqnum_id = parts.next()?.to_string();
    let boot_id = parts.next()?.to_string();
    parts.next()?.strip_prefix("kernel:")?;
    Some((boot_id, seqnum_id, seqnum))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOOT: &str = "756747b89689450dab7d90556ced35da";
    const SEQ: &str = "73f2e7bccbce438bb830953e0ca97109";

    fn config() -> NodeConfig {
        NodeConfig {
            organization: "ih-local".into(),
            cluster_uid: "5f51b1b5".into(),
            cluster_name: Some("k-lab".into()),
            node_name: "amedes".into(),
            node_uid: "0d9912ab".into(),
            host_name: "amedes".into(),
        }
    }

    fn record(seqnum: u64, realtime_usec: u64, message: &str) -> KernelRecord {
        KernelRecord {
            realtime_usec,
            monotonic_usec: 1_000_000,
            boot_id: BOOT.into(),
            seqnum_id: SEQ.into(),
            seqnum,
            message: message.into(),
        }
    }

    const SEGV_20: &str = "db::materialize[3588289]: segfault at 1000160 ip 000055704a098657 sp 00007f9ab5bf8160 error 4 in poet-bc7834abebfed280[7e96657,557043e76000+63ea000] likely on CPU 20 (core 4, socket 0)";
    const SEGV_1: &str = "rustc[2772395]: segfault at 8 ip 00007f1a79ccf8ee sp 00007f1a79bb06b0 error 4 in libgcc_s.so.1[158ee,7f1a79cbd000+17000] likely on CPU 1 (core 1, socket 0)";
    const GPF: &str = "traps: opt cgu.00[848121] general protection fault ip:7f47836c84cb sp:7f463f1f7d70 error:0 in libLLVM.so.20.1-rust-1.90.0-stable[7ec84cb,7f4783400000+1f8c000]";

    fn boot_records() -> Vec<KernelRecord> {
        vec![
            record(10, 2_000_000, "Linux version 6.12.30"),
            record(11, 3_000_000, SEGV_20),
            record(12, 4_000_000, GPF),
            record(13, 5_000_000, SEGV_20),
            record(14, 6_000_000, "x: segfault at 8"),
            record(15, 7_000_000, SEGV_1),
        ]
    }

    fn answer() -> GraphIntakeAnswer {
        GraphIntakeAnswer {
            definitions_epoch: "e1".into(),
        }
    }

    #[test]
    fn faults_become_events_at_their_own_time_with_counters_per_kind_and_cpu() {
        let mut muse = KernelMuse::new(config(), Cursor::default()).unwrap();
        let (faults, skips) = muse.ingest(boot_records());
        assert_eq!(faults.len(), 4);
        assert_eq!(skips.len(), 1);
        let request = muse.intake(&faults, &skips, 99_000_000_000).unwrap();
        request.batch.validate().unwrap();
        let batch = &request.batch;
        assert_eq!(batch.events.len(), 4);
        let first = &batch.events[0];
        assert_eq!(first.entity, config().node_key());
        assert_eq!(
            first.time.from_unix_nano, 3_000_000_000,
            "the line's time, not now"
        );
        assert_eq!(first.event_id, format!("kernel:amedes:{BOOT}:{SEQ}:11"));
        assert_eq!(first.attributes["event.name"], string("kernel.segfault"));
        assert_eq!(
            first.attributes["process.executable.name"],
            string("db::materialize")
        );
        assert_eq!(
            first.attributes["process.pid"],
            AttributeValue::I64(3_588_289)
        );
        assert_eq!(first.attributes[CPU_ATTRIBUTE], AttributeValue::I64(20));
        assert_eq!(first.attributes["ih.cpu.core"], AttributeValue::I64(4));
        assert_eq!(
            first.attributes["ih.kernel.fault.object"],
            string("poet-bc7834abebfed280")
        );
        assert_eq!(
            first.attributes["ih.kernel.fault.ip"],
            string("0x55704a098657")
        );
        assert_eq!(first.body, Some(string(SEGV_20)));
        // Counters: segfaults 1, 2, 3; on CPU 20 twice, CPU 1 once.
        let points: Vec<(String, String, i64)> = batch
            .observations
            .iter()
            .map(|o| {
                let value = match &o.value {
                    MetricObservation::Measured {
                        value: TypedMetricValue::Sum(Number::I64(v)),
                    } => *v,
                    other => panic!("{other:?}"),
                };
                let cpu = o
                    .attributes
                    .get(CPU_ATTRIBUTE)
                    .map(|v| format!("{v:?}"))
                    .unwrap_or_default();
                (o.descriptor.name.clone(), cpu, value)
            })
            .collect();
        let segv_totals: Vec<i64> = batch
            .observations
            .iter()
            .filter(|o| {
                o.descriptor.name == FAULTS_METRIC
                    && o.attributes[KIND_ATTRIBUTE] == string("segfault")
            })
            .map(|o| match &o.value {
                MetricObservation::Measured {
                    value: TypedMetricValue::Sum(Number::I64(v)),
                } => *v,
                _ => 0,
            })
            .collect();
        assert_eq!(segv_totals, [1, 2, 3], "{points:?}");
        let cpu20: Vec<i64> = points
            .iter()
            .filter(|(name, cpu, _)| name == CPU_FAULTS_METRIC && cpu.contains("20"))
            .map(|(_, _, v)| *v)
            .collect();
        assert_eq!(cpu20, [1, 2]);
        assert_eq!(muse.cpu_totals()[&(FaultKind::Segfault, 20)], 2);
        assert_eq!(muse.totals()[&FaultKind::GeneralProtection], 1);
        let skipped: Vec<&Observation> = batch
            .observations
            .iter()
            .filter(|o| o.descriptor.name == SKIPPED_METRIC)
            .collect();
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].time_unix_nano, 6_000_000_000);
        // Counters start at boot: realtime - monotonic of the lines.
        assert_eq!(
            batch.observations[0].start_time_unix_nano,
            Some(1_000_000_000)
        );
        assert_eq!(batch.dashboards.len(), 1, "the definition rides along");
    }

    #[test]
    fn with_no_faults_nothing_is_sent_after_the_introduction() {
        let mut muse = KernelMuse::new(config(), Cursor::default()).unwrap();
        let (faults, skips) = muse.ingest(vec![record(1, 1_000_000, "usb 1-1: new device")]);
        assert!(faults.is_empty() && skips.is_empty());
        let hello = muse
            .intake(&faults, &skips, 5)
            .expect("entities and definition, once");
        assert!(hello.batch.events.is_empty());
        assert!(hello.batch.observations.is_empty(), "no synthetic zeros");
        assert_eq!(hello.batch.dashboards.len(), 1);
        muse.acknowledge(&hello, &answer());
        assert!(muse.intake(&[], &[], 6).is_none());
    }

    #[test]
    fn a_restart_sends_nothing_twice_and_keeps_the_boot_totals() {
        let mut muse = KernelMuse::new(config(), Cursor::default()).unwrap();
        let records = boot_records();
        let (faults, skips) = muse.ingest(records[..3].to_vec());
        let request = muse.intake(&faults, &skips, 5).unwrap();
        muse.acknowledge(&request, &answer());
        let cursor = muse.acknowledged_cursor().clone();
        assert_eq!(cursor.sent[SEQ], 12);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cursor.json");
        cursor.save(&path).unwrap();

        // A new process reads the whole boot again.
        let mut restarted = KernelMuse::new(config(), Cursor::load(&path)).unwrap();
        let (faults, skips) = restarted.ingest(boot_records());
        let seqnums: Vec<u64> = faults.iter().map(|f| f.record.seqnum).collect();
        assert_eq!(seqnums, [13, 15], "only the lines no Poet acknowledged");
        assert_eq!(skips.len(), 1);
        // Totals count the whole boot: the segfault at 13 is the second on CPU 20.
        assert_eq!(faults[0].kind_total, 2);
        assert_eq!(faults[0].cpu_total, Some((20, 2)));
    }

    #[test]
    fn unacknowledged_requests_leave_the_saved_cursor_behind() {
        let mut muse = KernelMuse::new(config(), Cursor::default()).unwrap();
        let (faults, skips) = muse.ingest(boot_records());
        let _queued = muse.intake(&faults, &skips, 5).unwrap();
        assert_eq!(muse.acknowledged_cursor(), &Cursor::default());
        // A new boot resets the totals.
        let mut other = record(1, 9_000_000, SEGV_1);
        other.boot_id = "ffff".into();
        let (faults, _) = muse.ingest(vec![other]);
        assert_eq!(faults[0].kind_total, 1);
    }

    #[test]
    fn event_ids_round_trip_and_a_missing_cursor_file_is_empty() {
        let line = record(42, 1, "x");
        assert_eq!(
            parse_event_id(&event_id("pi00", &line)),
            Some((BOOT.into(), SEQ.into(), 42))
        );
        assert_eq!(parse_event_id("k8s:x:backlog:1-2"), None);
        assert_eq!(
            Cursor::load(Path::new("/nonexistent/cursor.json")),
            Cursor::default()
        );
    }
}
