//! The kernel's lines from the host's systemd journal files, read directly.
//!
//! The journal is read from its files (`/var/log/journal/<machine-id>/` and
//! `/run/log/journal/<machine-id>/`, mounted read-only), not from
//! `/dev/kmsg`: a container may only open `/dev/kmsg` with the device in
//! its cgroup's allow list, which needs a privileged pod. Reading the files
//! needs only their group (`systemd-journal`).
//!
//! Only the parts of the format this Muse needs are read (systemd's
//! `JOURNAL_FILE_FORMAT.md`): the header, the field hash table to find the
//! `_TRANSPORT=kernel` data object, its entry list (the kernel's entries
//! only), and each entry's `MESSAGE`. Regular and compact files (systemd
//! 252+) are both read. Every offset is bounds-checked and every chain
//! walk bounded, so a file being written or a corrupt file ends that
//! file's read with a count, never a panic. A compressed `MESSAGE` (only
//! above 512 bytes by default) is skipped and counted.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};

const SIGNATURE: &[u8; 8] = b"LPKSHHRH";
const INCOMPATIBLE_COMPACT: u32 = 1 << 4;
/// Header `state` of a file journald closed for good (rotated).
const STATE_ARCHIVED: u8 = 2;
const OBJECT_DATA: u8 = 1;
const OBJECT_FIELD: u8 = 2;
const OBJECT_ENTRY: u8 = 3;
const OBJECT_ENTRY_ARRAY: u8 = 6;
/// Object flags of a compressed payload (XZ, LZ4, ZSTD).
const OBJECT_COMPRESSED: u8 = 1 | 2 | 4;
/// The longest chain or list walked in one file: a cycle in a corrupt file
/// ends the walk.
const MAX_STEPS: usize = 4_000_000;
/// The longest payload read (a kernel line is at most 1 KiB).
const MAX_PAYLOAD: u64 = 64 * 1024;
const TRANSPORT_FIELD: &[u8] = b"_TRANSPORT";
const TRANSPORT_KERNEL: &[u8] = b"_TRANSPORT=kernel";
const MESSAGE_PREFIX: &[u8] = b"MESSAGE=";

/// One kernel line from the journal, with the record's own time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KernelRecord {
    /// Wall-clock time journald stored for the line, in microseconds.
    pub realtime_usec: u64,
    pub monotonic_usec: u64,
    /// The boot the line belongs to (32 hex digits).
    pub boot_id: String,
    /// The journal's sequence number space (32 hex digits) and the line's
    /// number in it: together they identify the line.
    pub seqnum_id: String,
    pub seqnum: u64,
    pub message: String,
}

/// What a read could not use, by reason.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReadStats {
    pub files_read: u64,
    /// Files that could not be opened or are not journal files.
    pub files_unreadable: u64,
    /// Files whose read stopped at a bad offset or object (being written
    /// or corrupt); what was read before it is kept.
    pub files_truncated: u64,
    /// Kernel lines whose `MESSAGE` is compressed or missing.
    pub messages_skipped: u64,
}

impl ReadStats {
    fn merge(&mut self, other: &ReadStats) {
        self.files_read += other.files_read;
        self.files_unreadable += other.files_unreadable;
        self.files_truncated += other.files_truncated;
        self.messages_skipped += other.messages_skipped;
    }
}

fn id_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    bytes
        .get(at..at + 8)
        .and_then(|slice| slice.try_into().ok())
        .map_or(0, u64::from_le_bytes)
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    bytes
        .get(at..at + 4)
        .and_then(|slice| slice.try_into().ok())
        .map_or(0, u32::from_le_bytes)
}

/// The header fields this reader uses.
#[derive(Clone, Debug)]
struct Header {
    compact: bool,
    archived: bool,
    file_id: [u8; 16],
    seqnum_id: String,
    field_hash_table_offset: u64,
    field_hash_table_size: u64,
    tail_entry_realtime: u64,
}

/// One open journal file.
struct JournalFile {
    file: File,
    len: u64,
    header: Header,
}

/// A read failed inside a file: the walk stops there.
#[derive(Debug)]
struct Bad;

impl JournalFile {
    fn open(path: &Path) -> io::Result<Option<Self>> {
        let file = File::open(path)?;
        let len = file.metadata()?.len();
        let mut head = [0_u8; 208];
        if len < head.len() as u64 {
            return Ok(None);
        }
        file.read_exact_at(&mut head, 0)?;
        if &head[..8] != SIGNATURE {
            return Ok(None);
        }
        let incompatible = u32_at(&head, 12);
        let mut file_id = [0_u8; 16];
        file_id.copy_from_slice(&head[24..40]);
        Ok(Some(Self {
            file,
            len,
            header: Header {
                compact: incompatible & INCOMPATIBLE_COMPACT != 0,
                archived: head[16] == STATE_ARCHIVED,
                file_id,
                seqnum_id: id_hex(&head[72..88]),
                field_hash_table_offset: u64_at(&head, 120),
                field_hash_table_size: u64_at(&head, 128),
                tail_entry_realtime: u64_at(&head, 192),
            },
        }))
    }

    fn read(&self, offset: u64, len: u64) -> Result<Vec<u8>, Bad> {
        let end = offset.checked_add(len).ok_or(Bad)?;
        if end > self.len || len > MAX_PAYLOAD + 128 {
            return Err(Bad);
        }
        let mut buffer = vec![0_u8; usize::try_from(len).map_err(|_| Bad)?];
        self.file
            .read_exact_at(&mut buffer, offset)
            .map_err(|_| Bad)?;
        Ok(buffer)
    }

    /// An object's type, flags and size, checked against the file.
    fn object(&self, offset: u64, expected: u8) -> Result<(u8, u64), Bad> {
        if offset == 0 || offset % 8 != 0 {
            return Err(Bad);
        }
        let head = self.read(offset, 16)?;
        let size = u64_at(&head, 8);
        if head[0] != expected || size < 16 || offset.checked_add(size).ok_or(Bad)? > self.len {
            return Err(Bad);
        }
        Ok((head[1], size))
    }

    fn data_payload_start(&self) -> u64 {
        if self.header.compact {
            72
        } else {
            64
        }
    }

    /// The `_TRANSPORT=kernel` data object, through the field hash table.
    fn kernel_data(&self) -> Result<Option<u64>, Bad> {
        // A few hundred buckets: read in one go.
        let table = self.read(
            self.header.field_hash_table_offset,
            self.header.field_hash_table_size.min(MAX_PAYLOAD),
        )?;
        let mut steps = 0_usize;
        for bucket in table.chunks_exact(16) {
            let mut field = u64_at(bucket, 0);
            while field != 0 {
                steps += 1;
                if steps > MAX_STEPS {
                    return Err(Bad);
                }
                let (_, size) = self.object(field, OBJECT_FIELD)?;
                let body = self.read(field, size.min(40 + MAX_PAYLOAD))?;
                if body.get(40..) == Some(TRANSPORT_FIELD) {
                    return self.find_data(u64_at(&body, 32), TRANSPORT_KERNEL);
                }
                field = u64_at(&body, 24);
            }
        }
        Ok(None)
    }

    /// The data object of a field's chain whose payload is `payload`.
    fn find_data(&self, mut data: u64, payload: &[u8]) -> Result<Option<u64>, Bad> {
        let start = self.data_payload_start();
        let mut steps = 0_usize;
        while data != 0 {
            steps += 1;
            if steps > MAX_STEPS {
                return Err(Bad);
            }
            let (flags, size) = self.object(data, OBJECT_DATA)?;
            if flags & OBJECT_COMPRESSED == 0 && size == start + payload.len() as u64 {
                let body = self.read(data, size)?;
                if &body[start as usize..] == payload {
                    return Ok(Some(data));
                }
            }
            let head = self.read(data, 40)?;
            data = u64_at(&head, 32);
        }
        Ok(None)
    }

    /// Offsets of the entries that reference `data`, oldest first, after
    /// the first `skip`.
    fn entries_of(&self, data: u64, skip: u64) -> Result<(Vec<u64>, u64), Bad> {
        let head = self.read(data, 64)?;
        let first = u64_at(&head, 40);
        let mut array = u64_at(&head, 48);
        let total = u64_at(&head, 56);
        let mut offsets = Vec::new();
        let mut seen = 0_u64;
        if total == 0 || first == 0 {
            return Ok((offsets, 0));
        }
        seen += 1;
        if seen > skip {
            offsets.push(first);
        }
        let item_size: u64 = if self.header.compact { 4 } else { 8 };
        let mut steps = 0_usize;
        while array != 0 && seen < total {
            let (_, size) = self.object(array, OBJECT_ENTRY_ARRAY)?;
            let body = self.read(array, size.min(24 + MAX_PAYLOAD))?;
            let items = (body.len() as u64).saturating_sub(24) / item_size;
            for index in 0..items {
                steps += 1;
                if steps > MAX_STEPS {
                    return Err(Bad);
                }
                let at = (24 + index * item_size) as usize;
                let offset = if self.header.compact {
                    u64::from(u32_at(&body, at))
                } else {
                    u64_at(&body, at)
                };
                if offset == 0 || seen >= total {
                    break;
                }
                seen += 1;
                if seen > skip {
                    offsets.push(offset);
                }
            }
            array = u64_at(&body, 16);
        }
        Ok((offsets, seen))
    }

    /// One entry's record; `None` when its `MESSAGE` is compressed or absent.
    fn record(&self, entry: u64) -> Result<Option<KernelRecord>, Bad> {
        let (_, size) = self.object(entry, OBJECT_ENTRY)?;
        let body = self.read(entry, size.min(64 + MAX_PAYLOAD))?;
        let item_size: usize = if self.header.compact { 4 } else { 16 };
        let items = body.len().saturating_sub(64) / item_size;
        let start = self.data_payload_start();
        for index in 0..items {
            let at = 64 + index * item_size;
            let data = if self.header.compact {
                u64::from(u32_at(&body, at))
            } else {
                u64_at(&body, at)
            };
            let (flags, data_size) = self.object(data, OBJECT_DATA)?;
            if data_size < start + MESSAGE_PREFIX.len() as u64 {
                continue;
            }
            let prefix = self.read(data + start, MESSAGE_PREFIX.len() as u64)?;
            if prefix != MESSAGE_PREFIX {
                continue;
            }
            if flags & OBJECT_COMPRESSED != 0 {
                return Ok(None);
            }
            let text = self.read(
                data + start + MESSAGE_PREFIX.len() as u64,
                (data_size - start - MESSAGE_PREFIX.len() as u64).min(MAX_PAYLOAD),
            )?;
            return Ok(Some(KernelRecord {
                seqnum: u64_at(&body, 16),
                realtime_usec: u64_at(&body, 24),
                monotonic_usec: u64_at(&body, 32),
                boot_id: id_hex(&body[40..56]),
                seqnum_id: self.header.seqnum_id.clone(),
                message: String::from_utf8_lossy(&text).into_owned(),
            }));
        }
        Ok(None)
    }
}

/// The journal files under `roots`: each root's `*.journal` files and
/// those of its subdirectories (`<machine-id>/`). User journals and files
/// journald left unclean (`*.journal~`) are left out.
pub fn journal_files(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let visit = |dir: &Path, depth: u8, files: &mut Vec<PathBuf>| -> Vec<PathBuf> {
        let mut subdirs = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return subdirs;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() && depth == 0 {
                subdirs.push(path);
            } else if name.ends_with(".journal") && !name.starts_with("user-") {
                files.push(path);
            }
        }
        subdirs
    };
    for root in roots {
        for subdir in visit(root, 0, &mut files) {
            visit(&subdir, 1, &mut files);
        }
    }
    files.sort();
    files
}

/// Reads the kernel's lines as journald appends them: each call returns
/// the lines of `boot_id` not returned before, oldest first. Progress is
/// kept per journal file id, so a rotated (renamed) file is not re-read.
#[derive(Debug, Default)]
pub struct KernelJournal {
    roots: Vec<PathBuf>,
    /// Kernel entries already returned (or passed over), per file id.
    consumed: HashMap<[u8; 16], u64>,
    /// Archived files read to their end.
    finished: HashSet<[u8; 16]>,
}

impl KernelJournal {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self {
            roots,
            consumed: HashMap::new(),
            finished: HashSet::new(),
        }
    }

    /// The new kernel lines of `boot_id` (32 hex digits), in record order,
    /// from files written at or after `since_realtime_usec`.
    pub fn poll(
        &mut self,
        boot_id: &str,
        since_realtime_usec: u64,
    ) -> (Vec<KernelRecord>, ReadStats) {
        let mut stats = ReadStats::default();
        let mut records = Vec::new();
        for path in journal_files(&self.roots) {
            let (mut found, file_stats) = self.read_file(&path, boot_id, since_realtime_usec);
            stats.merge(&file_stats);
            records.append(&mut found);
        }
        records.sort_by(|a, b| {
            (a.realtime_usec, &a.seqnum_id, a.seqnum).cmp(&(
                b.realtime_usec,
                &b.seqnum_id,
                b.seqnum,
            ))
        });
        records.dedup_by(|a, b| a.seqnum_id == b.seqnum_id && a.seqnum == b.seqnum);
        (records, stats)
    }

    fn read_file(
        &mut self,
        path: &Path,
        boot_id: &str,
        since: u64,
    ) -> (Vec<KernelRecord>, ReadStats) {
        let mut stats = ReadStats::default();
        let journal = match JournalFile::open(path) {
            Ok(Some(journal)) => journal,
            _ => {
                stats.files_unreadable += 1;
                return (Vec::new(), stats);
            }
        };
        if journal.header.tail_entry_realtime != 0 && journal.header.tail_entry_realtime < since {
            return (Vec::new(), stats);
        }
        // An archived file never grows: once read whole, never again.
        if journal.header.archived && self.finished.contains(&journal.header.file_id) {
            return (Vec::new(), stats);
        }
        stats.files_read += 1;
        let skip = self
            .consumed
            .get(&journal.header.file_id)
            .copied()
            .unwrap_or(0);
        let entries = journal.kernel_data().and_then(|data| match data {
            Some(data) => journal.entries_of(data, skip),
            None => Ok((Vec::new(), skip)),
        });
        let (offsets, _seen) = match entries {
            Ok(found) => found,
            Err(Bad) => {
                stats.files_truncated += 1;
                return (Vec::new(), stats);
            }
        };
        let mut records = Vec::new();
        let mut consumed = skip;
        let mut whole = true;
        for offset in offsets {
            match journal.record(offset) {
                Ok(Some(record)) => {
                    consumed += 1;
                    if record.boot_id == boot_id && record.realtime_usec >= since {
                        records.push(record);
                    }
                }
                Ok(None) => {
                    consumed += 1;
                    stats.messages_skipped += 1;
                }
                // Being written: read it again next time.
                Err(Bad) => {
                    stats.files_truncated += 1;
                    whole = false;
                    break;
                }
            }
        }
        if whole && journal.header.archived {
            self.finished.insert(journal.header.file_id);
        }
        self.consumed.insert(journal.header.file_id, consumed);
        (records, stats)
    }
}

#[cfg(test)]
pub(crate) mod testing {
    //! A minimal journal file writer for tests: the objects this reader
    //! walks, in the layout journald writes (regular or compact).

    use super::*;

    pub struct Line<'a> {
        pub seqnum: u64,
        pub realtime_usec: u64,
        pub boot: [u8; 16],
        pub kernel: bool,
        pub message: &'a str,
    }

    fn align(buffer: &mut Vec<u8>) {
        while buffer.len() % 8 != 0 {
            buffer.push(0);
        }
    }

    fn object(buffer: &mut Vec<u8>, kind: u8, body: &[u8]) -> u64 {
        align(buffer);
        let offset = buffer.len() as u64;
        buffer.push(kind);
        buffer.extend_from_slice(&[0; 7]);
        buffer.extend_from_slice(&((16 + body.len()) as u64).to_le_bytes());
        buffer.extend_from_slice(body);
        offset
    }

    fn set_u64(buffer: &mut [u8], at: usize, value: u64) {
        buffer[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    /// A journal file holding `lines`; the kernel lines are listed through
    /// `_TRANSPORT=kernel`: the first inline, the rest in two entry arrays.
    pub fn journal(compact: bool, file_id: u8, lines: &[Line<'_>]) -> Vec<u8> {
        let mut buffer = vec![0_u8; 272];
        buffer[..8].copy_from_slice(SIGNATURE);
        let incompatible: u32 = if compact { INCOMPATIBLE_COMPACT } else { 0 };
        buffer[12..16].copy_from_slice(&incompatible.to_le_bytes());
        buffer[24..40].copy_from_slice(&[file_id; 16]);
        buffer[72..88].copy_from_slice(&[0xab; 16]);
        set_u64(&mut buffer, 88, 272);
        // Field hash table: 4 buckets, the field in bucket 2.
        let table = object(&mut buffer, 5, &[0; 64]) + 16;
        let payload_pad: usize = if compact { 8 } else { 0 };
        let data_body = |payload: &[u8], next_field: u64| {
            let mut body = vec![0_u8; 48 + payload_pad];
            body[16..24].copy_from_slice(&next_field.to_le_bytes());
            body.extend_from_slice(payload);
            body
        };
        // A decoy data object first in the field's chain.
        let other = object(
            &mut buffer,
            OBJECT_DATA,
            &data_body(b"_TRANSPORT=journal", 0),
        );
        let kernel = object(&mut buffer, OBJECT_DATA, &data_body(TRANSPORT_KERNEL, 0));
        set_u64(&mut buffer, other as usize + 32, kernel);
        let mut field_body = vec![0_u8; 24];
        field_body[16..24].copy_from_slice(&other.to_le_bytes());
        field_body.extend_from_slice(TRANSPORT_FIELD);
        let field = object(&mut buffer, OBJECT_FIELD, &field_body);
        set_u64(&mut buffer, (table + 2 * 16) as usize, field);
        set_u64(&mut buffer, 120, table);
        set_u64(&mut buffer, 128, 64);
        let mut kernel_entries = Vec::new();
        let mut tail = 0;
        for line in lines {
            let message = format!("MESSAGE={}", line.message);
            let data = object(&mut buffer, OBJECT_DATA, &data_body(message.as_bytes(), 0));
            let host = object(&mut buffer, OBJECT_DATA, &data_body(b"_HOSTNAME=amedes", 0));
            let mut body = vec![0_u8; 48];
            body[0..8].copy_from_slice(&line.seqnum.to_le_bytes());
            body[8..16].copy_from_slice(&line.realtime_usec.to_le_bytes());
            body[24..40].copy_from_slice(&line.boot);
            let mut items = vec![host];
            if line.kernel {
                items.push(kernel);
            }
            items.push(data);
            for item in items {
                if compact {
                    body.extend_from_slice(&(item as u32).to_le_bytes());
                } else {
                    body.extend_from_slice(&item.to_le_bytes());
                    body.extend_from_slice(&[0; 8]);
                }
            }
            let entry = object(&mut buffer, OBJECT_ENTRY, &body);
            tail = tail.max(line.realtime_usec);
            if line.kernel {
                kernel_entries.push(entry);
            }
        }
        if let Some((first, rest)) = kernel_entries.split_first() {
            set_u64(&mut buffer, kernel as usize + 40, *first);
            // Two arrays, the second with an unused slot at its end.
            let half = rest.len() / 2;
            let array = |items: &[u64], spare: usize| {
                let mut body = vec![0_u8; 8];
                for item in items {
                    if compact {
                        body.extend_from_slice(&(*item as u32).to_le_bytes());
                    } else {
                        body.extend_from_slice(&item.to_le_bytes());
                    }
                }
                body.extend(std::iter::repeat(0).take(spare * if compact { 4 } else { 8 }));
                body
            };
            let second = object(&mut buffer, OBJECT_ENTRY_ARRAY, &array(&rest[half..], 1));
            let first_array = object(&mut buffer, OBJECT_ENTRY_ARRAY, &array(&rest[..half], 0));
            set_u64(&mut buffer, first_array as usize + 16, second);
            set_u64(&mut buffer, kernel as usize + 48, first_array);
        }
        set_u64(
            &mut buffer,
            kernel as usize + 56,
            kernel_entries.len() as u64,
        );
        set_u64(&mut buffer, 192, tail);
        align(&mut buffer);
        buffer
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{journal, Line};
    use super::*;

    const BOOT: [u8; 16] = [0x75; 16];
    const OLD_BOOT: [u8; 16] = [0x11; 16];

    fn lines() -> Vec<Line<'static>> {
        vec![
            Line { seqnum: 1, realtime_usec: 100, boot: OLD_BOOT, kernel: true, message: "old boot line" },
            Line { seqnum: 2, realtime_usec: 200, boot: BOOT, kernel: true, message: "Linux version 6.12.30" },
            Line { seqnum: 3, realtime_usec: 300, boot: BOOT, kernel: false, message: "a service line" },
            Line { seqnum: 4, realtime_usec: 400, boot: BOOT, kernel: true, message: "rustc[1]: segfault at 8 ip 1 sp 2 error 4 in libgcc_s.so.1[1,2+3] likely on CPU 20 (core 4, socket 0)" },
            Line { seqnum: 5, realtime_usec: 500, boot: BOOT, kernel: true, message: "usb 1-1: new device" },
            Line { seqnum: 6, realtime_usec: 600, boot: BOOT, kernel: true, message: "traps: x[2] general protection fault ip:1 sp:2 error:0 in y[1+2]" },
        ]
    }

    fn write(dir: &Path, name: &str, bytes: &[u8]) {
        let machine = dir.join("18b4817d304846508341fdea61d4b354");
        std::fs::create_dir_all(&machine).unwrap();
        std::fs::write(machine.join(name), bytes).unwrap();
    }

    fn boot() -> String {
        id_hex(&BOOT)
    }

    #[test]
    fn reads_the_kernel_lines_of_this_boot_from_regular_and_compact_files() {
        for compact in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            write(dir.path(), "system.journal", &journal(compact, 1, &lines()));
            let mut reader = KernelJournal::new(vec![dir.path().to_path_buf()]);
            let (records, stats) = reader.poll(&boot(), 0);
            let messages: Vec<&str> = records.iter().map(|r| r.message.as_str()).collect();
            assert_eq!(
                messages,
                [
                    "Linux version 6.12.30",
                    lines()[3].message,
                    "usb 1-1: new device",
                    lines()[5].message,
                ],
                "compact={compact}"
            );
            assert_eq!(records[1].seqnum, 4);
            assert_eq!(records[1].realtime_usec, 400);
            assert_eq!(records[1].boot_id, boot());
            assert_eq!(records[1].seqnum_id, "ab".repeat(16));
            assert_eq!(stats.files_read, 1);
            assert_eq!(stats.files_truncated, 0);
            // Nothing new: nothing returned.
            let (again, _) = reader.poll(&boot(), 0);
            assert!(again.is_empty());
        }
    }

    #[test]
    fn returns_only_what_was_appended_since_the_last_poll() {
        let dir = tempfile::tempdir().unwrap();
        let mut all = lines();
        write(dir.path(), "system.journal", &journal(true, 1, &all[..4]));
        let mut reader = KernelJournal::new(vec![dir.path().to_path_buf()]);
        let (first, _) = reader.poll(&boot(), 0);
        assert_eq!(first.len(), 2);
        // journald appended; the file (same id) is rewritten here whole.
        all.push(Line {
            seqnum: 7,
            realtime_usec: 700,
            boot: BOOT,
            kernel: true,
            message: "late",
        });
        write(dir.path(), "system.journal", &journal(true, 1, &all));
        let (next, _) = reader.poll(&boot(), 0);
        let messages: Vec<&str> = next.iter().map(|r| r.message.as_str()).collect();
        assert_eq!(
            messages,
            ["usb 1-1: new device", lines()[5].message, "late"]
        );
    }

    #[test]
    fn a_window_and_old_files_bound_the_read() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "system.journal", &journal(false, 1, &lines()));
        let mut reader = KernelJournal::new(vec![dir.path().to_path_buf()]);
        let (records, stats) = reader.poll(&boot(), 450);
        assert_eq!(records.len(), 2);
        assert_eq!(stats.files_read, 1);
        let mut fresh = KernelJournal::new(vec![dir.path().to_path_buf()]);
        let (none, stats) = fresh.poll(&boot(), 10_000);
        assert!(none.is_empty());
        assert_eq!(
            stats.files_read, 0,
            "a file older than the window is not read"
        );
    }

    #[test]
    fn corrupt_truncated_and_foreign_files_are_counted_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let good = journal(false, 1, &lines());
        write(dir.path(), "system.journal", &good);
        // Cut inside the objects: the header still points past the end.
        write(dir.path(), "system@cut.journal", &{
            let mut cut = journal(false, 2, &lines());
            cut.truncate(cut.len() * 2 / 3);
            cut
        });
        write(
            dir.path(),
            "system@garbage.journal",
            b"not a journal file at all",
        );
        write(
            dir.path(),
            "user-1000.journal",
            &journal(false, 3, &lines()),
        );
        write(
            dir.path(),
            "system@dirty.journal~",
            &journal(false, 4, &lines()),
        );
        // Random bytes after a valid header.
        let mut noise = good.clone();
        for (index, byte) in noise.iter_mut().enumerate().skip(272) {
            *byte = (index * 31 % 251) as u8;
        }
        noise[24..40].copy_from_slice(&[5; 16]);
        write(dir.path(), "system@noise.journal", &noise);
        let mut reader = KernelJournal::new(vec![dir.path().to_path_buf()]);
        let (records, stats) = reader.poll(&boot(), 0);
        assert_eq!(records.len(), 4, "the good file is read whole");
        assert_eq!(stats.files_unreadable, 1);
        assert!(stats.files_truncated >= 1, "{stats:?}");
    }

    #[test]
    fn an_archived_file_read_whole_is_not_opened_for_entries_again() {
        let dir = tempfile::tempdir().unwrap();
        let mut archived = journal(false, 1, &lines());
        archived[16] = STATE_ARCHIVED;
        write(dir.path(), "system@a.journal", &archived);
        let mut reader = KernelJournal::new(vec![dir.path().to_path_buf()]);
        let (records, stats) = reader.poll(&boot(), 0);
        assert_eq!((records.len(), stats.files_read), (4, 1));
        // Even if its bytes changed, an archived file is not walked again.
        let mut more = lines();
        more.push(Line {
            seqnum: 7,
            realtime_usec: 700,
            boot: BOOT,
            kernel: true,
            message: "never",
        });
        let mut changed = journal(false, 1, &more);
        changed[16] = STATE_ARCHIVED;
        write(dir.path(), "system@a.journal", &changed);
        let (records, stats) = reader.poll(&boot(), 0);
        assert!(records.is_empty());
        assert_eq!(stats.files_read, 0);
    }

    #[test]
    fn lists_system_journals_one_level_down() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "system.journal", b"x");
        write(dir.path(), "user-1000.journal", b"x");
        std::fs::write(dir.path().join("top.journal"), b"x").unwrap();
        let files = journal_files(&[dir.path().to_path_buf(), dir.path().join("missing")]);
        let names: Vec<String> = files
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["system.journal", "top.journal"]);
    }
}
