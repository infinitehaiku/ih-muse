//! Dashboard definitions: the panels a Muse's author ships with the Muse.
//!
//! A definition is data only (panels, golden signals, thresholds, layout
//! blocks). It never carries queries, scripts, URLs or credentials. A Muse
//! sends its definitions inside a [`crate::GraphBatch`]; Poet stores every
//! revision and a renderer (Kabuki) draws them. Dashboard packs for sources
//! without a Muse (OpenTelemetry Collector receivers) use the same format with
//! [`DashboardAppliesTo::Recognition`].
//!
//! The panel types ([`PanelSpec`] and what it needs) moved here from the
//! Infinite Haiku `model` crate unchanged, so their JSON is byte-compatible.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// How the values of one panel are combined per time bucket.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelAggregation {
    /// Mean of the values in the bucket (gauges, up-down counters).
    Avg,
    Max,
    Min,
    /// Latest value in the bucket.
    Last,
    /// Sum of values across series (e.g. bytes used by every pool).
    Sum,
    /// Increase per second of a monotonic counter.
    Rate,
    /// Number of samples recorded by a histogram in the bucket.
    Count,
    /// Histogram samples per second.
    CountRate,
    /// Histogram sum divided by its count.
    Mean,
    P50,
    P90,
    P95,
    P99,
}

impl PanelAggregation {
    /// Every aggregation, in declaration order (used by the JSON Schema).
    pub const ALL: [Self; 13] = [
        Self::Avg,
        Self::Max,
        Self::Min,
        Self::Last,
        Self::Sum,
        Self::Rate,
        Self::Count,
        Self::CountRate,
        Self::Mean,
        Self::P50,
        Self::P90,
        Self::P95,
        Self::P99,
    ];

    /// The quantile this aggregation reads from a distribution, if any.
    pub fn quantile(self) -> Option<f64> {
        match self {
            Self::P50 => Some(0.50),
            Self::P90 => Some(0.90),
            Self::P95 => Some(0.95),
            Self::P99 => Some(0.99),
            _ => None,
        }
    }
}

/// The Google SRE golden signal a panel stands for.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GoldenSignal {
    Traffic,
    Errors,
    Latency,
    Saturation,
}

impl GoldenSignal {
    /// Every signal, in declaration order (used by the JSON Schema).
    pub const ALL: [Self; 4] = [Self::Traffic, Self::Errors, Self::Latency, Self::Saturation];
}

/// Comparison applied to one point attribute before aggregation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterOp {
    Eq,
    Ne,
    Ge,
    Gt,
    Le,
    Lt,
    Prefix,
}

impl FilterOp {
    /// Every operator, in declaration order (used by the JSON Schema).
    pub const ALL: [Self; 7] = [
        Self::Eq,
        Self::Ne,
        Self::Ge,
        Self::Gt,
        Self::Le,
        Self::Lt,
        Self::Prefix,
    ];
}

/// Keeps only series whose attribute satisfies the comparison.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PanelFilter {
    pub key: String,
    pub op: FilterOp,
    pub value: serde_json::Value,
}

/// Splits a panel into one series per group, keeping the top N.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelGroupBy {
    /// One series per measured entity (e.g. per process).
    Entity,
    /// One series per value of a point attribute.
    Attribute(String),
}

/// Thresholds turning a panel's latest value into a health state.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PanelThresholds {
    pub warning: f64,
    pub critical: f64,
    /// `true` when larger values are worse (usage, latency, errors).
    #[serde(default = "higher_is_worse")]
    pub higher_is_worse: bool,
}

fn higher_is_worse() -> bool {
    true
}

/// One panel definition, shared by built-in profiles and custom boards.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PanelSpec {
    pub id: String,
    pub title: String,
    /// Exact metric name as stored (OTel or Muse metric code).
    pub metric: String,
    pub aggregation: PanelAggregation,
    #[serde(default)]
    pub filters: Vec<PanelFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_by: Option<PanelGroupBy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_n: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<GoldenSignal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thresholds: Option<PanelThresholds>,
}

impl PanelSpec {
    pub fn validate(&self) -> Result<(), DashboardError> {
        let identifier = |value: &str| {
            !value.is_empty()
                && value.len() <= 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_-.".contains(&byte))
        };
        if !identifier(&self.id) {
            return Err(DashboardError::Invalid("panel id".into()));
        }
        if self.title.is_empty() || self.title.len() > 120 {
            return Err(DashboardError::Invalid("panel title".into()));
        }
        if self.metric.is_empty() || self.metric.len() > 255 {
            return Err(DashboardError::Invalid("panel metric".into()));
        }
        // Any number of filters and series (owner decision 2026-10-05:
        // dashboards are limitless); only empty keys and a zero top_n are
        // refused.
        if self.filters.iter().any(|filter| filter.key.is_empty()) {
            return Err(DashboardError::Invalid("panel filters".into()));
        }
        if self.top_n == Some(0) {
            return Err(DashboardError::Invalid(
                "panel top_n must be at least 1".into(),
            ));
        }
        if let Some(thresholds) = &self.thresholds {
            let ordered = if thresholds.higher_is_worse {
                thresholds.warning <= thresholds.critical
            } else {
                thresholds.warning >= thresholds.critical
            };
            if !thresholds.warning.is_finite() || !thresholds.critical.is_finite() || !ordered {
                return Err(DashboardError::Invalid("panel thresholds".into()));
            }
        }
        Ok(())
    }
}

/// How a profile is recognized from data. Any listed entity kind, identity
/// kind or root attribute matches; otherwise at least `min_metrics` metrics
/// with one of the prefixes must be present.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileRecognition {
    #[serde(default)]
    pub entity_kinds: Vec<String>,
    #[serde(default)]
    pub identity_kinds: Vec<String>,
    /// Attributes of the source root, for sources whose metric names are
    /// shared with other kinds (a Muse's `database.*` for Redis).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub root_attributes: Vec<AttributeRule>,
    /// OpenTelemetry instrumentation scope names (for example the Collector's
    /// PostgreSQL receiver) that recognize a dashboard pack. Added with
    /// [`DashboardDefinition`]; matchers that predate it ignore the field.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scope_names: Vec<String>,
    #[serde(default)]
    pub metric_prefixes: Vec<String>,
    #[serde(default = "one")]
    pub min_metrics: usize,
}

fn one() -> usize {
    1
}

/// One source-root attribute that recognizes a profile: the key is present
/// and, when `value` is given, its text equals it.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AttributeRule {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

/// Invalid panel, profile or board (kept from the product `model` crate).
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum DashboardError {
    #[error("invalid dashboard {0}")]
    Invalid(String),
}

// A definition holds any number of panels and blocks (owner decision
// 2026-10-05: dashboards are limitless). Only byte lengths of strings are
// capped here; the transport's request-size limit bounds the whole batch.
/// Most bytes of a block's text.
pub const MAX_BLOCK_TEXT_BYTES: usize = 4096;
/// Most bytes of a definition's description.
pub const MAX_DESCRIPTION_BYTES: usize = 1024;
/// Most bytes of a definition id (`<muse_kind>.<name>`).
pub const MAX_DEFINITION_ID_BYTES: usize = 96;
/// Most bytes of a Muse kind.
pub const MAX_MUSE_KIND_BYTES: usize = 32;
/// Most entries of each list in [`DashboardAppliesTo`].
pub const MAX_APPLIES_TO_ENTRIES: usize = 32;
/// Most definitions one [`crate::GraphBatch`] may carry.
pub const MAX_BATCH_DASHBOARDS: usize = 32;

/// Which sources a definition is drawn for.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum DashboardAppliesTo {
    /// The owning Muse's own source roots, by entity kind or identity kind
    /// (any listed kind matches). Only sources fed by a Muse of
    /// `muse_kind` qualify.
    MuseRoots {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        entity_kinds: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        identity_kinds: Vec<String>,
    },
    /// A pack for any source whose data matches the rules (instrumentation
    /// scope names, metric name prefixes, root attributes, entity kinds).
    Recognition(ProfileRecognition),
}

/// A titled group of panels in a definition, drawn in order after the
/// implicit "Measurements" row (one tile per golden signal, fed by the
/// panels that carry it). Each panel's chart is drawn once: in the block
/// that lists it, else under Measurements, so a panel in no block needs a
/// golden signal, and a block may list a golden-signal panel (its value
/// still feeds the tile). See `docs/contract/index.md`.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DashboardBlock {
    pub label: String,
    /// Plain text or Markdown shown under the label, at most
    /// [`MAX_BLOCK_TEXT_BYTES`] bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Ids of panels of the same definition, in drawing order.
    pub panels: Vec<String>,
}

/// One default dashboard a Muse's author defines in the Muse's code. Poet
/// keeps every `(id, revision)` it receives with its origin; the author bumps
/// `revision` whenever the content changes.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DashboardDefinition {
    /// `<muse_kind>.<name>`, for example `macos.host`.
    pub id: String,
    /// Starts at 1; a changed definition carries a higher revision.
    pub revision: u32,
    pub title: String,
    pub description: String,
    /// The Muse kind that owns the definition (a pack names its namespace).
    pub muse_kind: String,
    /// Semver requirement on the Muse version the definition suits
    /// (for example `>=0.3, <0.5`); `None` suits every version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muse_versions: Option<String>,
    pub applies_to: DashboardAppliesTo,
    pub panels: Vec<PanelSpec>,
    /// Layout after the implicit "Measurements" row. Empty: the renderer
    /// derives the layout from the panels.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<DashboardBlock>,
    /// Panels per row, 2..=4; `None` lets the renderer choose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub columns: Option<u8>,
}

/// Why a [`DashboardDefinition`] is rejected.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum DashboardDefinitionError {
    #[error("muse_kind must be 1..={MAX_MUSE_KIND_BYTES} bytes of a-z, 0-9, '_' or '-'")]
    MuseKind,
    #[error("id must be `<muse_kind>.<name>` (a-z, 0-9, '_', '-', '.'), at most {MAX_DEFINITION_ID_BYTES} bytes")]
    Id,
    #[error("revision must be greater than 0")]
    Revision,
    #[error("title must be 1..=120 bytes")]
    Title,
    #[error("description must be at most {MAX_DESCRIPTION_BYTES} bytes")]
    Description,
    #[error("muse_versions must be 1..=64 printable ASCII bytes")]
    MuseVersions,
    #[error("applies_to must list 1..={MAX_APPLIES_TO_ENTRIES} non-empty matchers per list (each at most 255 bytes, min_metrics >= 1)")]
    AppliesTo,
    #[error("a definition needs at least one panel")]
    PanelCount,
    #[error("panel {index}: {source}")]
    Panel {
        index: usize,
        source: DashboardError,
    },
    #[error("duplicate panel id {0}")]
    DuplicatePanel(String),
    #[error("block {index}: label must be 1..=80 bytes")]
    BlockLabel { index: usize },
    #[error("block {index}: text must be at most {MAX_BLOCK_TEXT_BYTES} bytes")]
    BlockText { index: usize },
    #[error("block {index}: needs at least one panel")]
    BlockPanels { index: usize },
    #[error("block {index}: unknown panel id {panel}")]
    UnknownBlockPanel { index: usize, panel: String },
    #[error("block {index}: panel {panel} is already placed in a block")]
    PanelInTwoBlocks { index: usize, panel: String },
    #[error("columns must be 2..=4")]
    Columns,
}

impl DashboardDefinition {
    /// Checks every rule a sender and a receiver enforce before storage.
    pub fn validate(&self) -> Result<(), DashboardDefinitionError> {
        use DashboardDefinitionError as E;
        let kind_byte =
            |byte: u8| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_-".contains(&byte);
        if self.muse_kind.is_empty()
            || self.muse_kind.len() > MAX_MUSE_KIND_BYTES
            || !self.muse_kind.bytes().all(kind_byte)
        {
            return Err(E::MuseKind);
        }
        let name = self
            .id
            .strip_prefix(self.muse_kind.as_str())
            .and_then(|rest| rest.strip_prefix('.'));
        if self.id.len() > MAX_DEFINITION_ID_BYTES
            || !name.is_some_and(|name| {
                !name.is_empty()
                    && !name.starts_with('.')
                    && !name.ends_with('.')
                    && name.bytes().all(|byte| kind_byte(byte) || byte == b'.')
            })
        {
            return Err(E::Id);
        }
        if self.revision == 0 {
            return Err(E::Revision);
        }
        if self.title.trim().is_empty() || self.title.len() > 120 {
            return Err(E::Title);
        }
        if self.description.len() > MAX_DESCRIPTION_BYTES {
            return Err(E::Description);
        }
        if self.muse_versions.as_ref().is_some_and(|versions| {
            versions.trim().is_empty()
                || versions.len() > 64
                || !versions.bytes().all(|byte| (b' '..=b'~').contains(&byte))
        }) {
            return Err(E::MuseVersions);
        }
        self.validate_applies_to()?;
        if self.panels.is_empty() {
            return Err(E::PanelCount);
        }
        let mut panels = BTreeSet::new();
        for (index, panel) in self.panels.iter().enumerate() {
            panel
                .validate()
                .map_err(|source| E::Panel { index, source })?;
            if !panels.insert(panel.id.as_str()) {
                return Err(E::DuplicatePanel(panel.id.clone()));
            }
        }
        let mut placed = BTreeSet::new();
        for (index, block) in self.blocks.iter().enumerate() {
            if block.label.trim().is_empty() || block.label.len() > 80 {
                return Err(E::BlockLabel { index });
            }
            if block
                .text
                .as_ref()
                .is_some_and(|text| text.len() > MAX_BLOCK_TEXT_BYTES)
            {
                return Err(E::BlockText { index });
            }
            if block.panels.is_empty() {
                return Err(E::BlockPanels { index });
            }
            for panel in &block.panels {
                if !panels.contains(panel.as_str()) {
                    return Err(E::UnknownBlockPanel {
                        index,
                        panel: panel.clone(),
                    });
                }
                if !placed.insert(panel.as_str()) {
                    return Err(E::PanelInTwoBlocks {
                        index,
                        panel: panel.clone(),
                    });
                }
            }
        }
        if self
            .columns
            .is_some_and(|columns| !(2..=4).contains(&columns))
        {
            return Err(E::Columns);
        }
        Ok(())
    }

    fn validate_applies_to(&self) -> Result<(), DashboardDefinitionError> {
        let list = |values: &[String]| {
            values.len() <= MAX_APPLIES_TO_ENTRIES
                && values
                    .iter()
                    .all(|value| !value.is_empty() && value.len() <= 255)
        };
        let valid = match &self.applies_to {
            DashboardAppliesTo::MuseRoots {
                entity_kinds,
                identity_kinds,
            } => {
                list(entity_kinds)
                    && list(identity_kinds)
                    && !(entity_kinds.is_empty() && identity_kinds.is_empty())
            }
            DashboardAppliesTo::Recognition(rules) => {
                list(&rules.entity_kinds)
                    && list(&rules.identity_kinds)
                    && list(&rules.scope_names)
                    && list(&rules.metric_prefixes)
                    && rules.root_attributes.len() <= MAX_APPLIES_TO_ENTRIES
                    && rules.root_attributes.iter().all(|rule| {
                        !rule.key.is_empty()
                            && rule.key.len() <= 255
                            && rule.value.as_ref().map_or(true, |value| value.len() <= 255)
                    })
                    && (1..=64).contains(&rules.min_metrics)
                    && !(rules.entity_kinds.is_empty()
                        && rules.identity_kinds.is_empty()
                        && rules.scope_names.is_empty()
                        && rules.metric_prefixes.is_empty()
                        && rules.root_attributes.is_empty())
            }
        };
        if valid {
            Ok(())
        } else {
            Err(DashboardDefinitionError::AppliesTo)
        }
    }
}

/// `$id` of the committed JSON Schema for [`DashboardDefinition`].
pub const DASHBOARD_DEFINITION_SCHEMA_ID: &str =
    "https://infinitehaiku.com/schemas/dashboard-definition.schema.json";

/// JSON Schema (draft 2020-12) of [`DashboardDefinition`], built from the
/// types' variant lists and caps. The committed copy is
/// `schemas/dashboard-definition.schema.json`; tests fail when it drifts from
/// this function or from the serialized types. `schemars` derive is not used
/// because its derive crate is unavailable to offline builds; switching to it
/// later must keep this output's meaning.
pub fn dashboard_definition_schema() -> serde_json::Value {
    use serde_json::json;
    let names = |values: Vec<serde_json::Value>| -> Vec<String> {
        values
            .into_iter()
            .map(|value| {
                value
                    .as_str()
                    .expect("unit variants serialize as strings")
                    .to_owned()
            })
            .collect()
    };
    let enumerate = |values: Vec<String>| json!({"type": "string", "enum": values});
    let aggregations = names(
        PanelAggregation::ALL
            .iter()
            .map(|value| json!(value))
            .collect(),
    );
    let signals = names(GoldenSignal::ALL.iter().map(|value| json!(value)).collect());
    let ops = names(FilterOp::ALL.iter().map(|value| json!(value)).collect());
    let strings = |max_items: usize| json!({"type": "array", "maxItems": max_items, "items": {"type": "string", "minLength": 1, "maxLength": 255}});
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": DASHBOARD_DEFINITION_SCHEMA_ID,
        "title": "DashboardDefinition",
        "description": "A default dashboard a Muse's author defines in the Muse's code (or a pack for OpenTelemetry sources). Data only: no queries, scripts, URLs or credentials. The Rust validate() also enforces cross-field rules (id prefix, block panel ids, threshold order).",
        "type": "object",
        "additionalProperties": false,
        "required": ["id", "revision", "title", "description", "muse_kind", "applies_to", "panels"],
        "properties": {
            "id": {"type": "string", "description": "`<muse_kind>.<name>`, for example `macos.host`.", "maxLength": MAX_DEFINITION_ID_BYTES, "pattern": "^[a-z0-9_-]+\\.[a-z0-9_.-]*[a-z0-9_-]$"},
            "revision": {"type": "integer", "description": "Starts at 1; bumped whenever the content changes.", "minimum": 1, "maximum": u32::MAX},
            "title": {"type": "string", "minLength": 1, "maxLength": 120},
            "description": {"type": "string", "maxLength": MAX_DESCRIPTION_BYTES},
            "muse_kind": {"type": "string", "description": "The Muse kind that owns the definition.", "minLength": 1, "maxLength": MAX_MUSE_KIND_BYTES, "pattern": "^[a-z0-9_-]+$"},
            "muse_versions": {"type": "string", "description": "Semver requirement on the Muse version the definition suits.", "minLength": 1, "maxLength": 64},
            "applies_to": {"$ref": "#/$defs/DashboardAppliesTo"},
            "panels": {"type": "array", "minItems": 1, "items": {"$ref": "#/$defs/PanelSpec"}},
            "blocks": {"type": "array", "items": {"$ref": "#/$defs/DashboardBlock"}},
            "columns": {"type": "integer", "description": "Panels per row.", "minimum": 2, "maximum": 4}
        },
        "$defs": {
            "DashboardAppliesTo": {
                "description": "Which sources the definition is drawn for: the owning Muse's own roots, or recognition rules for a pack.",
                "oneOf": [
                    {
                        "type": "object", "additionalProperties": false, "required": ["muse_roots"],
                        "properties": {"muse_roots": {
                            "type": "object", "additionalProperties": false,
                            "properties": {"entity_kinds": strings(MAX_APPLIES_TO_ENTRIES), "identity_kinds": strings(MAX_APPLIES_TO_ENTRIES)}
                        }}
                    },
                    {
                        "type": "object", "additionalProperties": false, "required": ["recognition"],
                        "properties": {"recognition": {"$ref": "#/$defs/ProfileRecognition"}}
                    }
                ]
            },
            "ProfileRecognition": {
                "description": "Any listed entity kind, identity kind, scope name or root attribute matches; otherwise at least min_metrics metrics with one of the prefixes.",
                "type": "object", "additionalProperties": false,
                "properties": {
                    "entity_kinds": strings(MAX_APPLIES_TO_ENTRIES),
                    "identity_kinds": strings(MAX_APPLIES_TO_ENTRIES),
                    "root_attributes": {"type": "array", "maxItems": MAX_APPLIES_TO_ENTRIES, "items": {"$ref": "#/$defs/AttributeRule"}},
                    "scope_names": strings(MAX_APPLIES_TO_ENTRIES),
                    "metric_prefixes": strings(MAX_APPLIES_TO_ENTRIES),
                    "min_metrics": {"type": "integer", "minimum": 1, "maximum": 64, "default": 1}
                }
            },
            "AttributeRule": {
                "type": "object", "additionalProperties": false, "required": ["key"],
                "properties": {"key": {"type": "string", "minLength": 1, "maxLength": 255}, "value": {"type": "string", "maxLength": 255}}
            },
            "DashboardBlock": {
                "description": "A titled group of panels drawn after the implicit Measurements row (panels with a golden signal).",
                "type": "object", "additionalProperties": false, "required": ["label", "panels"],
                "properties": {
                    "label": {"type": "string", "minLength": 1, "maxLength": 80},
                    "text": {"type": "string", "description": "Plain text or Markdown.", "maxLength": MAX_BLOCK_TEXT_BYTES},
                    "panels": {"type": "array", "minItems": 1, "items": {"type": "string"}}
                }
            },
            "PanelSpec": {
                "type": "object", "additionalProperties": false, "required": ["id", "title", "metric", "aggregation"],
                "properties": {
                    "id": {"type": "string", "minLength": 1, "maxLength": 64, "pattern": "^[A-Za-z0-9_.-]+$"},
                    "title": {"type": "string", "minLength": 1, "maxLength": 120},
                    "metric": {"type": "string", "description": "Exact metric name as stored (OTel or Muse metric code).", "minLength": 1, "maxLength": 255},
                    "aggregation": enumerate(aggregations),
                    "filters": {"type": "array", "items": {"$ref": "#/$defs/PanelFilter"}},
                    "group_by": {"$ref": "#/$defs/PanelGroupBy"},
                    "top_n": {"type": "integer", "description": "Series shown when grouped (any number).", "minimum": 1},
                    "signal": enumerate(signals),
                    "thresholds": {"$ref": "#/$defs/PanelThresholds"}
                }
            },
            "PanelFilter": {
                "type": "object", "additionalProperties": false, "required": ["key", "op", "value"],
                "properties": {"key": {"type": "string", "minLength": 1}, "op": enumerate(ops), "value": {}}
            },
            "PanelGroupBy": {
                "oneOf": [
                    {"type": "string", "enum": ["entity"], "description": "One series per measured entity."},
                    {"type": "object", "additionalProperties": false, "required": ["attribute"], "properties": {"attribute": {"type": "string", "minLength": 1}}}
                ]
            },
            "PanelThresholds": {
                "type": "object", "additionalProperties": false, "required": ["warning", "critical"],
                "properties": {"warning": {"type": "number"}, "critical": {"type": "number"}, "higher_is_worse": {"type": "boolean", "default": true}}
            }
        }
    })
}
