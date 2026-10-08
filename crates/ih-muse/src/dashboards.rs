//! Default dashboards a Muse defines and the rule for delivering them.
//!
//! A Muse attaches its definitions to every [`GraphBatch`] until a Poet
//! acknowledges a batch that carried them. After that, batches leave the
//! field out, so their bytes stay as they were before definitions existed.
//! While no Poet is reachable every queued batch carries them, so a queue
//! that drops its oldest batches cannot lose the definitions. Receivers
//! deduplicate by `(id, revision)`.
//!
//! A Poet that lost them (restarted on a wiped store) gets them back
//! without a Muse restart: each acknowledgement names the Poet's
//! [`GraphIntakeAnswer::definitions_epoch`], and an answer with another
//! epoch than the one that acknowledged the definitions sends them again,
//! once. Nothing is resent periodically.

use std::collections::BTreeSet;

use ih_muse_proto::dashboard::MAX_BATCH_DASHBOARDS;
use ih_muse_proto::{DashboardDefinition, DashboardDefinitionError, GraphBatch, GraphIntakeAnswer};

/// Why a set of dashboard definitions cannot be delivered.
#[derive(Debug, PartialEq, thiserror::Error)]
pub enum DashboardSetError {
    /// The text is not JSON of the definition shape.
    #[error("dashboard definitions are not valid JSON of the definition shape: {0}")]
    Parse(String),
    /// One definition broke a rule; `id` is empty when the id itself is missing.
    #[error("dashboard {id:?}: {source}")]
    Invalid {
        id: String,
        source: DashboardDefinitionError,
    },
    /// Two definitions share an id.
    #[error("duplicate dashboard id {0}")]
    Duplicate(String),
}

/// Parses one definition object or an array of them from JSON text, then
/// checks them as [`DashboardDelivery::new`] does.
pub fn parse_dashboard_definitions(
    json: &str,
) -> Result<Vec<DashboardDefinition>, DashboardSetError> {
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        Many(Vec<DashboardDefinition>),
        One(Box<DashboardDefinition>),
    }
    // Parse twice on failure only, so the error names the real problem
    // instead of serde's generic "did not match any variant".
    let definitions = match serde_json::from_str::<OneOrMany>(json) {
        Ok(OneOrMany::Many(many)) => many,
        Ok(OneOrMany::One(one)) => vec![*one],
        Err(_) => {
            let value: serde_json::Value =
                serde_json::from_str(json).map_err(|e| DashboardSetError::Parse(e.to_string()))?;
            let error = if value.is_array() {
                serde_json::from_value::<Vec<DashboardDefinition>>(value).err()
            } else {
                serde_json::from_value::<DashboardDefinition>(value).err()
            };
            return Err(DashboardSetError::Parse(
                error.map_or_else(|| "unrecognised shape".into(), |e| e.to_string()),
            ));
        }
    };
    check_definitions(&definitions)?;
    Ok(definitions)
}

/// Checks each definition and that ids are unique. A set may hold any
/// number of definitions; [`DashboardDelivery`] splits it into batches.
pub fn check_definitions(definitions: &[DashboardDefinition]) -> Result<(), DashboardSetError> {
    let mut ids = BTreeSet::new();
    for definition in definitions {
        definition
            .validate()
            .map_err(|source| DashboardSetError::Invalid {
                id: definition.id.clone(),
                source,
            })?;
        if !ids.insert(definition.id.as_str()) {
            return Err(DashboardSetError::Duplicate(definition.id.clone()));
        }
    }
    Ok(())
}

/// A Muse's validated definitions, split into chunks of at most
/// [`MAX_BATCH_DASHBOARDS`] (one per batch), which chunks a Poet has
/// acknowledged, and under which Poet definitions epoch.
#[derive(Clone, Debug, Default)]
pub struct DashboardDelivery {
    definitions: Vec<DashboardDefinition>,
    /// One flag per chunk: a Poet acknowledged a batch that carried it.
    delivered: Vec<bool>,
    /// The epoch of the answers that acknowledged the delivered chunks;
    /// `None` while no chunk is delivered.
    epoch: Option<String>,
}

impl DashboardDelivery {
    /// Validates the definitions. An empty set is valid and never attaches.
    pub fn new(definitions: Vec<DashboardDefinition>) -> Result<Self, DashboardSetError> {
        check_definitions(&definitions)?;
        let chunks = definitions.len().div_ceil(MAX_BATCH_DASHBOARDS);
        Ok(Self {
            definitions,
            delivered: vec![false; chunks],
            epoch: None,
        })
    }

    pub fn definitions(&self) -> &[DashboardDefinition] {
        &self.definitions
    }

    /// The definitions as batches carry them, in order.
    pub fn chunks(&self) -> impl Iterator<Item = &[DashboardDefinition]> {
        self.definitions.chunks(MAX_BATCH_DASHBOARDS)
    }

    /// True once a Poet acknowledged a batch for every chunk.
    pub fn is_delivered(&self) -> bool {
        self.delivered.iter().all(|delivered| *delivered)
    }

    /// The chunks no Poet has acknowledged yet.
    pub fn pending_chunks(&self) -> usize {
        self.delivered
            .iter()
            .filter(|delivered| !**delivered)
            .count()
    }

    /// The chunk the next batch carries: the first one not acknowledged
    /// yet, or `None` once all are delivered.
    pub fn next_chunk(&self) -> Option<&[DashboardDefinition]> {
        self.chunks()
            .zip(&self.delivered)
            .find(|(_, delivered)| !**delivered)
            .map(|(chunk, _)| chunk)
    }

    /// Puts the next undelivered chunk on `batch`, if any.
    pub fn attach(&self, batch: &mut GraphBatch) {
        if let Some(chunk) = self.next_chunk() {
            batch.dashboards = chunk.to_vec();
        }
    }

    /// Call after a Poet acknowledged `batch` with `answer`. When the
    /// answer names another definitions epoch than the one that
    /// acknowledged the delivered chunks, that Poet may have lost them:
    /// every chunk is sent again from the next batch. Then the chunk the
    /// batch carried is marked delivered (a batch that carried none proves
    /// nothing).
    pub fn acknowledge(&mut self, batch: &GraphBatch, answer: &GraphIntakeAnswer) {
        self.acknowledge_carried(&batch.dashboards, &answer.definitions_epoch);
    }

    /// [`Self::acknowledge`] for a Muse that builds batches itself (for
    /// example as JSON through the Python binding): `carried` is the
    /// acknowledged batch's `dashboards` field, `definitions_epoch` the
    /// answer's (empty when the Poet did not say).
    pub fn acknowledge_carried(
        &mut self,
        carried: &[DashboardDefinition],
        definitions_epoch: &str,
    ) {
        if self
            .epoch
            .as_deref()
            .is_some_and(|epoch| epoch != definitions_epoch)
        {
            self.resend();
        }
        if carried.is_empty() {
            return;
        }
        let Self {
            definitions,
            delivered,
            epoch,
        } = self;
        for (chunk, delivered) in definitions.chunks(MAX_BATCH_DASHBOARDS).zip(delivered) {
            if chunk == carried {
                *delivered = true;
                *epoch = Some(definitions_epoch.to_owned());
            }
        }
    }

    /// Sends every chunk again from the next batch, for example after the
    /// Muse replaced its definitions or a Poet cluster lost its stored copy.
    pub fn resend(&mut self) {
        self.delivered
            .iter_mut()
            .for_each(|delivered| *delivered = false);
        self.epoch = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MACOS: &str = include_str!("../../../examples/dashboards/macos-host.json");
    const K8S: &str = include_str!("../../../examples/dashboards/k8s-cluster.json");

    fn batch() -> GraphBatch {
        GraphBatch {
            schema_version: ih_muse_proto::GRAPH_SCHEMA_VERSION,
            contract_revision: ih_muse_proto::GRAPH_CONTRACT_REVISION.into(),
            entities: Vec::new(),
            relations: Vec::new(),
            observations: Vec::new(),
            events: Vec::new(),
            derivations: Vec::new(),
            availability: Vec::new(),
            dashboards: Vec::new(),
        }
    }

    #[test]
    fn shipped_examples_parse_alone_and_as_a_list() {
        let k8s = parse_dashboard_definitions(K8S).unwrap();
        assert_eq!(k8s.len(), 1);
        assert_eq!(k8s[0].id, "k8s.cluster");
        assert_eq!(k8s[0].muse_kind, "k8s");
        let both = parse_dashboard_definitions(&format!("[{MACOS},{K8S}]")).unwrap();
        assert_eq!(
            both.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(),
            ["macos.host", "k8s.cluster"]
        );
    }

    #[test]
    fn invalid_definitions_are_rejected_with_the_reason() {
        let err = parse_dashboard_definitions("{not json").unwrap_err();
        assert!(matches!(err, DashboardSetError::Parse(_)), "{err}");

        let unknown = K8S.replacen("\"revision\"", "\"colour\": 1, \"revision\"", 1);
        let err = parse_dashboard_definitions(&unknown).unwrap_err();
        assert!(err.to_string().contains("colour"), "{err}");

        let columns = K8S.replace("\"columns\": 3", "\"columns\": 9");
        assert_eq!(
            parse_dashboard_definitions(&columns).unwrap_err(),
            DashboardSetError::Invalid {
                id: "k8s.cluster".into(),
                source: DashboardDefinitionError::Columns
            }
        );

        let wrong_kind = K8S.replace("\"id\": \"k8s.cluster\"", "\"id\": \"macos.cluster\"");
        assert!(matches!(
            parse_dashboard_definitions(&wrong_kind).unwrap_err(),
            DashboardSetError::Invalid {
                source: DashboardDefinitionError::Id,
                ..
            }
        ));

        assert_eq!(
            parse_dashboard_definitions(&format!("[{K8S},{K8S}]")).unwrap_err(),
            DashboardSetError::Duplicate("k8s.cluster".into())
        );
    }

    /// `n` distinct definitions built from the Kubernetes example.
    fn many(n: usize) -> Vec<DashboardDefinition> {
        let one = parse_dashboard_definitions(K8S).unwrap().remove(0);
        (0..n)
            .map(|index| DashboardDefinition {
                id: format!("k8s.cluster{index}"),
                ..one.clone()
            })
            .collect()
    }

    #[test]
    fn a_muse_with_more_than_one_batch_of_dashboards_sends_them_in_several_batches() {
        let definitions = many(2 * MAX_BATCH_DASHBOARDS + 5);
        let mut delivery = DashboardDelivery::new(definitions.clone()).unwrap();
        assert_eq!(delivery.pending_chunks(), 3);

        // Unacknowledged: every batch carries the first chunk.
        let mut first = batch();
        delivery.attach(&mut first);
        assert_eq!(first.dashboards, definitions[..MAX_BATCH_DASHBOARDS]);
        first.validate().expect("a chunk fits one batch");
        let mut again = batch();
        delivery.attach(&mut again);
        assert_eq!(again.dashboards, first.dashboards);

        // Acknowledged: the next batch carries the second chunk, and so on.
        delivery.acknowledge(&first, &GraphIntakeAnswer::default());
        let mut second = batch();
        delivery.attach(&mut second);
        assert_eq!(
            second.dashboards,
            definitions[MAX_BATCH_DASHBOARDS..2 * MAX_BATCH_DASHBOARDS]
        );
        delivery.acknowledge(&again, &GraphIntakeAnswer::default()); // the first chunk again: no change
        assert_eq!(delivery.pending_chunks(), 2);
        delivery.acknowledge(&second, &GraphIntakeAnswer::default());
        let mut third = batch();
        delivery.attach(&mut third);
        assert_eq!(third.dashboards, definitions[2 * MAX_BATCH_DASHBOARDS..]);
        third.validate().unwrap();
        assert!(!delivery.is_delivered());
        delivery.acknowledge(&third, &GraphIntakeAnswer::default());
        assert!(delivery.is_delivered());

        // Every definition rode exactly one acknowledged batch, in order.
        let sent = [first.dashboards, second.dashboards, third.dashboards].concat();
        assert_eq!(sent, definitions);

        let mut later = batch();
        delivery.attach(&mut later);
        assert!(later.dashboards.is_empty());

        delivery.resend();
        let mut resent = batch();
        delivery.attach(&mut resent);
        assert_eq!(resent.dashboards, definitions[..MAX_BATCH_DASHBOARDS]);
    }

    #[test]
    fn exactly_one_batch_of_dashboards_is_one_chunk() {
        let delivery = DashboardDelivery::new(many(MAX_BATCH_DASHBOARDS)).unwrap();
        assert_eq!(delivery.pending_chunks(), 1);
        assert_eq!(delivery.chunks().count(), 1);
    }

    #[test]
    fn definitions_ride_batches_until_a_poet_acknowledges_one_that_carried_them() {
        let definitions = parse_dashboard_definitions(K8S).unwrap();
        let mut delivery = DashboardDelivery::new(definitions.clone()).unwrap();

        let mut first = batch();
        delivery.attach(&mut first);
        assert_eq!(first.dashboards, definitions);
        first.validate().unwrap();

        // Unacknowledged: the next batch carries them too.
        let mut second = batch();
        delivery.attach(&mut second);
        assert_eq!(second.dashboards, definitions);

        // A batch that did not carry them proves nothing.
        delivery.acknowledge(&batch(), &GraphIntakeAnswer::default());
        assert!(!delivery.is_delivered());

        delivery.acknowledge(&second, &GraphIntakeAnswer::default());
        assert!(delivery.is_delivered());
        let mut later = batch();
        delivery.attach(&mut later);
        assert!(later.dashboards.is_empty());
        let json = serde_json::to_string(&later).unwrap();
        assert!(!json.contains("dashboards"), "{json}");

        delivery.resend();
        let mut again = batch();
        delivery.attach(&mut again);
        assert_eq!(again.dashboards, definitions);
    }

    /// A Poet that lost the definitions (wiped store, restart) answers
    /// with another epoch: they go out again once, in every chunk, and are
    /// delivered under the new epoch; the same epoch never resends, and
    /// an answer naming no epoch (an older Poet) changes nothing.
    #[test]
    fn a_new_definitions_epoch_sends_every_chunk_again_once() {
        let answer = |epoch: &str| GraphIntakeAnswer {
            definitions_epoch: epoch.into(),
        };
        let definitions = many(MAX_BATCH_DASHBOARDS + 1);
        let mut delivery = DashboardDelivery::new(definitions.clone()).unwrap();
        for _ in 0..2 {
            let mut carrying = batch();
            delivery.attach(&mut carrying);
            delivery.acknowledge(&carrying, &answer("a"));
        }
        assert!(delivery.is_delivered());
        delivery.acknowledge(&batch(), &answer("a"));
        assert!(delivery.is_delivered(), "the same epoch resends nothing");

        delivery.acknowledge(&batch(), &answer("b"));
        assert_eq!(
            delivery.pending_chunks(),
            2,
            "a new epoch resends every chunk"
        );
        let mut sent = Vec::new();
        while !delivery.is_delivered() {
            let mut carrying = batch();
            delivery.attach(&mut carrying);
            sent.extend(carrying.dashboards.clone());
            delivery.acknowledge(&carrying, &answer("b"));
        }
        assert_eq!(sent, definitions, "each definition once");
        delivery.acknowledge(&batch(), &answer("b"));
        assert!(delivery.is_delivered());

        // A chunk acknowledged under one epoch and another chunk under a
        // newer one: the first is sent again too.
        let mut delivery = DashboardDelivery::new(definitions.clone()).unwrap();
        let mut first = batch();
        delivery.attach(&mut first);
        delivery.acknowledge(&first, &answer("a"));
        let mut second = batch();
        delivery.attach(&mut second);
        delivery.acknowledge(&second, &answer("b"));
        assert_eq!(delivery.pending_chunks(), 1);
        let mut again = batch();
        delivery.attach(&mut again);
        assert_eq!(again.dashboards, first.dashboards);

        // An older Poet names no epoch: nothing is resent.
        let mut delivery = DashboardDelivery::new(definitions[..1].to_vec()).unwrap();
        let mut only = batch();
        delivery.attach(&mut only);
        delivery.acknowledge(&only, &GraphIntakeAnswer::default());
        delivery.acknowledge(&batch(), &GraphIntakeAnswer::default());
        assert!(delivery.is_delivered());
    }

    #[test]
    fn an_empty_set_never_attaches() {
        let delivery = DashboardDelivery::new(Vec::new()).unwrap();
        assert!(delivery.is_delivered());
        let mut b = batch();
        delivery.attach(&mut b);
        assert!(b.dashboards.is_empty());
    }
}
