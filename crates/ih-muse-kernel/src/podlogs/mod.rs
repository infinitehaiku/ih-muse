//! Pod and service logs from the node: the node Muse tails kubelet's
//! `/var/log/pods` (the host's `/var/log`, mounted read-only) and sends
//! each container's lines to the Poets as OTLP logs, so a client deploys
//! nothing new to see its pods' logs (decisions.md, "Parity matrix
//! answers", item 6).
//!
//! Each container log file is one OTLP resource named with the identity
//! keys the Kubernetes Muse uses for the same pod and container
//! (`k8s.cluster.uid`, `k8s.pod.uid`, `k8s.container.name`, `container.id`
//! from kubelet's `/var/log/containers` links), plus the names people read
//! (`k8s.namespace.name`, `k8s.pod.name`, `k8s.node.name`) and a
//! `service.name`: the pod's workload, read from its generated name.
//!
//! Delivery is at least once and bounded: one pass reads at most
//! [`PodLogsConfig::max_batch_bytes`] of new lines into one export; the
//! next pass reads only after a Poet acknowledged it ([`PodLogs::commit`]),
//! so while the Poets are busy (429 `Retry-After`) or away, the unread
//! lines wait in the files, not in memory. The acknowledged offsets are
//! saved ([`files::Checkpoint`]); a restart resumes there. A line resent
//! after a crash between acknowledgement and save is stored once (the
//! Poet deduplicates records by content).

pub mod files;
pub mod otlp;
pub mod parse;

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use files::{Checkpoint, FileKey, LogFile, Mark, Pod, FINGERPRINT_BYTES};
use otlp::{Attribute, LogRecord, ResourceLogs, Value};

/// Instrumentation scope of the records.
pub const SCOPE_NAME: &str = "ih.pod.logs";

/// Which pods' logs are sent: shell-style patterns (`*`, `?`). An empty
/// include list includes everything; an exclusion wins over an inclusion.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filter {
    pub include_namespaces: Vec<String>,
    pub exclude_namespaces: Vec<String>,
    /// `<namespace>/<pod>` patterns.
    pub include_pods: Vec<String>,
    pub exclude_pods: Vec<String>,
}

/// `pattern` with `*` (any run) and `?` (one character) matches `text`.
pub fn glob(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let (mut pi, mut ti) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((star_p, star_t)) = star {
            pi = star_p + 1;
            ti = star_t + 1;
            star = Some((star_p, star_t + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|c| *c == '*')
}

impl Filter {
    pub fn admits(&self, pod: &Pod) -> bool {
        let any = |patterns: &[String], text: &str| patterns.iter().any(|p| glob(p, text));
        let qualified = format!("{}/{}", pod.namespace, pod.name);
        let included = (self.include_namespaces.is_empty() && self.include_pods.is_empty())
            || any(&self.include_namespaces, &pod.namespace)
            || any(&self.include_pods, &qualified);
        included
            && !any(&self.exclude_namespaces, &pod.namespace)
            && !any(&self.exclude_pods, &qualified)
    }
}

/// Where the tailer reads, who it speaks for and its bounds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PodLogsConfig {
    /// kubelet's `/var/log/pods` as mounted in the container.
    pub pods_root: PathBuf,
    /// kubelet's `/var/log/containers` (container ids) as mounted.
    pub containers_root: PathBuf,
    pub node_name: String,
    /// The cluster root's identity, as the Kubernetes Muse sends it.
    pub cluster_uid: String,
    pub cluster_name: Option<String>,
    pub filter: Filter,
    /// On a first start (no checkpoint), files already there are read from
    /// their end (`true`) or their start.
    pub start_at_end: bool,
    /// New log bytes read per pass (one export).
    pub max_batch_bytes: usize,
    /// New bytes read from one file per pass; also the longest line kept
    /// whole before it is cut.
    pub max_file_bytes: usize,
}

impl PodLogsConfig {
    /// Settings for `pods_root` with the defaults: from the end on a first
    /// start, 1 MiB per export, 256 KiB per file.
    pub fn new(pods_root: PathBuf, node_name: String, cluster_uid: String) -> Self {
        let containers_root = pods_root.parent().map_or_else(
            || PathBuf::from("/var/log/containers"),
            |p| p.join("containers"),
        );
        Self {
            pods_root,
            containers_root,
            node_name,
            cluster_uid,
            cluster_name: None,
            filter: Filter::default(),
            start_at_end: true,
            max_batch_bytes: 1024 * 1024,
            max_file_bytes: 256 * 1024,
        }
    }
}

/// Totals since start, for the Muse's own log lines.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PodLogStats {
    /// Files followed at the last pass.
    pub files: u64,
    pub bytes_read: u64,
    pub records_sent: u64,
    /// Lines neither format reads (sent whole).
    pub unparsed_lines: u64,
    /// Files truncated in place or replaced under a reused inode: read
    /// again from their start.
    pub restarts: u64,
    /// Bytes of files deleted before they were read (rotated away while
    /// the Poets did not take more).
    pub bytes_lost: u64,
    /// Exports a Poet refused as invalid; their lines are skipped.
    pub exports_refused: u64,
}

/// One export, and the offsets to save once a Poet acknowledged it.
#[derive(Clone, Debug)]
pub struct Batch {
    /// `ExportLogsServiceRequest`, protobuf.
    pub export: Vec<u8>,
    pub records: usize,
    pub bytes_read: u64,
    advances: Vec<(FileKey, u64)>,
}

/// The tailer between passes: the files followed and their acknowledged
/// offsets.
pub struct PodLogs {
    config: PodLogsConfig,
    marks: HashMap<FileKey, Mark>,
    /// No checkpoint was found: files of the first scan start per
    /// [`PodLogsConfig::start_at_end`].
    first_scan: bool,
    /// The file the next pass starts with (fairness under the budget).
    turn: usize,
    stats: PodLogStats,
}

/// `pod`'s workload: its generated suffixes taken off
/// (`<deployment>-<hash>-<id>`, `<daemonset or job>-<id>`,
/// `<statefulset>-<ordinal>`), else its own name.
pub fn workload_of(pod: &str) -> &str {
    let generated = |part: &str, lengths: std::ops::RangeInclusive<usize>| {
        lengths.contains(&part.len())
            && part
                .bytes()
                .all(|b| b"bcdfghjklmnpqrstvwxz2456789".contains(&b))
    };
    if let Some((rest, id)) = pod.rsplit_once('-') {
        if generated(id, 5..=5) {
            if let Some((base, hash)) = rest.rsplit_once('-') {
                if !base.is_empty() && generated(hash, 6..=10) {
                    return base;
                }
            }
            if !rest.is_empty() {
                return rest;
            }
        }
        if !rest.is_empty() && !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) {
            return rest;
        }
    }
    pod
}

fn read_range(path: &Path, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::with_capacity(len);
    file.take(len as u64).read_to_end(&mut bytes)?;
    Ok(bytes)
}

impl PodLogs {
    /// A tailer resuming from `checkpoint` (`None`: a first start).
    pub fn new(config: PodLogsConfig, checkpoint: Option<Checkpoint>) -> Self {
        Self {
            first_scan: checkpoint.is_none(),
            marks: checkpoint.map(|c| c.marks()).unwrap_or_default(),
            config,
            turn: 0,
            stats: PodLogStats::default(),
        }
    }

    pub fn config(&self) -> &PodLogsConfig {
        &self.config
    }

    pub fn stats(&self) -> &PodLogStats {
        &self.stats
    }

    /// The acknowledged offsets, to save.
    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint::from_marks(&self.marks)
    }

    /// The mark of `file`: kept, restarted (truncated or another file under
    /// the same inode) or new.
    fn mark_for(&mut self, file: &LogFile) -> Mark {
        if let Some(mark) = self.marks.get(&file.key) {
            // Unchanged since the last pass, its start already known.
            if mark.fingerprint_len == FINGERPRINT_BYTES && mark.seen_len == file.len {
                let mut kept = mark.clone();
                kept.path = file.path.display().to_string();
                return kept;
            }
        }
        let prefix_len = file.len.min(FINGERPRINT_BYTES);
        let prefix = read_range(&file.path, 0, prefix_len as usize).unwrap_or_default();
        let fresh = |offset: u64| Mark {
            offset,
            seen_len: file.len,
            fingerprint: files::fnv1a(&prefix),
            fingerprint_len: prefix.len() as u64,
            path: file.path.display().to_string(),
        };
        match self.marks.get(&file.key) {
            Some(mark) => {
                let same_start = (prefix.len() as u64) >= mark.fingerprint_len
                    && files::fnv1a(&prefix[..mark.fingerprint_len as usize]) == mark.fingerprint;
                if !same_start || file.len < mark.offset {
                    self.stats.restarts += 1;
                    return fresh(0);
                }
                // Its fingerprint grows with the file up to its full length.
                fresh(mark.offset)
            }
            None if self.first_scan && self.config.start_at_end => fresh(file.len),
            None => fresh(0),
        }
    }

    fn resource(
        &self,
        file: &LogFile,
        ids: &HashMap<(String, String, u32), String>,
    ) -> Vec<Attribute> {
        let text = |key: &str, value: &str| (key.to_string(), Value::Text(value.to_string()));
        let mut resource = vec![
            text("service.name", workload_of(&file.pod.name)),
            text("k8s.cluster.uid", &self.config.cluster_uid),
            text("k8s.namespace.name", &file.pod.namespace),
            text("k8s.pod.name", &file.pod.name),
            text("k8s.pod.uid", &file.pod.uid),
            text("k8s.container.name", &file.container),
            (
                "k8s.container.restart_count".to_string(),
                Value::Int(i64::from(file.restart)),
            ),
            text("k8s.node.name", &self.config.node_name),
            text("host.name", &self.config.node_name),
        ];
        if let Some(name) = &self.config.cluster_name {
            resource.push(text("k8s.cluster.name", name));
        }
        if let Some(id) = ids.get(&(file.pod.uid.clone(), file.container.clone(), file.restart)) {
            resource.push(text("container.id", id));
        }
        resource
    }

    /// Scans the files and reads their new complete records into one
    /// export; `None` when nothing new. Offsets advance only on
    /// [`Self::commit`].
    pub fn poll(&mut self, now_unix_nano: u64) -> Option<Batch> {
        let scanned: Vec<LogFile> = files::scan(&self.config.pods_root)
            .into_iter()
            .filter(|file| self.config.filter.admits(&file.pod))
            .collect();
        let mut marks = HashMap::with_capacity(scanned.len());
        for file in &scanned {
            let mark = self.mark_for(file);
            marks.insert(file.key, mark);
        }
        // Gone: deleted by kubelet (rotation, pod removal).
        for (key, mark) in &self.marks {
            if !marks.contains_key(key) {
                self.stats.bytes_lost += mark.seen_len.saturating_sub(mark.offset);
            }
        }
        self.marks = marks;
        self.first_scan = false;
        self.stats.files = scanned.len() as u64;

        let ids = files::container_ids(&self.config.containers_root);
        let mut budget = self.config.max_batch_bytes;
        let mut resources = Vec::new();
        let mut advances = Vec::new();
        let (mut records, mut bytes_read) = (0, 0_u64);
        let count = scanned.len();
        let first = if count == 0 { 0 } else { self.turn % count };
        // The first file the budget cut short starts the next pass.
        let mut starved = None;
        for index in (0..count).map(|i| (first + i) % count) {
            let file = &scanned[index];
            let offset = self.marks[&file.key].offset;
            if file.len <= offset {
                continue;
            }
            let share = ((file.len - offset) as usize).min(self.config.max_file_bytes);
            if budget < share {
                starved.get_or_insert(index);
            }
            if budget == 0 {
                break;
            }
            let want = share.min(budget);
            let Ok(bytes) = read_range(&file.path, offset, want) else {
                continue;
            };
            let at_end = file.rotated && file.quiet && offset + bytes.len() as u64 >= file.len;
            let assembled = parse::assemble(
                &bytes,
                offset,
                at_end,
                self.config.max_file_bytes,
                now_unix_nano,
            );
            if assembled.resume_offset == offset {
                continue;
            }
            let used = assembled.resume_offset - offset;
            budget = budget.saturating_sub(used as usize);
            bytes_read += used;
            self.stats.unparsed_lines += assembled.unparsed_lines;
            advances.push((file.key, assembled.resume_offset));
            if assembled.records.is_empty() {
                continue;
            }
            records += assembled.records.len();
            let file_name = file
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            resources.push(ResourceLogs {
                resource: self.resource(file, &ids),
                records: assembled
                    .records
                    .into_iter()
                    .map(|record| {
                        let (severity_number, severity_text) =
                            parse::severity(&record.body).unwrap_or((0, ""));
                        LogRecord {
                            time_unix_nano: record.time_unix_nano,
                            observed_time_unix_nano: now_unix_nano,
                            severity_number,
                            severity_text,
                            body: record.body,
                            attributes: vec![
                                (
                                    "log.iostream".into(),
                                    Value::Text(record.stream.as_str().into()),
                                ),
                                ("log.file.name".into(), Value::Text(file_name.clone())),
                            ],
                        }
                    })
                    .collect(),
            });
        }
        self.turn = starved.unwrap_or(first);
        if advances.is_empty() {
            return None;
        }
        self.stats.bytes_read += bytes_read;
        Some(Batch {
            export: otlp::export_logs_request(&resources, SCOPE_NAME, crate::VERSION),
            records,
            bytes_read,
            advances,
        })
    }

    /// A Poet acknowledged `batch` (or refused it as invalid: `refused`):
    /// its files are read past it from now on.
    pub fn commit(&mut self, batch: &Batch, refused: bool) {
        for (key, offset) in &batch.advances {
            if let Some(mark) = self.marks.get_mut(key) {
                mark.offset = mark.offset.max(*offset);
            }
        }
        if refused {
            self.stats.exports_refused += 1;
        } else {
            self.stats.records_sent += batch.records as u64;
        }
    }
}

#[cfg(test)]
mod tests;
