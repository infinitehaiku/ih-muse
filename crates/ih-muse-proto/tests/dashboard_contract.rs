//! Dashboard definitions: validation, the GraphBatch wire field, and the
//! committed JSON Schema and example staying in step with the Rust types.

use ih_muse_proto::dashboard::{
    dashboard_definition_schema, AttributeRule, DashboardError, FilterOp, GoldenSignal,
    PanelAggregation, PanelFilter, PanelGroupBy, PanelSpec, PanelThresholds, ProfileRecognition,
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
        panels: vec![
            panel("a", PanelGroupBy::Entity),
            panel("b", PanelGroupBy::Attribute("db".into())),
        ],
        blocks: vec![DashboardBlock {
            label: "Queries".into(),
            text: Some("**Slow** first.".into()),
            panels: vec!["a".into(), "b".into()],
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

fn serialized_keys(value: &Value, keys: &mut Vec<String>) {
    match value {
        Value::Object(object) => object.iter().for_each(|(key, item)| {
            keys.push(key.clone());
            serialized_keys(item, keys);
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
/// once: in the implicit Measurements row (it has a golden signal) or in a
/// block.
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
        for panel in &definition.panels {
            let placed = definition
                .blocks
                .iter()
                .any(|block| block.panels.contains(&panel.id));
            assert_eq!(placed, panel.signal.is_none(), "{path:?}: {}", panel.id);
        }
        ids.push(definition.id);
    }
    ids.sort();
    assert_eq!(ids, ["k8s.cluster", "macos.host"]);
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
                source: DashboardError::Invalid("panel top_n must be 1..=20".into()),
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
        (|d| d.blocks = vec![d.blocks[0].clone(); 13], E::BlockCount),
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
