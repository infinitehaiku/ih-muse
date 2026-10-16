//! OTLP logs, protobuf-encoded by hand: the few messages of
//! `opentelemetry/proto/collector/logs/v1` an `ExportLogsServiceRequest`
//! needs (resource, scope, log records, string and integer attributes).
//! The Poet's OTLP intake accepts protobuf only; a hand encoder of these
//! fields avoids a code generator and keeps the Muse's dependencies small.

/// One attribute value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Text(String),
    Int(i64),
    Bool(bool),
}

/// A key and its value.
pub type Attribute = (String, Value);

/// One log record of a resource.
#[derive(Clone, Debug, PartialEq)]
pub struct LogRecord {
    pub time_unix_nano: u64,
    pub observed_time_unix_nano: u64,
    /// OpenTelemetry severity number (0 = unspecified).
    pub severity_number: i32,
    pub severity_text: &'static str,
    pub body: String,
    pub attributes: Vec<Attribute>,
}

/// The records of one resource (one container's log file).
#[derive(Clone, Debug, PartialEq)]
pub struct ResourceLogs {
    pub resource: Vec<Attribute>,
    pub records: Vec<LogRecord>,
}

fn varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn tag(out: &mut Vec<u8>, field: u32, wire: u8) {
    varint(out, (u64::from(field) << 3) | u64::from(wire));
}

fn bytes_field(out: &mut Vec<u8>, field: u32, bytes: &[u8]) {
    tag(out, field, 2);
    varint(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

fn message(out: &mut Vec<u8>, field: u32, build: impl FnOnce(&mut Vec<u8>)) {
    let mut inner = Vec::new();
    build(&mut inner);
    bytes_field(out, field, &inner);
}

fn fixed64(out: &mut Vec<u8>, field: u32, value: u64) {
    tag(out, field, 1);
    out.extend_from_slice(&value.to_le_bytes());
}

fn any_value(out: &mut Vec<u8>, value: &Value) {
    match value {
        Value::Text(text) => bytes_field(out, 1, text.as_bytes()),
        Value::Bool(flag) => {
            tag(out, 2, 0);
            varint(out, u64::from(*flag));
        }
        Value::Int(number) => {
            tag(out, 3, 0);
            varint(out, *number as u64);
        }
    }
}

fn key_values(out: &mut Vec<u8>, field: u32, attributes: &[Attribute]) {
    for (key, value) in attributes {
        message(out, field, |kv| {
            bytes_field(kv, 1, key.as_bytes());
            message(kv, 2, |any| any_value(any, value));
        });
    }
}

/// An `ExportLogsServiceRequest` of `resources`, scope `scope`/`version`.
pub fn export_logs_request(resources: &[ResourceLogs], scope: &str, version: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for resource in resources {
        // ResourceLogs = 1 { Resource resource = 1; ScopeLogs scope_logs = 2 }
        message(&mut out, 1, |rl| {
            message(rl, 1, |r| key_values(r, 1, &resource.resource));
            message(rl, 2, |sl| {
                message(sl, 1, |s| {
                    bytes_field(s, 1, scope.as_bytes());
                    bytes_field(s, 2, version.as_bytes());
                });
                for record in &resource.records {
                    message(sl, 2, |lr| {
                        fixed64(lr, 1, record.time_unix_nano);
                        if record.severity_number != 0 {
                            tag(lr, 2, 0);
                            varint(lr, record.severity_number as u64);
                            bytes_field(lr, 3, record.severity_text.as_bytes());
                        }
                        message(lr, 5, |any| {
                            any_value(any, &Value::Text(record.body.clone()))
                        });
                        key_values(lr, 6, &record.attributes);
                        fixed64(lr, 11, record.observed_time_unix_nano);
                    });
                }
            });
        });
    }
    out
}

/// What a Poet's `ExportLogsServiceResponse` says it refused: the number
/// of records and its message (`partial_success`); zero for a full success
/// or a body this reader does not understand.
pub fn rejected_records(response: &[u8]) -> (i64, String) {
    fn varint(bytes: &[u8], at: &mut usize) -> Option<u64> {
        let mut value = 0_u64;
        for shift in (0..64).step_by(7) {
            let byte = *bytes.get(*at)?;
            *at += 1;
            value |= u64::from(byte & 0x7f) << shift;
            if byte < 0x80 {
                return Some(value);
            }
        }
        None
    }
    /// The fields of one message: (number, varint or bytes).
    fn fields(bytes: &[u8]) -> Vec<(u64, u64, &[u8])> {
        let mut out = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            let Some(key) = varint(bytes, &mut at) else {
                break;
            };
            match key & 7 {
                0 => match varint(bytes, &mut at) {
                    Some(value) => out.push((key >> 3, value, &bytes[0..0])),
                    None => break,
                },
                2 => {
                    let Some(len) = varint(bytes, &mut at) else {
                        break;
                    };
                    let Some(end) = at
                        .checked_add(len as usize)
                        .filter(|end| *end <= bytes.len())
                    else {
                        break;
                    };
                    out.push((key >> 3, 0, &bytes[at..end]));
                    at = end;
                }
                1 => at += 8,
                5 => at += 4,
                _ => break,
            }
        }
        out
    }
    let mut rejected = 0;
    let mut message = String::new();
    for (number, _, partial) in fields(response) {
        if number != 1 {
            continue;
        }
        for (field, value, text) in fields(partial) {
            match field {
                1 => rejected = value as i64,
                2 => message = String::from_utf8_lossy(text).into_owned(),
                _ => {}
            }
        }
    }
    (rejected, message)
}

#[cfg(test)]
pub(crate) mod decode {
    //! A protobuf reader for tests: the encoder's output read back field
    //! by field, independent of the encoder's code.

    /// A decoded field: number and value.
    #[derive(Debug, Clone, PartialEq)]
    pub enum Field {
        Varint(u64),
        Fixed64(u64),
        Bytes(Vec<u8>),
    }

    fn varint(bytes: &[u8], at: &mut usize) -> u64 {
        let mut value = 0_u64;
        let mut shift = 0;
        loop {
            let byte = bytes[*at];
            *at += 1;
            value |= u64::from(byte & 0x7f) << shift;
            if byte < 0x80 {
                return value;
            }
            shift += 7;
        }
    }

    /// Every field of one message, in order.
    pub fn fields(bytes: &[u8]) -> Vec<(u32, Field)> {
        let mut at = 0;
        let mut out = Vec::new();
        while at < bytes.len() {
            let key = varint(bytes, &mut at);
            let (number, wire) = ((key >> 3) as u32, key & 7);
            let field = match wire {
                0 => Field::Varint(varint(bytes, &mut at)),
                1 => {
                    let value = u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
                    at += 8;
                    Field::Fixed64(value)
                }
                2 => {
                    let len = varint(bytes, &mut at) as usize;
                    let value = bytes[at..at + len].to_vec();
                    at += len;
                    Field::Bytes(value)
                }
                other => panic!("wire type {other}"),
            };
            out.push((number, field));
        }
        out
    }

    /// The bytes of every field `number`.
    pub fn all(bytes: &[u8], number: u32) -> Vec<Vec<u8>> {
        fields(bytes)
            .into_iter()
            .filter_map(|(n, f)| match (n == number, f) {
                (true, Field::Bytes(b)) => Some(b),
                _ => None,
            })
            .collect()
    }

    pub fn one(bytes: &[u8], number: u32) -> Vec<u8> {
        all(bytes, number).into_iter().next().expect("field")
    }

    pub fn scalar(bytes: &[u8], number: u32) -> Option<u64> {
        fields(bytes)
            .into_iter()
            .find_map(|(n, f)| match (n == number, f) {
                (true, Field::Varint(v) | Field::Fixed64(v)) => Some(v),
                _ => None,
            })
    }

    /// `KeyValue`s of a message's field `number` as (key, string or int).
    pub fn attributes(bytes: &[u8], number: u32) -> Vec<(String, String)> {
        all(bytes, number)
            .iter()
            .map(|kv| {
                let key = String::from_utf8(one(kv, 1)).unwrap();
                let any = one(kv, 2);
                let value = match fields(&any).into_iter().next().unwrap() {
                    (1, Field::Bytes(text)) => String::from_utf8(text).unwrap(),
                    (_, Field::Varint(v)) => v.to_string(),
                    other => panic!("{other:?}"),
                };
                (key, value)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::decode::{all, attributes, one, scalar};
    use super::*;

    #[test]
    fn encodes_the_otlp_logs_fields_the_intake_reads() {
        let request = export_logs_request(
            &[ResourceLogs {
                resource: vec![
                    ("service.name".into(), Value::Text("shop".into())),
                    ("k8s.container.restart_count".into(), Value::Int(3)),
                ],
                records: vec![
                    LogRecord {
                        time_unix_nano: 1_760_601_060_123_456_789,
                        observed_time_unix_nano: 1_760_601_061_000_000_000,
                        severity_number: 17,
                        severity_text: "ERROR",
                        body: "ERROR é broke".into(),
                        attributes: vec![("log.iostream".into(), Value::Text("stderr".into()))],
                    },
                    LogRecord {
                        time_unix_nano: 5,
                        observed_time_unix_nano: 6,
                        severity_number: 0,
                        severity_text: "",
                        body: "x".repeat(300),
                        attributes: vec![],
                    },
                ],
            }],
            "ih.pod.logs",
            "1.0.0",
        );
        let resource_logs = all(&request, 1);
        assert_eq!(resource_logs.len(), 1);
        let resource = one(&resource_logs[0], 1);
        assert_eq!(
            attributes(&resource, 1),
            [
                ("service.name".to_string(), "shop".to_string()),
                ("k8s.container.restart_count".to_string(), "3".to_string())
            ]
        );
        let scope_logs = one(&resource_logs[0], 2);
        let scope = one(&scope_logs, 1);
        assert_eq!(one(&scope, 1), b"ih.pod.logs");
        let records = all(&scope_logs, 2);
        assert_eq!(records.len(), 2);
        assert_eq!(scalar(&records[0], 1), Some(1_760_601_060_123_456_789));
        assert_eq!(scalar(&records[0], 2), Some(17));
        assert_eq!(one(&records[0], 3), b"ERROR");
        assert_eq!(one(&one(&records[0], 5), 1), "ERROR é broke".as_bytes());
        assert_eq!(
            attributes(&records[0], 6),
            [("log.iostream".to_string(), "stderr".to_string())]
        );
        assert_eq!(scalar(&records[0], 11), Some(1_760_601_061_000_000_000));
        assert_eq!(
            scalar(&records[1], 2),
            None,
            "unspecified severity is omitted"
        );
        assert_eq!(one(&one(&records[1], 5), 1).len(), 300);
    }

    #[test]
    fn reads_the_rejected_count_of_a_partial_success() {
        // partial_success { rejected_log_records: 2, error_message: "no time" }
        let mut partial = vec![0x08, 0x02, 0x12, 0x07];
        partial.extend_from_slice(b"no time");
        let mut response = vec![0x0a, partial.len() as u8];
        response.extend_from_slice(&partial);
        assert_eq!(rejected_records(&response), (2, "no time".into()));
        assert_eq!(rejected_records(b""), (0, String::new()));
        assert_eq!(
            rejected_records(&[0x0a, 0x50, 0x01]),
            (0, String::new()),
            "cut short"
        );
    }
}
