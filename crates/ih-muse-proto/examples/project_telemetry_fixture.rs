use std::env;
use std::fs;

use ih_muse_proto::TelemetryEnvelope;

fn main() {
    let mut arguments = env::args().skip(1);
    let input = arguments.next().expect("input fixture path");
    let output = arguments.next().expect("output projection path");
    assert!(arguments.next().is_none(), "expected exactly two paths");
    let envelope: TelemetryEnvelope =
        serde_json::from_slice(&fs::read(input).expect("read fixture")).expect("parse fixture");
    let projection = envelope.persistence_projection().expect("project fixture");
    let encoded = serde_json::to_string_pretty(&projection).expect("encode projection") + "\n";
    fs::write(output, encoded).expect("write projection");
}
