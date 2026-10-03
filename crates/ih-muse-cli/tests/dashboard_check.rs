//! `ih-muse-cli dashboard check` accepts the shipped example and rejects
//! invalid variants with a non-zero exit; `dashboard schema` prints the
//! committed schema.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A scratch directory removed on every outcome, including a failed assert.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn workspace_file(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

fn cli(args: &[&std::ffi::OsStr]) -> (bool, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_ih-muse-cli"))
        .args(args)
        .output()
        .unwrap();
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

#[test]
fn check_accepts_the_example_and_rejects_invalid_variants() {
    let example = workspace_file("examples/dashboards/macos-host.json");
    let (ok, out) = cli(&["dashboard".as_ref(), "check".as_ref(), example.as_os_str()]);
    assert!(ok, "{out}");
    assert!(out.starts_with("OK "), "{out}");
    assert!(
        out.contains("macos.host revision 1 (11 panels, 2 blocks)"),
        "{out}"
    );

    let scratch = Scratch(
        std::env::temp_dir().join(format!("ih-muse-dashboard-check-{}", std::process::id())),
    );
    std::fs::create_dir_all(&scratch.0).unwrap();
    let base: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&example).unwrap()).unwrap();
    type Variant = (&'static str, fn(&mut serde_json::Value), &'static str);
    let variants: [Variant; 4] = [
        (
            "unknown-block-panel",
            |v| v["blocks"][0]["panels"][0] = "gpu".into(),
            "unknown panel id gpu",
        ),
        (
            "bad-columns",
            |v| v["columns"] = 7.into(),
            "columns must be 2..=4",
        ),
        (
            "foreign-id",
            |v| v["id"] = "linux.host".into(),
            "id must be",
        ),
        (
            "unknown-field",
            |v| v["query"] = "SELECT 1".into(),
            "parse: unknown field `query`",
        ),
    ];
    let mut files = vec![example.clone()];
    for (name, change, _) in &variants {
        let mut value = base.clone();
        change(&mut value);
        let path = scratch.0.join(format!("{name}.json"));
        std::fs::write(&path, value.to_string()).unwrap();
        files.push(path);
    }
    let mut args: Vec<&std::ffi::OsStr> = vec!["dashboard".as_ref(), "check".as_ref()];
    args.extend(files.iter().map(|path| path.as_os_str()));
    let (ok, out) = cli(&args);
    assert!(!ok, "any invalid file fails the run: {out}");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 5, "{out}");
    assert!(lines[0].starts_with("OK "), "{out}");
    for ((name, _, message), line) in variants.iter().zip(&lines[1..]) {
        assert!(
            line.starts_with("ERROR ") && line.contains(name) && line.contains(message),
            "{line}"
        );
    }
    let (ok, out) = cli(&[
        "dashboard".as_ref(),
        "check".as_ref(),
        scratch.0.join("missing.json").as_os_str(),
    ]);
    assert!(!ok && out.contains("read:"), "{out}");
}

#[test]
fn schema_command_prints_the_committed_schema() {
    let (ok, out) = cli(&["dashboard".as_ref(), "schema".as_ref()]);
    assert!(ok);
    let committed =
        std::fs::read_to_string(workspace_file("schemas/dashboard-definition.schema.json"))
            .unwrap();
    assert_eq!(out, committed);
}
