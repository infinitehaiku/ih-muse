//! The two line formats kubelet's container runtimes write under
//! `/var/log/pods`, and the assembly of lines into log records.
//!
//! - **CRI** (containerd, CRI-O): `<RFC 3339 time> <stream> <tag> <text>`,
//!   tag `F` (full line) or `P` (partial: the runtime split a long line;
//!   the parts up to the next `F` of the same stream form one line).
//! - **Docker JSON** (dockershim, cri-dockerd):
//!   `{"log":"<text>\n","stream":"stdout","time":"<RFC 3339>"}`; a `log`
//!   without its trailing newline is a partial part.
//!
//! Multi-line records are joined where it is cheap: a full line of the same
//! stream that starts with a space or a tab (the frames of a Java or Python
//! stack trace) continues the record before it, when both were read in the
//! same pass. A line neither format reads is kept whole as its own record
//! at the time it was read, and counted.

use chrono::DateTime;

/// The longest record body sent (bytes); longer bodies are cut at a
/// character boundary and marked.
pub const MAX_RECORD_BYTES: usize = 64 * 1024;
/// Appended to a body cut at [`MAX_RECORD_BYTES`].
pub const TRUNCATED_MARK: &str = "…[truncated]";

/// The output stream a line was written to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Stream {
    Stdout,
    Stderr,
    /// The line did not say (unreadable lines).
    Unknown,
}

impl Stream {
    fn parse(text: &str) -> Self {
        match text {
            "stdout" => Self::Stdout,
            "stderr" => Self::Stderr,
            _ => Self::Unknown,
        }
    }

    /// OpenTelemetry's `log.iostream` value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
            Self::Unknown => "unknown",
        }
    }
}

/// One line of a pod log file, decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// The runtime's time for the line (ns since the Unix epoch).
    pub time_unix_nano: u64,
    pub stream: Stream,
    /// The text without its line end.
    pub text: String,
    /// A part of a longer line (CRI `P`, Docker without `\n`).
    pub partial: bool,
}

fn rfc3339_nano(text: &str) -> Option<u64> {
    let time = DateTime::parse_from_rfc3339(text).ok()?;
    u64::try_from(time.timestamp_nanos_opt()?).ok()
}

/// Decodes one line (without its `\n`), CRI or Docker JSON. `None` when
/// neither format reads it.
pub fn parse_line(raw: &[u8]) -> Option<Line> {
    let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
    if raw.first() == Some(&b'{') {
        return parse_docker(raw);
    }
    let text = std::str::from_utf8(raw)
        .ok()
        .map_or_else(|| String::from_utf8_lossy(raw).into_owned(), str::to_string);
    let mut parts = text.splitn(4, ' ');
    let time = rfc3339_nano(parts.next()?)?;
    let stream = Stream::parse(parts.next()?);
    let tag = parts.next()?;
    // A tag is `F` or `P`, possibly with more flags after `:`.
    let partial = match tag.split(':').next()? {
        "F" => false,
        "P" => true,
        _ => return None,
    };
    Some(Line {
        time_unix_nano: time,
        stream,
        text: parts.next().unwrap_or("").to_string(),
        partial,
    })
}

fn parse_docker(raw: &[u8]) -> Option<Line> {
    #[derive(serde::Deserialize)]
    struct Docker {
        log: String,
        #[serde(default)]
        stream: String,
        time: String,
    }
    let docker: Docker = serde_json::from_slice(raw).ok()?;
    let time = rfc3339_nano(&docker.time)?;
    let (text, partial) = match docker.log.strip_suffix('\n') {
        Some(text) => (text.strip_suffix('\r').unwrap_or(text).to_string(), false),
        None => (docker.log, true),
    };
    Some(Line {
        time_unix_nano: time,
        stream: Stream::parse(&docker.stream),
        text,
        partial,
    })
}

/// A log record assembled from one or more lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    /// The time of its first line.
    pub time_unix_nano: u64,
    pub stream: Stream,
    pub body: String,
    /// The file offset just past its last line.
    pub end_offset: u64,
    /// The body was cut at [`MAX_RECORD_BYTES`].
    pub truncated: bool,
}

impl Record {
    fn push(&mut self, separator: &str, text: &str) {
        if self.truncated {
            return;
        }
        if self.body.len() + separator.len() + text.len() <= MAX_RECORD_BYTES {
            self.body.push_str(separator);
            self.body.push_str(text);
            return;
        }
        let room = MAX_RECORD_BYTES.saturating_sub(self.body.len() + separator.len());
        let mut cut = room.min(text.len());
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        self.body.push_str(separator);
        self.body.push_str(&text[..cut]);
        self.body.push_str(TRUNCATED_MARK);
        self.truncated = true;
    }
}

/// What one pass over a file's new bytes produced.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Assembled {
    pub records: Vec<Record>,
    /// Where the next pass starts: past the last line of a finished record,
    /// never inside a line or a partial record still being written.
    pub resume_offset: u64,
    /// Lines neither format reads (sent whole as their own records).
    pub unparsed_lines: u64,
}

/// A record whose parts are still arriving, with where its first part
/// starts (the next pass rereads from there if it is not finished).
struct Open {
    record: Record,
    start_offset: u64,
}

/// Assembles the records of `bytes`, read from `start_offset` of a file.
///
/// Only whole lines (ending in `\n`) are used, except when `at_end` says no
/// more bytes will come (a rotated file) or the bytes hold no line end at
/// all and fill `max_line` (a line longer than the read: cut and sent).
/// `read_at_unix_nano` dates lines no format reads.
pub fn assemble(
    bytes: &[u8],
    start_offset: u64,
    at_end: bool,
    max_line: usize,
    read_at_unix_nano: u64,
) -> Assembled {
    let mut out = Assembled {
        resume_offset: start_offset,
        ..Assembled::default()
    };
    // Records finished in this pass, by stream for multi-line joins.
    let mut done: Vec<Record> = Vec::new();
    let mut last_done_of: [Option<usize>; 3] = [None; 3];
    let mut open: [Option<Open>; 3] = [None, None, None];
    let slot = |stream: Stream| stream as usize;
    let mut position = 0_usize;
    while position < bytes.len() {
        let rest = &bytes[position..];
        let (line, next) = match rest.iter().position(|byte| *byte == b'\n') {
            Some(end) => (&rest[..end], position + end + 1),
            None if at_end || (position == 0 && rest.len() >= max_line) => (rest, bytes.len()),
            None => break,
        };
        let line_start = start_offset + position as u64;
        let line_end = start_offset + next as u64;
        position = next;
        let parsed = parse_line(line).unwrap_or_else(|| {
            out.unparsed_lines += 1;
            Line {
                time_unix_nano: read_at_unix_nano,
                stream: Stream::Unknown,
                text: String::from_utf8_lossy(line).into_owned(),
                partial: false,
            }
        });
        let index = slot(parsed.stream);
        let mut record = match open[index].take() {
            Some(mut pending) => {
                pending.record.push("", &parsed.text);
                pending.record.end_offset = line_end;
                if parsed.partial && !pending.record.truncated {
                    open[index] = Some(pending);
                    continue;
                }
                pending.record
            }
            None => {
                let mut record = Record {
                    time_unix_nano: parsed.time_unix_nano,
                    stream: parsed.stream,
                    body: String::new(),
                    end_offset: line_end,
                    truncated: false,
                };
                record.push("", &parsed.text);
                if parsed.partial && !record.truncated {
                    open[index] = Some(Open {
                        record,
                        start_offset: line_start,
                    });
                    continue;
                }
                record
            }
        };
        let continues = record.body.starts_with([' ', '\t']) && parsed.stream != Stream::Unknown;
        if let (true, Some(previous)) = (continues, last_done_of[index]) {
            let end = record.end_offset;
            let body = std::mem::take(&mut record.body);
            let target = &mut done[previous];
            target.push("\n", &body);
            target.end_offset = end;
            // A record joined out of order would hide the other stream's
            // lines between them from the resume offset: keep order by
            // moving it last.
            if previous + 1 != done.len() {
                let moved = done.remove(previous);
                done.push(moved);
                for last in last_done_of.iter_mut().flatten() {
                    if *last > previous {
                        *last -= 1;
                    }
                }
                last_done_of[index] = Some(done.len() - 1);
            }
            continue;
        }
        done.push(record);
        last_done_of[index] = Some(done.len() - 1);
    }
    // A pass ends before the first record still being written: its parts
    // are read again next pass.
    let cut = open
        .iter()
        .flatten()
        .map(|pending| pending.start_offset)
        .min();
    let read_to = start_offset + position as u64;
    out.resume_offset = match cut {
        Some(cut) if !at_end => cut,
        _ => read_to,
    };
    if at_end {
        // Nothing more comes: what is open is sent as it is.
        done.extend(open.into_iter().flatten().map(|pending| pending.record));
    }
    out.records = done
        .into_iter()
        .filter(|record| record.end_offset <= out.resume_offset)
        .collect();
    out.records.sort_by_key(|record| record.end_offset);
    out
}

/// The severity a line states in its first words, if any: OpenTelemetry
/// severity number and text. Cheap: a level word (`ERROR`, `WARN`, `INFO`,
/// `DEBUG`, `TRACE`, `FATAL`, any case) or `level=<word>` /
/// `"level":"<word>"` in the first 160 bytes.
pub fn severity(body: &str) -> Option<(i32, &'static str)> {
    let mut head = &body[..body.len().min(160)];
    while !body.is_char_boundary(head.len()) {
        head = &head[..head.len() - 1];
    }
    let words = head
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty());
    let mut level_next = false;
    for word in words {
        let level = match word.to_ascii_uppercase().as_str() {
            "TRACE" => Some((1, "TRACE")),
            "DEBUG" | "DBG" => Some((5, "DEBUG")),
            "INFO" | "INF" | "NOTICE" => Some((9, "INFO")),
            "WARN" | "WARNING" | "WRN" => Some((13, "WARN")),
            "ERROR" | "ERR" | "CRITICAL" => Some((17, "ERROR")),
            "FATAL" | "PANIC" | "EMERG" | "CRIT" => Some((21, "FATAL")),
            "LEVEL" | "SEVERITY" | "LVL" => {
                level_next = true;
                continue;
            }
            _ => None,
        };
        // Short level words like `ERR` or `INF` count only when uppercase
        // or after a level key: "info" in a sentence is not a level.
        let exact = word.len() > 3 && word.chars().all(|c| c.is_ascii_uppercase());
        if let Some(found) = level {
            if exact || level_next || word.len() > 4 && head.trim_start().starts_with(word) {
                return Some(found);
            }
        }
        level_next = false;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_760_600_000_000_000_000;

    fn cri(offset_ns: u64, stream: &str, tag: &str, text: &str) -> String {
        let time = DateTime::from_timestamp_nanos((T0 + offset_ns) as i64);
        format!(
            "{} {stream} {tag} {text}\n",
            time.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)
        )
    }

    fn docker(offset_ns: u64, stream: &str, log: &str) -> String {
        let time = DateTime::from_timestamp_nanos((T0 + offset_ns) as i64);
        let mut line = serde_json::to_string(&serde_json::json!({
            "log": log,
            "stream": stream,
            "time": time.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
        }))
        .unwrap();
        line.push('\n');
        line
    }

    fn bodies(assembled: &Assembled) -> Vec<&str> {
        assembled.records.iter().map(|r| r.body.as_str()).collect()
    }

    #[test]
    fn reads_cri_and_docker_lines_with_their_time_and_stream() {
        let line = parse_line(b"2026-10-16T07:51:00.123456789Z stderr F boom: x y").unwrap();
        assert_eq!(line.stream, Stream::Stderr);
        assert_eq!(line.text, "boom: x y");
        assert!(!line.partial);
        assert_eq!(line.time_unix_nano, 1_792_137_060_123_456_789);
        // An offset time and an empty text.
        let line = parse_line(b"2026-10-16T09:51:00+02:00 stdout F").unwrap();
        assert_eq!(line.time_unix_nano, 1_792_137_060_000_000_000);
        assert_eq!(line.text, "");
        let line = parse_line(b"2026-10-16T07:51:00Z stdout P part").unwrap();
        assert!(line.partial);
        let line =
            parse_line(br#"{"log":"hello\n","stream":"stdout","time":"2026-10-16T07:51:00.5Z"}"#)
                .unwrap();
        assert_eq!(
            (line.text.as_str(), line.stream, line.partial),
            ("hello", Stream::Stdout, false)
        );
        assert_eq!(line.time_unix_nano, 1_792_137_060_500_000_000);
        let line = parse_line(br#"{"log":"half","stream":"stderr","time":"2026-10-16T07:51:00Z"}"#)
            .unwrap();
        assert!(line.partial);
        for bad in [
            &b"not a log line"[..],
            b"2026-10-16T07:51:00Z stdout X tag",
            b"{\"log\":1}",
            b"",
        ] {
            assert_eq!(parse_line(bad), None, "{}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn joins_partial_parts_and_stack_frames_and_keeps_offsets_exact() {
        let text = [
            cri(1, "stdout", "P", "a long "),
            cri(2, "stderr", "F", "Traceback (most recent call last):"),
            cri(3, "stdout", "F", "line"),
            cri(4, "stderr", "F", "  File \"x.py\", line 1"),
            cri(5, "stderr", "F", "ValueError: bad"),
        ]
        .concat();
        let assembled = assemble(text.as_bytes(), 100, false, 1 << 20, 0);
        assert_eq!(
            bodies(&assembled),
            [
                "a long line",
                "Traceback (most recent call last):\n  File \"x.py\", line 1",
                "ValueError: bad"
            ]
        );
        assert_eq!(assembled.records[0].time_unix_nano, T0 + 1);
        assert_eq!(assembled.records[1].stream, Stream::Stderr);
        assert_eq!(assembled.resume_offset, 100 + text.len() as u64);
        assert_eq!(
            assembled.records.last().unwrap().end_offset,
            assembled.resume_offset
        );
    }

    #[test]
    fn a_pass_stops_before_an_unfinished_line_or_partial_record() {
        let done = cri(1, "stdout", "F", "one");
        let partial = cri(2, "stdout", "P", "two ");
        let text = format!("{done}{partial}2026-10-16T07:51");
        let assembled = assemble(text.as_bytes(), 0, false, 1 << 20, 0);
        assert_eq!(bodies(&assembled), ["one"]);
        assert_eq!(assembled.resume_offset, done.len() as u64);
        // The next pass rereads the partial part with its end.
        let rest = format!("{partial}{}", cri(3, "stdout", "F", "halves"));
        let next = assemble(rest.as_bytes(), done.len() as u64, false, 1 << 20, 0);
        assert_eq!(bodies(&next), ["two halves"]);
        assert_eq!(next.resume_offset, (done.len() + rest.len()) as u64);
        // A record of the other stream after an open partial waits too:
        // sending it now would send it twice.
        let text = format!("{partial}{}", cri(3, "stderr", "F", "other"));
        let waiting = assemble(text.as_bytes(), 0, false, 1 << 20, 0);
        assert!(waiting.records.is_empty());
        assert_eq!(waiting.resume_offset, 0);
        // At the end of a rotated file, what is open is sent, in file order.
        let last = assemble(text.as_bytes(), 0, true, 1 << 20, 0);
        assert_eq!(bodies(&last), ["two ", "other"]);
        assert_eq!(last.resume_offset, text.len() as u64);
    }

    #[test]
    fn docker_partials_and_frames_assemble_like_cri() {
        let text = [
            docker(1, "stdout", "par"),
            docker(2, "stdout", "tial\n"),
            docker(3, "stdout", "Exception in thread main\n"),
            docker(4, "stdout", "\tat Main.run(Main.java:1)\n"),
        ]
        .concat();
        let assembled = assemble(text.as_bytes(), 0, false, 1 << 20, 0);
        assert_eq!(
            bodies(&assembled),
            [
                "partial",
                "Exception in thread main\n\tat Main.run(Main.java:1)"
            ]
        );
    }

    #[test]
    fn unreadable_lines_are_kept_whole_and_counted() {
        let text = format!("garbage line\n{}", cri(1, "stdout", "F", "  indented"));
        let assembled = assemble(text.as_bytes(), 0, false, 1 << 20, 42);
        assert_eq!(bodies(&assembled), ["garbage line", "  indented"]);
        assert_eq!(assembled.records[0].time_unix_nano, 42);
        assert_eq!(assembled.records[0].stream, Stream::Unknown);
        assert_eq!(assembled.unparsed_lines, 1);
    }

    #[test]
    fn long_records_are_cut_and_marked_and_huge_lines_are_not_stuck() {
        let big = "é".repeat(MAX_RECORD_BYTES);
        let text = cri(1, "stdout", "F", &big);
        let assembled = assemble(text.as_bytes(), 0, false, 1 << 20, 0);
        let record = &assembled.records[0];
        assert!(record.truncated);
        assert!(record.body.ends_with(TRUNCATED_MARK));
        assert!(record.body.len() <= MAX_RECORD_BYTES + TRUNCATED_MARK.len());
        // A read full of bytes without a line end is cut and consumed.
        let endless = vec![b'x'; 1024];
        let assembled = assemble(&endless, 0, false, 1024, 7);
        assert_eq!(assembled.records.len(), 1);
        assert_eq!(assembled.resume_offset, 1024);
        // Fewer bytes than a read: wait for the line end.
        let assembled = assemble(&endless[..10], 0, false, 1024, 7);
        assert!(assembled.records.is_empty());
        assert_eq!(assembled.resume_offset, 0);
    }

    #[test]
    fn severity_comes_from_level_words_only() {
        assert_eq!(severity("ERROR something broke"), Some((17, "ERROR")));
        assert_eq!(
            severity("2026-10-16 07:51:00 WARN pool low"),
            Some((13, "WARN"))
        );
        assert_eq!(severity("ts=1 level=info msg=ok"), Some((9, "INFO")));
        assert_eq!(
            severity(r#"{"level":"error","msg":"x"}"#),
            Some((17, "ERROR"))
        );
        assert_eq!(
            severity("[INF] ready"),
            None,
            "short words need a level key"
        );
        assert_eq!(severity("lvl=dbg ready"), Some((5, "DEBUG")));
        assert_eq!(severity("Warning: disk almost full"), Some((13, "WARN")));
        assert_eq!(severity("fetched info about the error page"), None);
        assert_eq!(severity("plain line"), None);
    }
}
