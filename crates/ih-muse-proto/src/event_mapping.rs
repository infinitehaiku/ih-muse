//! Event mappings: how one system's own OpenTelemetry names become the
//! shared vocabulary's ([`crate::deployment`]), as data.
//!
//! A system that sends OTLP itself (Piceli 0.16 and later) names its events
//! and attributes after itself (`piceli.deploy.rolled`, `piceli.trigger`).
//! Poet maps the OpenTelemetry CI/CD conventions on its own (a root span
//! with `cicd.pipeline.run.id` is one release; see Poet's
//! `docs/components/events.md`), and an [`EventMapping`] says the rest:
//! which resources it applies to, the system's name and installation, and
//! for each of the system's event names the vocabulary event it is, its
//! severity, and which attributes carry which vocabulary attribute. Poet
//! holds no system's names in code.
//!
//! The integration pack of a system (its Muse in definitions-only mode, or
//! any sender) ships the mapping in a graph batch as one entity
//! ([`EventMapping::to_entity`]): identity `Other { namespace:
//! "ih.definition", id: "event_mapping/<id>" }` with the JSON in
//! [`DEFINITION_JSON`]. The entity has no observations, so it is never a
//! dashboard source; Poet keeps the highest revision of each id per tenant
//! and applies it to OTLP intake from then on (data already stored keeps
//! the mapping it was stored with).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::dashboard::AttributeRule;
use crate::graph::{Entity, EntityIdentity, EntityKey, OrganizationId};
use crate::telemetry::{AttributeValue, TimeRange};

/// Entity namespace (identity `Other`) of definitions sent as entities.
pub const DEFINITION_NAMESPACE: &str = "ih.definition";
/// Entity id prefix of an event mapping (`event_mapping/<id>`).
pub const EVENT_MAPPING_PREFIX: &str = "event_mapping/";
/// Entity attribute holding the definition's JSON.
pub const DEFINITION_JSON: &str = "ih.definition.json";
/// Entity attribute holding the definition's kind (`event_mapping`).
pub const DEFINITION_KIND: &str = "ih.definition.kind";
/// Entity attribute holding the definition's revision.
pub const DEFINITION_REVISION: &str = "ih.definition.revision";

const MAX_RULES: usize = 64;
const MAX_TEXT: usize = 255;
const MAX_JSON_BYTES: usize = 64 * 1024;

/// One system's OTLP names in the shared vocabulary (see the module docs).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EventMapping {
    /// `<system>.<name>` (`piceli.deployments`).
    pub id: String,
    /// Starts at 1; a changed mapping carries a higher revision.
    pub revision: u32,
    /// Resource attributes that must all hold (`service.name =
    /// piceli-controller`).
    pub resource: Vec<AttributeRule>,
    /// `deployment.system.name` (`piceli`); the first part of every id.
    pub system: String,
    /// Resource attribute naming the installation
    /// (`deployment.system.instance`, `k8s.cluster.name`).
    pub instance_attribute: String,
    /// Event attribute holding the system's own stable event id
    /// (`piceli.event.id`); a marker's id is `<system>:<that id>`, so a
    /// resent event is stored once. An event without it is not mapped.
    pub event_id_attribute: String,
    /// Root-span attributes that must all hold for the span to be a
    /// release (`piceli.run.kind = deploy`); stop and start runs are not
    /// releases.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub release_when: Vec<AttributeRule>,
    /// Root-span attribute holding the run's result (`piceli.deploy.result`).
    pub result_attribute: String,
    /// The result values: the vocabulary state and severity of each.
    pub results: Vec<ResultRule>,
    /// Attributes copied onto the release and its markers under the
    /// vocabulary's name (`piceli.trigger` -> `deployment.trigger`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attributes: Vec<AttributeRename>,
    /// The sources of a release: two parallel list attributes, names and
    /// commits (`deployment.sources` = `name=commit,...`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sources: Option<SourcesRule>,
    /// Stage spans that are markers (`apply` started, `checks` passed).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stages: Vec<StageRule>,
    /// The system's events (OTLP log records with an event name).
    pub events: Vec<EventRule>,
}

/// A run result value and what it is in the vocabulary.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResultRule {
    /// The system's value (`rolled-back`).
    pub value: String,
    /// `deployment.state` (`failed`, `deployed`, `interrupted`).
    pub state: String,
    /// `info`, `warning` or `error`.
    pub severity: String,
}

/// One attribute under another name. A list value is joined with `join`
/// (`,` by default; `\n` for `name: detail` lines).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AttributeRename {
    pub from: String,
    pub to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join: Option<String>,
}

/// Parallel list attributes of a release's sources.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourcesRule {
    pub names: String,
    pub revisions: String,
}

/// A stage span (`cicd.pipeline.task.name`) that is a marker.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StageRule {
    /// `cicd.pipeline.task.name` (`apply`).
    pub task: String,
    /// Only when `cicd.pipeline.task.run.result` has this value (`success`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    /// `start` or `end` of the span.
    pub at: String,
    /// The vocabulary event (`deployment.apply.started`).
    pub event: String,
    /// Last part of the marker id (`apply-started`).
    pub what: String,
    pub severity: String,
}

/// One of the system's events and the vocabulary event it is.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EventRule {
    /// The system's event name (`piceli.deploy.rolled`).
    pub name: String,
    /// The vocabulary event (`deployment.rolled`).
    pub event: String,
    /// `info`, `warning` or `error`; unset: `error` for an OTLP severity
    /// of ERROR or worse, else `info`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    /// What it is about: `release` (default), `environment` or `system`.
    #[serde(default = "release_scope")]
    pub on: String,
    /// Attributes under the vocabulary's names, besides the mapping's own.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attributes: Vec<AttributeRename>,
}

fn release_scope() -> String {
    "release".into()
}

/// Why an [`EventMapping`] is rejected.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum EventMappingError {
    #[error("id must be `<system>.<name>` of a-z, 0-9, '_', '-', '.' (at most 128 bytes)")]
    Id,
    #[error("revision must be greater than 0")]
    Revision,
    #[error("{0} must be 1..=255 bytes")]
    Text(&'static str),
    #[error("at most {MAX_RULES} entries per list, and at least one resource rule and one event")]
    Count,
    #[error("severity must be info, warning or error")]
    Severity,
    #[error("a target must be a `deployment.` name: {0}")]
    Target(String),
    #[error("an event rule's `on` must be release, environment or system")]
    Scope,
    #[error("a stage rule's `at` must be start or end")]
    StageAt,
    #[error("event {0} is mapped twice")]
    Duplicate(String),
    #[error("the definition JSON is invalid or larger than 64 KiB: {0}")]
    Json(String),
}

fn text(value: &str, what: &'static str) -> Result<(), EventMappingError> {
    if value.trim().is_empty() || value.len() > MAX_TEXT {
        return Err(EventMappingError::Text(what));
    }
    Ok(())
}

fn severity(value: &str) -> Result<(), EventMappingError> {
    if matches!(value, "info" | "warning" | "error") {
        Ok(())
    } else {
        Err(EventMappingError::Severity)
    }
}

fn target(value: &str) -> Result<(), EventMappingError> {
    text(value, "target")?;
    if value.starts_with("deployment.") {
        Ok(())
    } else {
        Err(EventMappingError::Target(value.into()))
    }
}

fn renames(list: &[AttributeRename]) -> Result<(), EventMappingError> {
    if list.len() > MAX_RULES {
        return Err(EventMappingError::Count);
    }
    for rename in list {
        text(&rename.from, "attribute from")?;
        text(&rename.to, "attribute to")?;
        if rename.join.as_ref().is_some_and(|join| join.len() > 8) {
            return Err(EventMappingError::Text("join"));
        }
    }
    Ok(())
}

impl EventMapping {
    /// Checks every rule a sender and Poet enforce.
    pub fn validate(&self) -> Result<(), EventMappingError> {
        use EventMappingError as E;
        let id_byte = |byte: u8| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_-.".contains(&byte)
        };
        if self.id.len() > 128
            || !self.id.contains('.')
            || self.id.starts_with('.')
            || self.id.ends_with('.')
            || !self.id.bytes().all(id_byte)
        {
            return Err(E::Id);
        }
        if self.revision == 0 {
            return Err(E::Revision);
        }
        text(&self.system, "system")?;
        text(&self.instance_attribute, "instance_attribute")?;
        text(&self.event_id_attribute, "event_id_attribute")?;
        text(&self.result_attribute, "result_attribute")?;
        if self.resource.is_empty()
            || self.resource.len() > MAX_RULES
            || self.release_when.len() > MAX_RULES
            || self.results.len() > MAX_RULES
            || self.stages.len() > MAX_RULES
            || self.events.is_empty()
            || self.events.len() > MAX_RULES
        {
            return Err(E::Count);
        }
        for rule in self.resource.iter().chain(&self.release_when) {
            text(&rule.key, "attribute rule key")?;
            if rule.value.as_ref().is_some_and(|value| value.len() > MAX_TEXT) {
                return Err(E::Text("attribute rule value"));
            }
        }
        for result in &self.results {
            text(&result.value, "result value")?;
            text(&result.state, "result state")?;
            severity(&result.severity)?;
        }
        renames(&self.attributes)?;
        if let Some(sources) = &self.sources {
            text(&sources.names, "sources names")?;
            text(&sources.revisions, "sources revisions")?;
        }
        for stage in &self.stages {
            text(&stage.task, "stage task")?;
            text(&stage.what, "stage what")?;
            target(&stage.event)?;
            severity(&stage.severity)?;
            if !matches!(stage.at.as_str(), "start" | "end") {
                return Err(E::StageAt);
            }
        }
        let mut names = BTreeSet::new();
        for rule in &self.events {
            text(&rule.name, "event name")?;
            target(&rule.event)?;
            if let Some(value) = &rule.severity {
                severity(value)?;
            }
            if !matches!(rule.on.as_str(), "release" | "environment" | "system") {
                return Err(E::Scope);
            }
            renames(&rule.attributes)?;
            if !names.insert(rule.name.as_str()) {
                return Err(E::Duplicate(rule.name.clone()));
            }
        }
        Ok(())
    }

    /// The rule of the system's event `name`, if mapped.
    pub fn event(&self, name: &str) -> Option<&EventRule> {
        self.events.iter().find(|rule| rule.name == name)
    }

    /// The vocabulary state and severity of a result value.
    pub fn result(&self, value: &str) -> Option<&ResultRule> {
        self.results.iter().find(|rule| rule.value == value)
    }

    /// Whether every rule holds for `attributes` (text values).
    pub fn matches(rules: &[AttributeRule], attributes: &BTreeMap<String, String>) -> bool {
        rules.iter().all(|rule| match (attributes.get(&rule.key), &rule.value) {
            (Some(found), Some(value)) => found == value,
            (Some(_), None) => true,
            (None, _) => false,
        })
    }

    /// The mapping as the entity a graph batch carries (see the module
    /// docs), alive over `lifetime`.
    pub fn to_entity(
        &self,
        organization: &str,
        lifetime: TimeRange,
    ) -> Result<Entity, EventMappingError> {
        self.validate()?;
        let json =
            serde_json::to_string(self).map_err(|error| EventMappingError::Json(error.to_string()))?;
        if json.len() > MAX_JSON_BYTES {
            return Err(EventMappingError::Json("larger than 64 KiB".into()));
        }
        Ok(Entity {
            key: EntityKey {
                organization: OrganizationId(organization.into()),
                identity: EntityIdentity::Other {
                    namespace: DEFINITION_NAMESPACE.into(),
                    id: format!("{EVENT_MAPPING_PREFIX}{}", self.id),
                },
            },
            lifetime,
            attributes: BTreeMap::from([
                (DEFINITION_KIND.into(), AttributeValue::String("event_mapping".into())),
                (
                    DEFINITION_REVISION.into(),
                    AttributeValue::String(self.revision.to_string()),
                ),
                (DEFINITION_JSON.into(), AttributeValue::String(json)),
            ]),
        })
    }

    /// The mapping an entity carries: `None` when the entity is not an event
    /// mapping, `Some(Err)` when it is one that does not parse or validate.
    pub fn from_entity(entity: &Entity) -> Option<Result<Self, EventMappingError>> {
        let EntityIdentity::Other { namespace, id } = &entity.key.identity else {
            return None;
        };
        if namespace != DEFINITION_NAMESPACE || !id.starts_with(EVENT_MAPPING_PREFIX) {
            return None;
        }
        let Some(AttributeValue::String(json)) = entity.attributes.get(DEFINITION_JSON) else {
            return Some(Err(EventMappingError::Json("no definition JSON".into())));
        };
        if json.len() > MAX_JSON_BYTES {
            return Some(Err(EventMappingError::Json("larger than 64 KiB".into())));
        }
        Some(
            serde_json::from_str::<Self>(json)
                .map_err(|error| EventMappingError::Json(error.to_string()))
                .and_then(|mapping| {
                    mapping.validate()?;
                    if id[EVENT_MAPPING_PREFIX.len()..] != mapping.id {
                        return Err(EventMappingError::Id);
                    }
                    Ok(mapping)
                }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn example() -> EventMapping {
        EventMapping {
            id: "demo.deployments".into(),
            revision: 1,
            resource: vec![AttributeRule {
                key: "service.name".into(),
                value: Some("demo-controller".into()),
            }],
            system: "demo".into(),
            instance_attribute: "k8s.cluster.name".into(),
            event_id_attribute: "demo.event.id".into(),
            release_when: vec![AttributeRule {
                key: "demo.run.kind".into(),
                value: Some("deploy".into()),
            }],
            result_attribute: "demo.result".into(),
            results: vec![ResultRule {
                value: "success".into(),
                state: "deployed".into(),
                severity: "info".into(),
            }],
            attributes: vec![AttributeRename {
                from: "demo.trigger".into(),
                to: "deployment.trigger".into(),
                join: None,
            }],
            sources: None,
            stages: vec![],
            events: vec![EventRule {
                name: "demo.rolled".into(),
                event: "deployment.rolled".into(),
                severity: None,
                on: "release".into(),
                attributes: vec![],
            }],
        }
    }

    #[test]
    fn a_mapping_travels_as_an_entity_and_back() {
        let mapping = example();
        let entity = mapping
            .to_entity(
                "org",
                TimeRange {
                    from_unix_nano: 1,
                    to_unix_nano: 2,
                },
            )
            .unwrap();
        assert_eq!(EventMapping::from_entity(&entity), Some(Ok(mapping)));
    }

    #[test]
    fn invalid_mappings_are_refused() {
        let mut mapping = example();
        mapping.events[0].event = "k8s.rollout.start".into();
        assert!(matches!(mapping.validate(), Err(EventMappingError::Target(_))));
        let mut mapping = example();
        mapping.events.push(mapping.events[0].clone());
        assert!(matches!(mapping.validate(), Err(EventMappingError::Duplicate(_))));
        let mut mapping = example();
        mapping.results[0].severity = "fatal".into();
        assert_eq!(mapping.validate(), Err(EventMappingError::Severity));
        let mut mapping = example();
        mapping.id = "nodot".into();
        assert_eq!(mapping.validate(), Err(EventMappingError::Id));
    }

    #[test]
    fn rules_match_present_and_equal_attributes() {
        let attributes = BTreeMap::from([("service.name".to_string(), "demo-controller".to_string())]);
        assert!(EventMapping::matches(&example().resource, &attributes));
        assert!(!EventMapping::matches(&example().release_when, &attributes));
    }
}
