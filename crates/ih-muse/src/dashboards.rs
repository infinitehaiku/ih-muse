//! Default dashboards a Muse defines and the rule for delivering them.
//!
//! A Muse attaches its definitions to every [`GraphBatch`] until a Poet
//! acknowledges a batch that carried them. After that, batches leave the
//! field out, so their bytes stay as they were before definitions existed.
//! While no Poet is reachable every queued batch carries them, so a queue
//! that drops its oldest batches cannot lose the definitions. Receivers
//! deduplicate by `(id, revision)`.

use std::collections::BTreeSet;

use ih_muse_proto::dashboard::MAX_BATCH_DASHBOARDS;
use ih_muse_proto::{DashboardDefinition, DashboardDefinitionError, GraphBatch};

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
    /// More definitions than one batch may carry.
    #[error("at most {MAX_BATCH_DASHBOARDS} dashboard definitions per Muse")]
    TooMany,
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

/// Checks each definition and the set rules a batch enforces.
pub fn check_definitions(definitions: &[DashboardDefinition]) -> Result<(), DashboardSetError> {
    if definitions.len() > MAX_BATCH_DASHBOARDS {
        return Err(DashboardSetError::TooMany);
    }
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

/// A Muse's validated definitions and whether a Poet has acknowledged a
/// batch that carried them.
#[derive(Clone, Debug, Default)]
pub struct DashboardDelivery {
    definitions: Vec<DashboardDefinition>,
    delivered: bool,
}

impl DashboardDelivery {
    /// Validates the definitions. An empty set is valid and never attaches.
    pub fn new(definitions: Vec<DashboardDefinition>) -> Result<Self, DashboardSetError> {
        check_definitions(&definitions)?;
        Ok(Self {
            definitions,
            delivered: false,
        })
    }

    pub fn definitions(&self) -> &[DashboardDefinition] {
        &self.definitions
    }

    /// True once a Poet acknowledged a batch that carried the definitions.
    pub fn is_delivered(&self) -> bool {
        self.delivered || self.definitions.is_empty()
    }

    /// Puts the definitions on `batch` unless they were already delivered.
    pub fn attach(&self, batch: &mut GraphBatch) {
        if !self.is_delivered() {
            batch.dashboards = self.definitions.clone();
        }
    }

    /// Call after a Poet acknowledged `batch`; it marks the definitions
    /// delivered only if that batch carried them.
    pub fn acknowledge(&mut self, batch: &GraphBatch) {
        self.acknowledge_carried(&batch.dashboards);
    }

    /// [`Self::acknowledge`] for a Muse that builds batches itself (for
    /// example as JSON through the Python binding): `carried` is the
    /// acknowledged batch's `dashboards` field.
    pub fn acknowledge_carried(&mut self, carried: &[DashboardDefinition]) {
        if !self.definitions.is_empty() && carried == self.definitions.as_slice() {
            self.delivered = true;
        }
    }

    /// Sends the definitions again from the next batch, for example after
    /// the Muse replaced them or a Poet cluster lost its stored copy.
    pub fn resend(&mut self) {
        self.delivered = false;
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

        let one = parse_dashboard_definitions(K8S).unwrap().remove(0);
        let many = (0..=MAX_BATCH_DASHBOARDS)
            .map(|n| DashboardDefinition {
                id: format!("k8s.cluster{n}"),
                ..one.clone()
            })
            .collect();
        assert_eq!(
            DashboardDelivery::new(many).unwrap_err(),
            DashboardSetError::TooMany
        );
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
        delivery.acknowledge(&batch());
        assert!(!delivery.is_delivered());

        delivery.acknowledge(&second);
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

    #[test]
    fn an_empty_set_never_attaches() {
        let delivery = DashboardDelivery::new(Vec::new()).unwrap();
        assert!(delivery.is_delivered());
        let mut b = batch();
        delivery.attach(&mut b);
        assert!(b.dashboards.is_empty());
    }
}
