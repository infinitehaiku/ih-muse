//! Dashboard definitions: validation, the GraphBatch wire field, and the
//! committed JSON Schema and example staying in step with the Rust types.

use ih_muse_proto::dashboard::{
    dashboard_definition_schema, AttributeRule, DashboardError, FilterOp, GoldenSignal,
    PanelAggregation, PanelColumn, PanelFilter, PanelGroupBy, PanelKind, PanelSize, PanelSpec,
    PanelQuery, PanelStream, PanelThresholds, ProfileRecognition, SectionColor,
};
use ih_muse_proto::{
    DashboardAppliesTo, DashboardBlock, DashboardDefinition, DashboardDefinitionError, GraphBatch,
    GraphValidationError, GRAPH_CONTRACT_REVISION, GRAPH_SCHEMA_VERSION,
};
use serde_json::{json, Value};

fn workspace_file(path: &str) -> String {
    format!("{}/../../{path}", env!("CARGO_MANIFEST_DIR"))
}

fn example() -> DashboardDefinition {
    let text =
        std::fs::read_to_string(workspace_file("examples/dashboards/macos-host.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

/// Every optional field set, every variant kind used: the instance the
/// schema coverage check walks.
fn full_definition() -> DashboardDefinition {
    let panel = |id: &str, group_by| PanelSpec {
        id: id.into(),
        title: format!("Panel {id}"),
        metric: "http.server.request.duration".into(),
        aggregation: PanelAggregation::P95,
        filters: vec![PanelFilter {
            key: "http.route".into(),
            op: FilterOp::Prefix,
            value: json!("/api"),
        }],
        group_by: Some(group_by),
        top_n: Some(5),
        signal: Some(GoldenSignal::Latency),
        thresholds: Some(PanelThresholds {
            warning: 0.5,
            critical: 2.0,
            higher_is_worse: true,
        }),
        ..PanelSpec::default()
    };
    let mut table = panel("b", PanelGroupBy::Attribute("db".into()));
    table.kind = PanelKind::Table;
    table.size = Some(PanelSize { w: 6, h: 4 });
    table.columns = vec![PanelColumn {
        title: "Rows read".into(),
        metric: "postgresql.rows".into(),
        aggregation: PanelAggregation::Rate,
        filters: vec![PanelFilter {
            key: "state".into(),
            op: FilterOp::Eq,
            value: json!("read"),
        }],
    }];
    let text = PanelSpec {
        id: "t".into(),
        title: "About".into(),
        kind: PanelKind::Text,
        text: Some("# PostgreSQL\nRead **slow** queries first.".into()),
        ..PanelSpec::default()
    };
    let logs = PanelSpec {
        id: "l".into(),
        title: "Errors in the log".into(),
        kind: PanelKind::Logs,
        stream: Some(PanelStream {
            search: Some("timeout".into()),
            errors_only: true,
            limit: Some(30),
        }),
        ..PanelSpec::default()
    };
    let events = PanelSpec {
        id: "e".into(),
        title: "What happened".into(),
        kind: PanelKind::Events,
        stream: Some(PanelStream { search: Some("deploy".into()), errors_only: false, limit: Some(50) }),
        ..PanelSpec::default()
    };
    let trace = PanelSpec {
        id: "tr".into(),
        title: "Slow request".into(),
        kind: PanelKind::Trace,
        trace_id: Some("0af7651916cd43dd8448eb211c80319c".into()),
        ..PanelSpec::default()
    };
    let query = PanelSpec {
        id: "q".into(),
        title: "Slow operations".into(),
        kind: PanelKind::Query,
        query: Some(PanelQuery {
            tool: "search".into(),
            arguments: json!({"signal": "spans", "entity": "kabuki", "group_by": "operation"}).as_object().unwrap().clone(),
        }),
        ..PanelSpec::default()
    };
    DashboardDefinition {
        id: "otel-postgresql.server".into(),
        revision: 3,
        title: "PostgreSQL".into(),
        description: "Pack for the Collector PostgreSQL receiver.".into(),
        muse_kind: "otel-postgresql".into(),
        muse_versions: Some(">=0.1".into()),
        applies_to: DashboardAppliesTo::Recognition(ProfileRecognition {
            entity_kinds: vec!["postgresql".into()],
            identity_kinds: vec!["service".into()],
            root_attributes: vec![AttributeRule {
                key: "db.system".into(),
                value: Some("postgresql".into()),
            }],
            scope_names: vec!["otelcol/postgresqlreceiver".into()],
            metric_prefixes: vec!["postgresql.".into()],
            min_metrics: 2,
        }),
        panels: vec![panel("a", PanelGroupBy::Entity), table, text, logs, events, trace, query],
        blocks: vec![DashboardBlock {
            label: "Queries".into(),
            text: Some("**Slow** first.".into()),
            panels: vec!["a".into(), "b".into(), "t".into(), "l".into(), "e".into(), "tr".into(), "q".into()],
            color: Some(SectionColor::Purple),
            collapsed: true,
            width: Some(6),
            ..Default::default()
        }],
        columns: Some(4),
    }
}

/// A minimal JSON Schema checker for the keywords the dashboard schema uses
/// (no regex: `pattern` is enforced by `validate()` instead). Returns errors
/// with their JSON pointer.
fn check(schema: &Value, root: &Value, value: &Value, at: &str, errors: &mut Vec<String>) {
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        let name = reference.strip_prefix("#/$defs/").expect("local $defs ref");
        return check(&root["$defs"][name], root, value, at, errors);
    }
    if let Some(options) = schema.get("oneOf").and_then(Value::as_array) {
        let matching = options
            .iter()
            .filter(|option| {
                let mut inner = Vec::new();
                check(option, root, value, at, &mut inner);
                inner.is_empty()
            })
            .count();
        if matching != 1 {
            errors.push(format!("{at}: matches {matching} oneOf branches"));
        }
        return;
    }
    let kind = schema.get("type").and_then(Value::as_str);
    let fits = match kind {
        Some("object") => value.is_object(),
        Some("array") => value.is_array(),
        Some("string") => value.is_string(),
        Some("integer") => value.is_u64() || value.is_i64(),
        Some("number") => value.is_number(),
        Some("boolean") => value.is_boolean(),
        Some(other) => panic!("unsupported type {other}"),
        None => true,
    };
    if !fits {
        return errors.push(format!("{at}: expected {kind:?}, got {value}"));
    }
    if let Some(allowed) = schema.get("enum").and_then(Value::as_array) {
        if !allowed.contains(value) {
            errors.push(format!("{at}: {value} not in enum"));
        }
    }
    let bound = |key: &str| schema.get(key).and_then(Value::as_f64);
    if let Some(number) = value.as_f64() {
        if bound("minimum").is_some_and(|min| number < min)
            || bound("maximum").is_some_and(|max| number > max)
        {
            errors.push(format!("{at}: {number} out of range"));
        }
    }
    let length = value
        .as_str()
        .map(str::len)
        .or(value.as_array().map(Vec::len));
    if let Some(length) = length {
        let (min, max) = if value.is_string() {
            ("minLength", "maxLength")
        } else {
            ("minItems", "maxItems")
        };
        if bound(min).is_some_and(|min| (length as f64) < min)
            || bound(max).is_some_and(|max| (length as f64) > max)
        {
            errors.push(format!("{at}: length {length} out of range"));
        }
    }
    if let (Some(items), Some(values)) = (schema.get("items"), value.as_array()) {
        for (index, item) in values.iter().enumerate() {
            check(items, root, item, &format!("{at}/{index}"), errors);
        }
    }
    if let Some(object) = value.as_object() {
        let properties = schema.get("properties").and_then(Value::as_object);
        for required in schema
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if !object.contains_key(required.as_str().unwrap()) {
                errors.push(format!("{at}: missing {required}"));
            }
        }
        for (key, item) in object {
            match properties.and_then(|properties| properties.get(key)) {
                Some(property) => check(property, root, item, &format!("{at}/{key}"), errors),
                None if schema.get("additionalProperties") == Some(&Value::Bool(false)) => {
                    errors.push(format!("{at}: unknown property {key}"))
                }
                None => {}
            }
        }
    }
}

fn schema_errors(value: &Value) -> Vec<String> {
    let schema = dashboard_definition_schema();
    let mut errors = Vec::new();
    check(&schema, &schema, value, "", &mut errors);
    errors
}

/// Every property a schema object declares, by `$defs` name, must appear in
/// the fully populated instance, and the reverse: fields cannot drift.
fn declared_properties(schema: &Value, root: &Value, value: &Value, seen: &mut Vec<String>) {
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        return declared_properties(
            &root["$defs"][&reference["#/$defs/".len()..]],
            root,
            value,
            seen,
        );
    }
    if let Some(options) = schema.get("oneOf").and_then(Value::as_array) {
        for option in options {
            if schema_errors_for(option, root, value).is_empty() {
                declared_properties(option, root, value, seen);
            }
        }
        return;
    }
    if let (Some(properties), Some(object)) = (
        schema.get("properties").and_then(Value::as_object),
        value.as_object(),
    ) {
        for (key, property) in properties {
            seen.push(key.clone());
            if let Some(item) = object.get(key) {
                declared_properties(property, root, item, seen);
            }
        }
    }
    if let (Some(items), Some(values)) = (schema.get("items"), value.as_array()) {
        values
            .iter()
            .for_each(|item| declared_properties(items, root, item, seen));
    }
}

fn schema_errors_for(schema: &Value, root: &Value, value: &Value) -> Vec<String> {
    let mut errors = Vec::new();
    check(schema, root, value, "", &mut errors);
    errors
}

/// Every key the instance serializes; a query panel's `arguments` is the
/// door call's own free-form object (the schema declares it as an open
/// object), so its keys are not contract fields.
fn serialized_keys(value: &Value, keys: &mut Vec<String>) {
    match value {
        Value::Object(object) => object.iter().for_each(|(key, item)| {
            keys.push(key.clone());
            if key != "arguments" {
                serialized_keys(item, keys);
            }
        }),
        Value::Array(values) => values.iter().for_each(|item| serialized_keys(item, keys)),
        _ => {}
    }
}

#[test]
fn committed_schema_matches_the_generated_one() {
    let path = workspace_file("schemas/dashboard-definition.schema.json");
    let generated = serde_json::to_string_pretty(&dashboard_definition_schema()).unwrap() + "\n";
    if std::env::var_os("IH_UPDATE_DASHBOARD_SCHEMA").is_some() {
        std::fs::write(&path, &generated).unwrap();
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == generated,
        "{path} is stale: run `IH_UPDATE_DASHBOARD_SCHEMA=1 cargo test -p ih-muse-proto --test dashboard_contract`"
    );
}

#[test]
fn schema_covers_every_serialized_field_and_nothing_else() {
    let mut full = serde_json::to_value(full_definition()).unwrap();
    // The other applies_to branch, so both oneOf sides are walked.
    let mut roots = full.clone();
    roots["applies_to"] =
        json!({"muse_roots": {"entity_kinds": ["macos_host"], "identity_kinds": ["host"]}});
    for instance in [&mut full, &mut roots] {
        assert_eq!(schema_errors(instance), Vec::<String>::new());
        let schema = dashboard_definition_schema();
        let mut declared = Vec::new();
        declared_properties(&schema, &schema, instance, &mut declared);
        let mut keys = Vec::new();
        serialized_keys(instance, &mut keys);
        let (declared, keys) = (dedup(declared), dedup(keys));
        assert_eq!(declared, keys, "schema properties vs serialized keys");
    }
}

fn dedup(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values.dedup();
    values
}

#[test]
fn schema_rejects_what_serde_rejects() {
    let base = serde_json::to_value(example()).unwrap();
    let cases = [
        ("/extra", json!(1)),
        ("/panels/0/aggregation", json!("median")),
        ("/columns", json!(5)),
        ("/applies_to", json!({"muse_roots": {}, "recognition": {}})),
        ("/panels/0/group_by", json!("process")),
    ];
    for (pointer, replacement) in cases {
        let mut value = base.clone();
        let (parent, key) = pointer.rsplit_once('/').unwrap();
        value
            .pointer_mut(if parent.is_empty() { "" } else { parent })
            .unwrap()[key] = replacement;
        assert!(
            !schema_errors(&value).is_empty(),
            "schema accepted {pointer}"
        );
        let parsed = serde_json::from_value::<DashboardDefinition>(value)
            .map_err(|error| error.to_string())
            .and_then(|definition| definition.validate().map_err(|error| error.to_string()));
        assert!(parsed.is_err(), "Rust accepted {pointer}");
    }
}

#[test]
fn the_macos_example_is_valid_and_matches_the_owner_mock() {
    let definition = example();
    definition.validate().unwrap();
    assert!(schema_errors(&serde_json::to_value(&definition).unwrap()).is_empty());
    assert_eq!(definition.id, "macos.host");
    assert_eq!(definition.columns, Some(3));
    let labels: Vec<_> = definition
        .blocks
        .iter()
        .map(|block| block.label.as_str())
        .collect();
    assert_eq!(labels, ["Host: over time", "Host: what uses the machine"]);
    // Measurements (golden signals) is implicit; every other panel is in a block.
    for panel in &definition.panels {
        let placed = definition
            .blocks
            .iter()
            .any(|block| block.panels.contains(&panel.id));
        assert_eq!(placed, panel.signal.is_none(), "{}", panel.id);
    }
    let uses = &definition.blocks[1].panels;
    assert!(uses.iter().all(|id| definition
        .panels
        .iter()
        .any(|panel| &panel.id == id && panel.group_by.is_some())));
}

/// Every example under `examples/dashboards` is valid, passes the JSON
/// Schema (raw file and serialized form), and places each panel exactly
/// once ([`placement_problems`]).
#[test]
fn every_shipped_example_is_valid_and_places_each_panel_once() {
    let dir = workspace_file("examples/dashboards");
    let mut ids = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let raw: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(
            schema_errors(&raw).is_empty(),
            "{path:?}: {:?}",
            schema_errors(&raw)
        );
        let definition: DashboardDefinition = serde_json::from_value(raw).unwrap();
        definition.validate().unwrap();
        assert!(schema_errors(&serde_json::to_value(&definition).unwrap()).is_empty());
        assert_eq!(placement_problems(&definition), Vec::<String>::new(), "{path:?}");
        ids.push(definition.id);
    }
    ids.sort();
    assert_eq!(ids, ["k8s.cluster", "macos.host"]);
}

/// Panels a definition draws nowhere or in the wrong place. A panel's chart
/// is drawn once: in the one block that lists it, else in the implicit
/// Measurements row, which only holds panels with a golden signal (a
/// definition without blocks lets the renderer lay everything out). A
/// golden signal also feeds the signal's Measurements tile wherever the
/// panel's chart is, so a block may list a signal panel (the Kubernetes
/// cluster's "Health now" lists its Errors panels). `validate()` already
/// refuses a panel in two blocks.
fn placement_problems(definition: &DashboardDefinition) -> Vec<String> {
    if definition.blocks.is_empty() {
        return Vec::new();
    }
    definition
        .panels
        .iter()
        .filter(|panel| panel.signal.is_none())
        .filter(|panel| {
            !definition
                .blocks
                .iter()
                .any(|block| block.panels.contains(&panel.id))
        })
        .map(|panel| format!("{} has no golden signal and is in no block", panel.id))
        .collect()
}

#[test]
fn a_signal_panel_may_sit_in_a_block_but_a_plain_panel_needs_one() {
    let mut definition = example();
    assert_eq!(placement_problems(&definition), Vec::<String>::new());
    // A golden-signal panel listed in a block: still placed once (its chart in
    // the block, its value in the Measurements tile).
    let signal = definition
        .panels
        .iter()
        .find(|panel| panel.signal.is_some())
        .unwrap()
        .id
        .clone();
    definition.blocks[0].panels.push(signal);
    definition.validate().unwrap();
    assert_eq!(placement_problems(&definition), Vec::<String>::new());
    // A panel without a signal taken out of every block is drawn nowhere it
    // belongs.
    let plain = definition.blocks[1].panels.remove(0);
    assert_eq!(
        placement_problems(&definition),
        [format!("{plain} has no golden signal and is in no block")]
    );
}

#[test]
fn invalid_definitions_name_their_rule() {
    use DashboardDefinitionError as E;
    type Case = (fn(&mut DashboardDefinition), E);
    let cases: Vec<Case> = vec![
        (|d| d.id = "linux.host".into(), E::Id),
        (|d| d.id = "macos".into(), E::Id),
        (|d| d.id = "macos.".into(), E::Id),
        (|d| d.muse_kind = "MacOS".into(), E::MuseKind),
        (|d| d.revision = 0, E::Revision),
        (|d| d.title = " ".into(), E::Title),
        (|d| d.description = "x".repeat(1025), E::Description),
        (|d| d.muse_versions = Some(String::new()), E::MuseVersions),
        (
            |d| {
                d.applies_to = DashboardAppliesTo::MuseRoots {
                    entity_kinds: vec![],
                    identity_kinds: vec![],
                }
            },
            E::AppliesTo,
        ),
        (
            |d| {
                d.applies_to = DashboardAppliesTo::Recognition(ProfileRecognition {
                    min_metrics: 1,
                    ..Default::default()
                })
            },
            E::AppliesTo,
        ),
        (|d| d.panels.clear(), E::PanelCount),
        (
            |d| d.panels[0].top_n = Some(0),
            E::Panel {
                index: 0,
                source: DashboardError::Invalid("panel top_n must be at least 1".into()),
            },
        ),
        (
            |d| {
                let first = d.panels[0].clone();
                d.panels.push(first)
            },
            E::DuplicatePanel("cpu".into()),
        ),
        (|d| d.blocks[0].label.clear(), E::BlockLabel { index: 0 }),
        (
            |d| d.blocks[1].text = Some("x".repeat(4097)),
            E::BlockText { index: 1 },
        ),
        (|d| d.blocks[0].panels.clear(), E::BlockPanels { index: 0 }),
        (
            |d| d.blocks[0].panels.push("gpu".into()),
            E::UnknownBlockPanel {
                index: 0,
                panel: "gpu".into(),
            },
        ),
        (
            |d| d.blocks[1].panels.push("load".into()),
            E::PanelInTwoBlocks {
                index: 1,
                panel: "load".into(),
            },
        ),
        (|d| d.columns = Some(1), E::Columns),
    ];
    for (change, expected) in cases {
        let mut definition = example();
        change(&mut definition);
        assert_eq!(definition.validate(), Err(expected));
    }
    let mut no_blocks = example();
    (no_blocks.blocks, no_blocks.columns) = (Vec::new(), None);
    no_blocks
        .validate()
        .expect("blocks and columns are optional");
}

/// Dashboards are limitless (owner decision 2026-10-05): a definition with
/// 60 panels in 20 blocks validates, passes the schema and rides a batch.
#[test]
fn a_definition_holds_any_number_of_panels_and_blocks() {
    let mut definition = example();
    let template = definition.panels[0].clone();
    definition.panels = (0..60)
        .map(|index| PanelSpec {
            id: format!("p{index}"),
            title: format!("Panel {index}"),
            signal: None,
            ..template.clone()
        })
        .collect();
    definition.blocks = (0..20)
        .map(|block| DashboardBlock {
            label: format!("Block {block}"),
            text: None,
            panels: (0..3).map(|i| format!("p{}", block * 3 + i)).collect(),
            ..Default::default()
        })
        .collect();
    definition.validate().expect("60 panels in 20 blocks");
    let value = serde_json::to_value(&definition).unwrap();
    assert_eq!(schema_errors(&value), Vec::<String>::new());
    batch(vec![definition]).validate().expect("a batch carries it");
}

#[test]
fn a_panel_holds_any_number_of_filters_and_series() {
    let mut definition = example();
    let panel = &mut definition.panels[0];
    panel.filters = (0..40)
        .map(|index| PanelFilter {
            key: format!("attribute.{index}"),
            op: FilterOp::Eq,
            value: serde_json::json!(index),
        })
        .collect();
    panel.group_by = Some(PanelGroupBy::Entity);
    panel.top_n = Some(200);
    definition.validate().expect("40 filters and 200 series");
    let value = serde_json::to_value(&definition).unwrap();
    assert_eq!(schema_errors(&value), Vec::<String>::new());
    batch(vec![definition.clone()]).validate().expect("a batch carries it");

    // Only an empty filter key and a zero series count stay refused.
    let mut empty_key = definition.clone();
    empty_key.panels[0].filters[39].key.clear();
    assert!(empty_key.validate().is_err());
    let mut zero = definition;
    zero.panels[0].top_n = Some(0);
    assert!(zero.validate().is_err());
    assert!(!schema_errors(&serde_json::to_value(&zero).unwrap()).is_empty());
}

#[test]
fn moved_panel_types_keep_their_json() {
    let text = r#"{"id":"top_cpu","title":"Top processes by CPU","metric":"process.cpu.usage_percent","aggregation":"avg","filters":[{"key":"entity.kind","op":"eq","value":"macos_process"}],"group_by":"entity","top_n":8}"#;
    let spec: PanelSpec = serde_json::from_str(text).unwrap();
    assert_eq!(serde_json::to_string(&spec).unwrap(), text);
    let thresholds: PanelThresholds =
        serde_json::from_str(r#"{"warning":1,"critical":2}"#).unwrap();
    assert!(thresholds.higher_is_worse);
    let recognition: ProfileRecognition =
        serde_json::from_str(r#"{"metric_prefixes":["host."]}"#).unwrap();
    assert_eq!(recognition.min_metrics, 1);
    assert_eq!(
        serde_json::to_string(&recognition).unwrap(),
        r#"{"entity_kinds":[],"identity_kinds":[],"metric_prefixes":["host."],"min_metrics":1}"#,
        "scope_names is omitted when empty"
    );
}

fn batch(dashboards: Vec<DashboardDefinition>) -> GraphBatch {
    GraphBatch {
        schema_version: GRAPH_SCHEMA_VERSION,
        contract_revision: GRAPH_CONTRACT_REVISION.into(),
        entities: Vec::new(),
        relations: Vec::new(),
        observations: Vec::new(),
        events: Vec::new(),
        derivations: Vec::new(),
        availability: Vec::new(),
        dashboards,
    }
}

#[test]
fn graph_batch_carries_dashboards_without_changing_older_json() {
    let empty = serde_json::to_value(batch(Vec::new())).unwrap();
    assert!(empty.get("dashboards").is_none(), "{empty}");
    let older: GraphBatch = serde_json::from_value(empty).unwrap();
    assert!(older.dashboards.is_empty());

    let sent = batch(vec![example()]);
    sent.validate().unwrap();
    let text = serde_json::to_string(&sent).unwrap();
    let received: GraphBatch = serde_json::from_str(&text).unwrap();
    assert_eq!(received, sent);

    let mut invalid = example();
    invalid.revision = 0;
    assert_eq!(
        batch(vec![invalid]).validate(),
        Err(GraphValidationError::InvalidDashboard {
            id: "macos.host".into(),
            source: DashboardDefinitionError::Revision
        })
    );
    let mut newer = example();
    newer.revision = 2;
    assert_eq!(
        batch(vec![example(), newer]).validate(),
        Err(GraphValidationError::DuplicateDashboard(
            "macos.host".into()
        ))
    );
    assert_eq!(
        batch(vec![example(); 33]).validate(),
        Err(GraphValidationError::TooManyDashboards)
    );
}

/// WP dashboard-widgets: every panel kind round trips through JSON, a kind
/// without its fields takes its default size, and an older panel (no kind)
/// reads as a time series and writes back byte for byte.
#[test]
fn every_panel_kind_round_trips_with_its_default_size() {
    for kind in PanelKind::ALL {
        let mut spec = PanelSpec {
            id: "p".into(),
            title: "P".into(),
            kind,
            metric: if kind.reads_metric() { "k8s.pod.phase".into() } else { String::new() },
            aggregation: PanelAggregation::Sum,
            group_by: kind.has_rows().then_some(PanelGroupBy::Attribute("k8s.namespace.name".into())),
            text: (kind == PanelKind::Text).then(|| "Some **text**".into()),
            trace_id: (kind == PanelKind::Trace).then(|| "0af7651916cd43dd8448eb211c80319c".into()),
            query: (kind == PanelKind::Query).then(|| PanelQuery {
                tool: "query".into(),
                arguments: json!({"action": "run", "entity": "kabuki", "metric": "m", "aggregation": "p95"}).as_object().unwrap().clone(),
            }),
            ..PanelSpec::default()
        };
        spec.validate().unwrap_or_else(|error| panic!("{kind:?}: {error}"));
        assert_eq!(spec.grid_size(), kind.default_size());
        spec.size = Some(PanelSize { w: 12, h: 1 });
        let text = serde_json::to_value(&spec).unwrap();
        assert_eq!(serde_json::from_value::<PanelSpec>(text.clone()).unwrap(), spec, "{text}");
        assert_eq!(spec.grid_size(), PanelSize { w: 12, h: 1 });
    }
    let older = json!({"id": "cpu", "title": "CPU", "metric": "cpu", "aggregation": "avg", "filters": []});
    let spec: PanelSpec = serde_json::from_value(older.clone()).unwrap();
    assert_eq!(spec.kind, PanelKind::TimeSeries);
    assert_eq!(serde_json::to_value(&spec).unwrap(), older, "no new keys on an older panel");
    let bare: PanelSpec = serde_json::from_value(json!({"id": "t", "title": "T", "kind": "text", "text": "hi"})).unwrap();
    assert_eq!(bare.aggregation, PanelAggregation::Last);
    bare.validate().unwrap();
}

/// What each kind needs, and what it must not carry, is refused with the
/// rule's name.
#[test]
fn each_panel_kind_is_validated_for_what_it_needs() {
    let base = PanelSpec {
        id: "p".into(),
        title: "P".into(),
        metric: "m".into(),
        group_by: Some(PanelGroupBy::Entity),
        ..PanelSpec::default()
    };
    let refused = |change: &dyn Fn(&mut PanelSpec), rule: &str| {
        let mut spec = base.clone();
        change(&mut spec);
        match spec.validate() {
            Err(DashboardError::Invalid(message)) => assert!(message.contains(rule), "{message} lacks {rule}"),
            Ok(()) => panic!("accepted a panel breaking {rule}: {spec:?}"),
        }
    };
    refused(&|spec| spec.size = Some(PanelSize { w: 13, h: 2 }), "panel size");
    refused(&|spec| spec.size = Some(PanelSize { w: 3, h: 0 }), "panel size");
    refused(&|spec| spec.kind = PanelKind::Text, "panel metric");
    refused(&|spec| { spec.kind = PanelKind::Text; spec.metric.clear(); }, "text panel needs");
    refused(&|spec| spec.text = Some("x".into()), "panel text belongs");
    refused(&|spec| { spec.kind = PanelKind::Donut; spec.group_by = None; }, "need group_by");
    refused(&|spec| { spec.kind = PanelKind::Donut; spec.aggregation = PanelAggregation::Avg; }, "additive");
    refused(&|spec| spec.columns = vec![PanelColumn { title: "c".into(), metric: "m2".into(), aggregation: PanelAggregation::Last, filters: Vec::new() }], "columns belong");
    refused(&|spec| { spec.kind = PanelKind::Table; spec.columns = vec![PanelColumn { title: " ".into(), metric: "m2".into(), aggregation: PanelAggregation::Last, filters: Vec::new() }]; }, "column title");
    refused(&|spec| spec.stream = Some(PanelStream::default()), "stream belongs");
    refused(&|spec| { spec.kind = PanelKind::Logs; spec.metric.clear(); spec.stream = Some(PanelStream { limit: Some(0), ..PanelStream::default() }); }, "stream limit");
    refused(&|spec| spec.metric.clear(), "panel metric");
    // WP incident-visuals: events, trace and query panels.
    refused(&|spec| { spec.kind = PanelKind::Events; }, "panel metric");
    refused(&|spec| { spec.kind = PanelKind::Trace; spec.metric.clear(); }, "trace panel needs trace_id");
    refused(&|spec| { spec.kind = PanelKind::Trace; spec.metric.clear(); spec.trace_id = Some("xyz".into()); }, "trace panel needs trace_id");
    refused(&|spec| spec.trace_id = Some("0af7651916cd43dd8448eb211c80319c".into()), "trace_id belongs");
    refused(&|spec| { spec.kind = PanelKind::Query; spec.metric.clear(); }, "query panel needs");
    let query = |tool: &str, arguments: Value| PanelQuery { tool: tool.into(), arguments: arguments.as_object().unwrap().clone() };
    refused(&|spec| { spec.kind = PanelKind::Query; spec.metric.clear(); spec.query = Some(query("follow_work", json!({}))); }, "tool is query or search");
    refused(&|spec| { spec.kind = PanelKind::Query; spec.metric.clear(); spec.query = Some(query("query", json!({"x": "y".repeat(5000)}))); }, "at most 4096 bytes");
    refused(&|spec| spec.query = Some(query("query", json!({}))), "query belongs");
    let mut events = base.clone();
    events.kind = PanelKind::Events;
    events.metric.clear();
    events.group_by = None;
    events.stream = Some(PanelStream::default());
    events.validate().unwrap();
    let mut stat = base.clone();
    stat.kind = PanelKind::Stat;
    stat.group_by = None;
    stat.validate().unwrap();
    let mut definition = full_definition();
    definition.blocks[0].width = Some(0);
    assert_eq!(definition.validate(), Err(DashboardDefinitionError::BlockWidth { index: 0 }));
    definition.blocks[0].width = Some(12);
    definition.validate().unwrap();
}
