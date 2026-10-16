//! The pod log files kubelet keeps on a node, and where each was read to.
//!
//! Layout (kubelet, every CRI runtime):
//!
//! ```text
//! /var/log/pods/<namespace>_<pod>_<pod-uid>/<container>/<restart>.log
//! /var/log/pods/…/<container>/<restart>.log.<YYYYMMDD-hhmmss>   rotated
//! /var/log/pods/…/<container>/<restart>.log.<…>.gz              old, skipped
//! /var/log/containers/<pod>_<namespace>_<container>-<id>.log    symlink
//! ```
//!
//! Kubelet rotates by renaming the live file and starting a new one, so a
//! file is followed by its identity (device and inode), not its name: the
//! rest of a renamed file is read under its new name, then the new file
//! from its start. A container restart starts `<restart+1>.log`. A file
//! whose size drops below the read offset was truncated and is read again
//! from its start; an inode reused by another file is told apart by a
//! fingerprint of its first bytes.

use std::collections::{BTreeMap, HashMap};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

/// Bytes of a file's start that tell one file from another (inode reuse).
pub const FINGERPRINT_BYTES: u64 = 128;

/// A pod, as its log directory names it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Pod {
    pub namespace: String,
    pub name: String,
    pub uid: String,
}

impl Pod {
    /// `<namespace>_<pod>_<uid>`; namespaces and pod names hold no `_`.
    pub fn from_dir_name(name: &str) -> Option<Self> {
        let mut parts = name.splitn(3, '_');
        let (namespace, pod, uid) = (parts.next()?, parts.next()?, parts.next()?);
        (!namespace.is_empty() && !pod.is_empty() && !uid.is_empty()).then(|| Self {
            namespace: namespace.into(),
            name: pod.into(),
            uid: uid.into(),
        })
    }
}

/// A file's identity on the node: device and inode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileKey {
    pub dev: u64,
    pub ino: u64,
}

impl FileKey {
    fn text(self) -> String {
        format!("{}:{}", self.dev, self.ino)
    }

    fn parse(text: &str) -> Option<Self> {
        let (dev, ino) = text.split_once(':')?;
        Some(Self {
            dev: dev.parse().ok()?,
            ino: ino.parse().ok()?,
        })
    }
}

/// One container log file found on a scan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogFile {
    pub path: PathBuf,
    pub key: FileKey,
    pub len: u64,
    pub pod: Pod,
    pub container: String,
    /// The container's restart number (the file's `<restart>.log`).
    pub restart: u32,
    /// Renamed by a rotation: it no longer grows.
    pub rotated: bool,
    /// Last written more than [`QUIET`] ago.
    pub quiet: bool,
}

/// A rotated file untouched this long is read to its very end (a last line
/// without its line end is sent as it is).
pub const QUIET: std::time::Duration = std::time::Duration::from_secs(10);

/// `<restart>.log` → (restart, false); `<restart>.log.<stamp>` → (restart,
/// true); compressed, temporary and other files → `None`.
fn log_file_name(name: &str) -> Option<(u32, bool)> {
    let (restart, rest) = name.split_once(".log")?;
    let restart = restart.parse().ok()?;
    match rest {
        "" => Some((restart, false)),
        stamp
            if stamp.starts_with('.')
                && stamp[1..]
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || byte == b'-') =>
        {
            Some((restart, true))
        }
        _ => None,
    }
}

/// Every container log file under `pods_root`, rotated ones before the
/// live one of each container, in path order.
pub fn scan(pods_root: &Path) -> Vec<LogFile> {
    let mut files = Vec::new();
    let now = SystemTime::now();
    let Ok(pods) = std::fs::read_dir(pods_root) else {
        return files;
    };
    for pod_dir in pods.flatten() {
        let Some(pod) = Pod::from_dir_name(&pod_dir.file_name().to_string_lossy()) else {
            continue;
        };
        let Ok(containers) = std::fs::read_dir(pod_dir.path()) else {
            continue;
        };
        for container_dir in containers.flatten() {
            let container = container_dir.file_name().to_string_lossy().into_owned();
            let Ok(entries) = std::fs::read_dir(container_dir.path()) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                let Some((restart, rotated)) = log_file_name(&name) else {
                    continue;
                };
                // The file itself, not a link to it.
                let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
                    continue;
                };
                if !meta.is_file() {
                    continue;
                }
                let quiet = meta
                    .modified()
                    .ok()
                    .and_then(|modified| now.duration_since(modified).ok())
                    .is_some_and(|age| age >= QUIET);
                files.push(LogFile {
                    path: entry.path(),
                    key: FileKey {
                        dev: meta.dev(),
                        ino: meta.ino(),
                    },
                    len: meta.len(),
                    pod: pod.clone(),
                    container: container.clone(),
                    restart,
                    rotated,
                    quiet,
                });
            }
        }
    }
    files.sort_by(|a, b| {
        (&a.pod, &a.container, a.restart, !a.rotated, &a.path).cmp(&(
            &b.pod,
            &b.container,
            b.restart,
            !b.rotated,
            &b.path,
        ))
    });
    files
}

/// Container ids from kubelet's `/var/log/containers` links:
/// (pod uid, container, restart) → id. A link
/// `<pod>_<namespace>_<container>-<id>.log` points at the container's
/// `<restart>.log` under `/var/log/pods`.
pub fn container_ids(containers_root: &Path) -> HashMap<(String, String, u32), String> {
    let mut ids = HashMap::new();
    let Ok(entries) = std::fs::read_dir(containers_root) else {
        return ids;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(id) = name
            .strip_suffix(".log")
            .and_then(|stem| stem.rsplit_once('-'))
            .map(|(_, id)| id.to_string())
            .filter(|id| id.len() >= 12 && id.bytes().all(|byte| byte.is_ascii_hexdigit()))
        else {
            continue;
        };
        let Ok(target) = std::fs::read_link(entry.path()) else {
            continue;
        };
        let parts: Vec<String> = target
            .iter()
            .rev()
            .take(3)
            .map(|part| part.to_string_lossy().into_owned())
            .collect();
        let [file, container, pod_dir] = parts.as_slice() else {
            continue;
        };
        let (Some(pod), Some((restart, false))) =
            (Pod::from_dir_name(pod_dir), log_file_name(file))
        else {
            continue;
        };
        ids.insert((pod.uid, container.clone(), restart), id);
    }
    ids
}

/// FNV-1a: stable across builds (it is saved in the checkpoint).
pub fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Where one file was read to, acknowledged by a Poet.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mark {
    /// Bytes sent and acknowledged (records end there).
    pub offset: u64,
    /// The file's size when last seen (to count what a deletion lost).
    pub seen_len: u64,
    /// [`fnv1a`] of the file's first `fingerprint_len` bytes.
    pub fingerprint: u64,
    pub fingerprint_len: u64,
    /// The path it was last seen under (for people reading the file).
    pub path: String,
}

/// The acknowledged read offsets of every file, saved so a restarted Muse
/// sends no line twice and skips none (on k-lab: a file in the pod's
/// memory volume, which survives container restarts).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkpoint {
    /// `<dev>:<ino>` → mark.
    pub files: BTreeMap<String, Mark>,
}

impl Checkpoint {
    /// The checkpoint in `path`; `None` when missing or unreadable (a
    /// first start).
    pub fn load(path: &Path) -> Option<Self> {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    }

    /// Writes through a temporary file and a rename.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let temporary = path.with_extension("tmp");
        std::fs::write(&temporary, serde_json::to_vec(self)?)?;
        std::fs::rename(temporary, path)
    }

    pub fn marks(&self) -> HashMap<FileKey, Mark> {
        self.files
            .iter()
            .filter_map(|(key, mark)| Some((FileKey::parse(key)?, mark.clone())))
            .collect()
    }

    pub fn from_marks(marks: &HashMap<FileKey, Mark>) -> Self {
        Self {
            files: marks
                .iter()
                .map(|(key, mark)| (key.text(), mark.clone()))
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pod_dirs_and_log_names_read_as_kubelet_writes_them() {
        assert_eq!(
            Pod::from_dir_name("ih-main_ih-next-poet-0_6f1c-77"),
            Some(Pod {
                namespace: "ih-main".into(),
                name: "ih-next-poet-0".into(),
                uid: "6f1c-77".into()
            })
        );
        assert_eq!(Pod::from_dir_name("no-underscores"), None);
        assert_eq!(log_file_name("0.log"), Some((0, false)));
        assert_eq!(log_file_name("3.log.20261016-075100"), Some((3, true)));
        assert_eq!(log_file_name("3.log.20261016-075100.gz"), None);
        assert_eq!(log_file_name("0.log.tmp"), None);
        assert_eq!(log_file_name("x.log"), None);
    }

    #[test]
    fn container_ids_come_from_the_containers_links() {
        let dir = tempfile::tempdir().unwrap();
        let pods = dir.path().join("pods/shop_web-1_uid-1/app");
        std::fs::create_dir_all(&pods).unwrap();
        std::fs::write(pods.join("2.log"), b"").unwrap();
        let containers = dir.path().join("containers");
        std::fs::create_dir_all(&containers).unwrap();
        let id = "ab".repeat(32);
        std::os::unix::fs::symlink(
            "/var/log/pods/shop_web-1_uid-1/app/2.log",
            containers.join(format!("web-1_shop_app-{id}.log")),
        )
        .unwrap();
        std::os::unix::fs::symlink("/nowhere", containers.join("bad_link-zz.log")).unwrap();
        let ids = container_ids(&containers);
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[&("uid-1".into(), "app".into(), 2)], id);
    }

    #[test]
    fn a_scan_lists_rotated_before_live_and_skips_compressed() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("shop_web-1_uid-1/app");
        std::fs::create_dir_all(&app).unwrap();
        for name in [
            "0.log",
            "0.log.20261016-075100",
            "0.log.20261015-000000.gz",
            "1.log",
        ] {
            std::fs::write(app.join(name), b"x").unwrap();
        }
        std::fs::create_dir_all(dir.path().join("not-a-pod/app")).unwrap();
        let files = scan(dir.path());
        let names: Vec<String> = files
            .iter()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["0.log.20261016-075100", "0.log", "1.log"]);
        assert!(files[0].rotated && !files[1].rotated);
        assert_eq!(files[2].restart, 1);
        assert_eq!(files[0].pod.namespace, "shop");
    }

    #[test]
    fn a_checkpoint_round_trips_and_a_missing_one_is_a_first_start() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pod-logs.json");
        assert_eq!(Checkpoint::load(&path), None);
        let mut marks = HashMap::new();
        marks.insert(
            FileKey { dev: 1, ino: 2 },
            Mark {
                offset: 10,
                seen_len: 12,
                fingerprint: 3,
                fingerprint_len: 10,
                path: "/p".into(),
            },
        );
        Checkpoint::from_marks(&marks).save(&path).unwrap();
        assert_eq!(Checkpoint::load(&path).unwrap().marks(), marks);
        std::fs::write(&path, b"{broken").unwrap();
        assert_eq!(Checkpoint::load(&path), None);
    }
}
