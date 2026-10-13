//! Kernel log lines that report a fault, parsed into typed records.
//!
//! The kernel prints one line per fault; this module reads the shapes
//! Linux 6.x prints on amd64 and arm64:
//!
//! ```text
//! rustc[2772395]: segfault at 8 ip 00007f1a79ccf8ee sp 00007f1a79bb06b0 error 4 in libgcc_s.so.1[158ee,7f1a79cbd000+17000] likely on CPU 1 (core 1, socket 0)
//! traps: opt cgu.00[848121] general protection fault ip:7f47836c84cb sp:7f463f1f7d70 error:0 in libLLVM.so[7ec84cb,7f4783400000+1f8c000]
//! traps: foo[12] trap invalid opcode ip:55d0c0a0 sp:7ffd0e00 error:0 in foo[55d0c000+1000]
//! foo[12]: unhandled level 2 translation fault (11) at 0x00000008, esr 0x92000006, in libc.so.6[ffff8000+190000]   (arm64)
//! mce: [Hardware Error]: Machine check events logged
//! EDAC MC0: 1 CE on mc#0csrow#0channel#0 (csrow:0 channel:0 page:0x0 offset:0x0 grain:8)
//! I/O error, dev sda, sector 2048 op 0x0:(READ) flags 0x0 phys_seg 1 prio class 2
//! Buffer I/O error on dev sda1, logical block 0, async page read
//! EXT4-fs error (device sda1): ext4_find_entry:1455: inode #2: comm ls: reading directory lblock 0
//! XFS (dm-0): Corruption detected. Unmount and run xfs_repair
//! BTRFS error (device sda1): bdev /dev/sda1 errs: wr 0, rd 1, flush 0, corrupt 0, gen 0
//! Memory cgroup out of memory: Killed process 4242 (rustc) total-vm:1000kB, anon-rss:900kB, ...
//! Out of memory: Killed process 4242 (rustc) total-vm:1000kB, ...
//! ```
//!
//! [`parse_line`] never panics. A line that has a fault's marker but not
//! the fields that identify it (a segfault without `comm[pid]`, say) is
//! [`Parsed::Malformed`], so the caller counts what it skipped instead of
//! dropping it silently or inventing fields.

use std::fmt;

/// What went wrong, as one word for attributes and dashboards.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FaultKind {
    /// A user process touched memory it may not (SIGSEGV), on amd64
    /// (`segfault at`) or arm64 (`unhandled ... fault`).
    Segfault,
    /// `traps: ... general protection fault`: often a corrupted pointer or,
    /// on flaky hardware, a bit flip.
    GeneralProtection,
    /// Any other `traps:` line (invalid opcode, divide error, ...).
    Trap,
    /// Machine-check and EDAC (memory controller) errors: the hardware
    /// itself reports a fault.
    MachineCheck,
    /// A block device failed a read or write.
    IoError,
    /// An ext4, XFS or Btrfs error.
    FilesystemError,
    /// The kernel (or a memory cgroup) killed a process to free memory.
    OomKill,
}

impl FaultKind {
    /// Every kind, in a stable order.
    pub const ALL: [FaultKind; 7] = [
        FaultKind::Segfault,
        FaultKind::GeneralProtection,
        FaultKind::Trap,
        FaultKind::MachineCheck,
        FaultKind::IoError,
        FaultKind::FilesystemError,
        FaultKind::OomKill,
    ];

    /// The attribute value (`ih.kernel.fault.kind`).
    pub fn as_str(self) -> &'static str {
        match self {
            FaultKind::Segfault => "segfault",
            FaultKind::GeneralProtection => "general_protection",
            FaultKind::Trap => "trap",
            FaultKind::MachineCheck => "machine_check",
            FaultKind::IoError => "io_error",
            FaultKind::FilesystemError => "filesystem_error",
            FaultKind::OomKill => "oom_kill",
        }
    }

    /// Hardware and storage faults are errors; process crashes and OOM
    /// kills are warnings (a buggy program crashes too).
    pub fn severity(self) -> &'static str {
        match self {
            FaultKind::MachineCheck | FaultKind::IoError | FaultKind::FilesystemError => "error",
            _ => "warning",
        }
    }
}

impl fmt::Display for FaultKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One fault line's fields; what the line does not state is `None`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FaultFields {
    /// Process name (`comm`, at most 15 bytes in the kernel, may hold spaces).
    pub process: Option<String>,
    pub pid: Option<u32>,
    /// The library or binary the faulting instruction was in.
    pub object: Option<String>,
    /// Instruction pointer.
    pub ip: Option<u64>,
    /// Faulting address (`segfault at`).
    pub address: Option<u64>,
    /// The kernel's error code, as printed (`4`, `0`, `0x92000006`).
    pub error_code: Option<String>,
    /// Logical CPU, and its core and socket, when the kernel says (6.1+).
    pub cpu: Option<u32>,
    pub core: Option<u32>,
    pub socket: Option<u32>,
    /// The trap's name (`invalid opcode`) for [`FaultKind::Trap`].
    pub trap: Option<String>,
    /// Block device (`sda`, `nvme0n1`, `dm-0`) for I/O and filesystem errors.
    pub device: Option<String>,
    /// `ext4`, `xfs` or `btrfs`.
    pub filesystem: Option<String>,
    /// `cgroup` or `global` for [`FaultKind::OomKill`].
    pub oom_scope: Option<String>,
}

/// A kernel line that reports a fault.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KernelFault {
    pub kind: FaultKind,
    pub fields: FaultFields,
}

/// What one kernel line is (short-lived: boxing the fault buys nothing).
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Parsed {
    Fault(KernelFault),
    /// Has a fault's marker but not its identifying fields: skipped, counted.
    Malformed(FaultKind),
    /// Any other kernel line.
    NotAFault,
}

/// Parses one kernel message (the journal's `MESSAGE`, without a timestamp
/// prefix).
pub fn parse_line(line: &str) -> Parsed {
    let line = line.trim_end_matches(['\n', '\r']);
    if let Some(at) = line.find(": segfault at ") {
        return segfault(line, at);
    }
    if let Some(rest) = line.strip_prefix("traps: ") {
        return trap(rest);
    }
    if line.contains("]: unhandled ") && line.contains(" fault") {
        return arm64_fault(line);
    }
    if let Some(parsed) = oom(line) {
        return parsed;
    }
    if machine_check(line) {
        return Parsed::Fault(KernelFault {
            kind: FaultKind::MachineCheck,
            fields: FaultFields {
                cpu: machine_check_cpu(line),
                ..FaultFields::default()
            },
        });
    }
    if let Some(parsed) = io_error(line) {
        return parsed;
    }
    if let Some(parsed) = filesystem_error(line) {
        return parsed;
    }
    Parsed::NotAFault
}

/// `comm[pid]` at the end of `head`: the name may hold spaces and brackets.
fn comm_pid(head: &str) -> Option<(String, u32)> {
    let head = head.strip_suffix(']')?;
    let open = head.rfind('[')?;
    let pid = head[open + 1..].parse().ok()?;
    let comm = &head[..open];
    (!comm.is_empty()).then(|| (comm.to_string(), pid))
}

fn hex(text: &str) -> Option<u64> {
    let text = text.trim_start_matches("0x");
    u64::from_str_radix(text, 16).ok()
}

/// The word after `key` (`ip`, `sp`, `error`) in `ip 0000 sp 0000 error 4`.
fn word_after<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let mut words = text.split_whitespace();
    while let Some(word) = words.next() {
        if word == key {
            return words.next();
        }
    }
    None
}

/// The value of `key:` in `ip:7f47 sp:7f46 error:0`.
fn colon_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.split_whitespace()
        .find_map(|word| word.strip_prefix(key)?.strip_prefix(':'))
}

/// The object of ` in libgcc_s.so.1[158ee,7f1a79cbd000+17000]`.
fn object(text: &str) -> Option<String> {
    let start = text.find(" in ")? + 4;
    let rest = &text[start..];
    // The bracket that closes the object's range, then the one that opens it.
    let close = rest.find(']')?;
    let open = rest[..close].rfind('[')?;
    let name = rest[..open].trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// `likely on CPU 20 (core 4, socket 0)` (6.1+), or an older ` cpu 3 `.
fn cpu_core_socket(text: &str) -> (Option<u32>, Option<u32>, Option<u32>) {
    if let Some(start) = text.find("likely on CPU ") {
        let rest = &text[start + "likely on CPU ".len()..];
        let cpu = rest
            .split(|c: char| !c.is_ascii_digit())
            .next()
            .and_then(|digits| digits.parse().ok());
        let number_after = |key: &str| {
            let at = rest.find(key)? + key.len();
            rest[at..]
                .split(|c: char| !c.is_ascii_digit())
                .next()?
                .parse()
                .ok()
        };
        return (cpu, number_after("core "), number_after("socket "));
    }
    let cpu = word_after(text, "cpu").and_then(|value| value.parse().ok());
    (cpu, None, None)
}

fn segfault(line: &str, at: usize) -> Parsed {
    let Some((process, pid)) = comm_pid(&line[..at]) else {
        return Parsed::Malformed(FaultKind::Segfault);
    };
    let rest = &line[at + 2..];
    let (cpu, core, socket) = cpu_core_socket(rest);
    Parsed::Fault(KernelFault {
        kind: FaultKind::Segfault,
        fields: FaultFields {
            process: Some(process),
            pid: Some(pid),
            object: object(rest),
            ip: word_after(rest, "ip").and_then(hex),
            address: word_after(rest, "at").and_then(hex),
            error_code: word_after(rest, "error").map(str::to_string),
            cpu,
            core,
            socket,
            ..FaultFields::default()
        },
    })
}

fn trap(rest: &str) -> Parsed {
    let (head, tail, kind, name) = if let Some(at) = rest.find(" general protection fault") {
        (&rest[..at], &rest[at..], FaultKind::GeneralProtection, None)
    } else if let Some(at) = rest.find(" trap ") {
        let tail = &rest[at + " trap ".len()..];
        let name = tail.split(" ip:").next().unwrap_or("").trim();
        (
            &rest[..at],
            tail,
            FaultKind::Trap,
            (!name.is_empty()).then(|| name.to_string()),
        )
    } else {
        return Parsed::Malformed(FaultKind::Trap);
    };
    let Some((process, pid)) = comm_pid(head) else {
        return Parsed::Malformed(kind);
    };
    let (cpu, core, socket) = cpu_core_socket(tail);
    Parsed::Fault(KernelFault {
        kind,
        fields: FaultFields {
            process: Some(process),
            pid: Some(pid),
            object: object(tail),
            ip: colon_value(tail, "ip").and_then(hex),
            error_code: colon_value(tail, "error").map(str::to_string),
            cpu,
            core,
            socket,
            trap: name,
            ..FaultFields::default()
        },
    })
}

/// arm64: `foo[12]: unhandled level 2 translation fault (11) at 0x8, esr 0x92000006, in libc.so.6[...]`.
fn arm64_fault(line: &str) -> Parsed {
    let Some(at) = line.find("]: unhandled ") else {
        return Parsed::Malformed(FaultKind::Segfault);
    };
    let Some((process, pid)) = comm_pid(&line[..=at]) else {
        return Parsed::Malformed(FaultKind::Segfault);
    };
    let rest = &line[at + 2..];
    let after = |key: &str| {
        let start = rest.find(key)? + key.len();
        rest[start..]
            .split(|c: char| c == ',' || c.is_whitespace())
            .find(|word| !word.is_empty())
    };
    Parsed::Fault(KernelFault {
        kind: FaultKind::Segfault,
        fields: FaultFields {
            process: Some(process),
            pid: Some(pid),
            object: object(rest),
            address: after(" at ").and_then(hex),
            error_code: after("esr ").map(str::to_string),
            ..FaultFields::default()
        },
    })
}

/// `[Memory cgroup ]out of memory: Killed process 4242 (rustc) ...`.
fn oom(line: &str) -> Option<Parsed> {
    let marker = "ut of memory: Killed process ";
    let at = line.find(marker)?;
    let scope = if line.starts_with("Memory cgroup") {
        "cgroup"
    } else {
        "global"
    };
    let rest = &line[at + marker.len()..];
    let pid = rest.split_whitespace().next()?.parse::<u32>().ok();
    let name = rest
        .find(" (")
        .and_then(|open| {
            let tail = &rest[open + 2..];
            // The name may hold parentheses; it ends at `) ` before the sizes.
            tail.find(") total-vm")
                .or_else(|| tail.rfind(')'))
                .map(|close| tail[..close].to_string())
        })
        .filter(|name| !name.is_empty());
    let (Some(pid), Some(name)) = (pid, name) else {
        return Some(Parsed::Malformed(FaultKind::OomKill));
    };
    Some(Parsed::Fault(KernelFault {
        kind: FaultKind::OomKill,
        fields: FaultFields {
            process: Some(name),
            pid: Some(pid),
            oom_scope: Some(scope.into()),
            ..FaultFields::default()
        },
    }))
}

/// Machine-check and EDAC error lines, not their boot-time notices
/// (`mce: CPU0: Thermal monitoring enabled`, `EDAC MC: Ver: 3.0.0`,
/// `EDAC amd64: Node 0: DRAM ECC disabled.`).
fn machine_check(line: &str) -> bool {
    if line.contains("[Hardware Error]") || line.contains("Machine check events logged") {
        return true;
    }
    if line.starts_with("mce: ") {
        let lower = line.to_ascii_lowercase();
        return lower.contains("machine check") && !lower.contains("enabled");
    }
    if line.starts_with("EDAC ") {
        // Corrected / uncorrected error reports: `1 CE on ...`, `1 UE ...`,
        // or a line naming an error.
        let lower = line.to_ascii_lowercase();
        return line.contains(" CE ") || line.contains(" UE ") || lower.contains("error");
    }
    false
}

fn machine_check_cpu(line: &str) -> Option<u32> {
    let at = line.find("CPU")? + 3;
    line[at..]
        .trim_start_matches([' ', ':'])
        .split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

/// `I/O error, dev sda, sector ...` and `Buffer I/O error on dev sda1, ...`.
fn io_error(line: &str) -> Option<Parsed> {
    let device = if let Some(at) = line.find("I/O error, dev ") {
        &line[at + "I/O error, dev ".len()..]
    } else if let Some(at) = line.find("I/O error on dev ") {
        &line[at + "I/O error on dev ".len()..]
    } else if line.contains("I/O error") {
        return Some(Parsed::Malformed(FaultKind::IoError));
    } else {
        return None;
    };
    let device = device
        .split([',', ' '])
        .next()
        .filter(|device| !device.is_empty())
        .map(str::to_string);
    Some(Parsed::Fault(KernelFault {
        kind: FaultKind::IoError,
        fields: FaultFields {
            device,
            ..FaultFields::default()
        },
    }))
}

/// `EXT4-fs error (device sda1): ...`, `XFS (dm-0): ... error/corruption`,
/// `BTRFS error (device sda1): ...` and `BTRFS critical (device ...)`.
fn filesystem_error(line: &str) -> Option<Parsed> {
    let device_in = |text: &str| {
        let start = text.find("(device ")? + "(device ".len();
        let end = text[start..].find(')')? + start;
        Some(text[start..end].to_string())
    };
    let (filesystem, device) = if line.starts_with("EXT4-fs error") {
        ("ext4", device_in(line))
    } else if line.starts_with("BTRFS error") || line.starts_with("BTRFS critical") {
        ("btrfs", device_in(line))
    } else if let Some(rest) = line.strip_prefix("XFS (") {
        let lower = line.to_ascii_lowercase();
        if !(lower.contains("error") || lower.contains("corruption")) {
            return None;
        }
        ("xfs", rest.split(')').next().map(str::to_string))
    } else {
        return None;
    };
    Some(Parsed::Fault(KernelFault {
        kind: FaultKind::FilesystemError,
        fields: FaultFields {
            device: device.filter(|device| !device.is_empty()),
            filesystem: Some(filesystem.into()),
            ..FaultFields::default()
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fault(line: &str) -> KernelFault {
        match parse_line(line) {
            Parsed::Fault(fault) => fault,
            other => panic!("{line:?} parsed as {other:?}"),
        }
    }

    #[test]
    fn a_rustc_segfault_on_amedes() {
        let found = fault("rustc[2772395]: segfault at 8 ip 00007f1a79ccf8ee sp 00007f1a79bb06b0 error 4 in libgcc_s.so.1[158ee,7f1a79cbd000+17000] likely on CPU 1 (core 1, socket 0)");
        assert_eq!(found.kind, FaultKind::Segfault);
        let f = found.fields;
        assert_eq!(f.process.as_deref(), Some("rustc"));
        assert_eq!(f.pid, Some(2_772_395));
        assert_eq!(f.object.as_deref(), Some("libgcc_s.so.1"));
        assert_eq!(f.ip, Some(0x7f1a_79cc_f8ee));
        assert_eq!(f.address, Some(8));
        assert_eq!(f.error_code.as_deref(), Some("4"));
        assert_eq!((f.cpu, f.core, f.socket), (Some(1), Some(1), Some(0)));
    }

    #[test]
    fn a_poet_test_thread_segfault_names_the_binary_and_cpu_20() {
        let f = fault("db::materialize[3588289]: segfault at 1000160 ip 000055704a098657 sp 00007f9ab5bf8160 error 4 in poet-bc7834abebfed280[7e96657,557043e76000+63ea000] likely on CPU 20 (core 4, socket 0)").fields;
        assert_eq!(f.process.as_deref(), Some("db::materialize"));
        assert_eq!(f.pid, Some(3_588_289));
        assert_eq!(f.object.as_deref(), Some("poet-bc7834abebfed280"));
        assert_eq!(f.address, Some(0x0100_0160));
        assert_eq!((f.cpu, f.core, f.socket), (Some(20), Some(4), Some(0)));
    }

    #[test]
    fn a_process_name_with_a_space() {
        let f = fault("lto cgu.04[99]: segfault at fffffffffdffdff0 ip 00007f89b87dae6f sp 00007f89ac97afa0 error 5 in libLLVM.so.20.1-rust-1.90.0-stable[87dae6f,7f89b7c00000+1f8c000] likely on CPU 17 (core 1, socket 0)").fields;
        assert_eq!(f.process.as_deref(), Some("lto cgu.04"));
        assert_eq!(f.address, Some(0xffff_ffff_fdff_dff0));
        assert_eq!(f.error_code.as_deref(), Some("5"));
        assert_eq!(
            f.object.as_deref(),
            Some("libLLVM.so.20.1-rust-1.90.0-stable")
        );
    }

    #[test]
    fn a_general_protection_fault_in_libllvm() {
        let found = fault("traps: opt cgu.00[848121] general protection fault ip:7f47836c84cb sp:7f463f1f7d70 error:0 in libLLVM.so.20.1-rust-1.90.0-stable[7ec84cb,7f4783400000+1f8c000]");
        assert_eq!(found.kind, FaultKind::GeneralProtection);
        let f = found.fields;
        assert_eq!(f.process.as_deref(), Some("opt cgu.00"));
        assert_eq!(f.pid, Some(848_121));
        assert_eq!(f.ip, Some(0x7f47_836c_84cb));
        assert_eq!(f.error_code.as_deref(), Some("0"));
        assert_eq!(
            f.object.as_deref(),
            Some("libLLVM.so.20.1-rust-1.90.0-stable")
        );
        assert_eq!(f.cpu, None);
    }

    #[test]
    fn an_invalid_opcode_trap() {
        let found = fault("traps: foo[12] trap invalid opcode ip:55d0c0a0 sp:7ffd0e00 error:0 in foo[55d0c000+1000] likely on CPU 3 (core 3, socket 0)");
        assert_eq!(found.kind, FaultKind::Trap);
        assert_eq!(found.fields.trap.as_deref(), Some("invalid opcode"));
        assert_eq!(found.fields.cpu, Some(3));
    }

    #[test]
    fn an_arm64_fault_is_a_segfault() {
        let found = fault("foo[12]: unhandled level 2 translation fault (11) at 0x0000000000000008, esr 0x0000000092000006, in libc.so.6[ffff80000000+190000]");
        assert_eq!(found.kind, FaultKind::Segfault);
        assert_eq!(found.fields.process.as_deref(), Some("foo"));
        assert_eq!(found.fields.address, Some(8));
        assert_eq!(
            found.fields.error_code.as_deref(),
            Some("0x0000000092000006")
        );
        assert_eq!(found.fields.object.as_deref(), Some("libc.so.6"));
    }

    #[test]
    fn oom_kills_cgroup_and_global() {
        let found = fault("Memory cgroup out of memory: Killed process 4242 (opt bkqg2il8o6q) total-vm:1000kB, anon-rss:900kB, file-rss:0kB, shmem-rss:0kB, UID:1000 pgtables:48kB oom_score_adj:0");
        assert_eq!(found.kind, FaultKind::OomKill);
        assert_eq!(found.fields.process.as_deref(), Some("opt bkqg2il8o6q"));
        assert_eq!(found.fields.pid, Some(4242));
        assert_eq!(found.fields.oom_scope.as_deref(), Some("cgroup"));
        let global = fault("Out of memory: Killed process 7 (java) total-vm:1kB, anon-rss:1kB");
        assert_eq!(global.fields.oom_scope.as_deref(), Some("global"));
        // Not a kill: the victim was already exiting.
        assert_eq!(
            parse_line("Memory cgroup out of memory: OOM victim 9 (gdb) is already exiting. Skip killing the task"),
            Parsed::NotAFault
        );
    }

    #[test]
    fn machine_checks_but_not_their_boot_notices() {
        assert_eq!(
            fault("mce: [Hardware Error]: Machine check events logged").kind,
            FaultKind::MachineCheck
        );
        let cpu = fault("mce: [Hardware Error]: CPU 5: Machine Check: 0 Bank 5: bea0000000000108");
        assert_eq!(cpu.fields.cpu, Some(5));
        assert_eq!(
            fault("EDAC MC0: 1 CE on mc#0csrow#0channel#0 (csrow:0 channel:0 page:0x0 offset:0x0 grain:8)").kind,
            FaultKind::MachineCheck
        );
        for notice in [
            "mce: CPU0: Thermal monitoring enabled (TM1)",
            "EDAC MC: Ver: 3.0.0",
            "EDAC amd64: Node 0: DRAM ECC disabled.",
            "MCE: In-kernel MCE decoding enabled.",
        ] {
            assert_eq!(parse_line(notice), Parsed::NotAFault, "{notice}");
        }
    }

    #[test]
    fn io_and_filesystem_errors() {
        let io = fault(
            "I/O error, dev sda, sector 2048 op 0x0:(READ) flags 0x0 phys_seg 1 prio class 2",
        );
        assert_eq!(io.kind, FaultKind::IoError);
        assert_eq!(io.fields.device.as_deref(), Some("sda"));
        let buffer = fault("Buffer I/O error on dev sda1, logical block 0, async page read");
        assert_eq!(buffer.fields.device.as_deref(), Some("sda1"));
        let ext4 = fault("EXT4-fs error (device mmcblk0p2): ext4_find_entry:1455: inode #2: comm ls: reading directory lblock 0");
        assert_eq!(ext4.kind, FaultKind::FilesystemError);
        assert_eq!(ext4.fields.filesystem.as_deref(), Some("ext4"));
        assert_eq!(ext4.fields.device.as_deref(), Some("mmcblk0p2"));
        let xfs = fault("XFS (dm-0): Corruption detected. Unmount and run xfs_repair");
        assert_eq!(xfs.fields.device.as_deref(), Some("dm-0"));
        let btrfs = fault(
            "BTRFS error (device sda1): bdev /dev/sda1 errs: wr 0, rd 1, flush 0, corrupt 0, gen 0",
        );
        assert_eq!(btrfs.fields.filesystem.as_deref(), Some("btrfs"));
        assert_eq!(
            parse_line("XFS (sda1): Mounting V5 Filesystem"),
            Parsed::NotAFault
        );
        assert_eq!(
            parse_line("EXT4-fs (sda1): mounted filesystem with ordered data mode"),
            Parsed::NotAFault
        );
    }

    #[test]
    fn malformed_lines_are_counted_never_panicked_on() {
        assert_eq!(
            parse_line("rustc: segfault at 8 ip 0 sp 0 error 4"),
            Parsed::Malformed(FaultKind::Segfault)
        );
        assert_eq!(
            parse_line("x[notapid]: segfault at 8"),
            Parsed::Malformed(FaultKind::Segfault)
        );
        assert_eq!(
            parse_line("traps: something odd happened"),
            Parsed::Malformed(FaultKind::Trap)
        );
        assert_eq!(
            parse_line("traps: [12] general protection fault ip:1"),
            Parsed::Malformed(FaultKind::GeneralProtection)
        );
        assert_eq!(
            parse_line("Out of memory: Killed process abc"),
            Parsed::Malformed(FaultKind::OomKill)
        );
        assert_eq!(
            parse_line("blk: I/O error somewhere"),
            Parsed::Malformed(FaultKind::IoError)
        );
        // Truncated and garbage lines never panic.
        for line in [
            "",
            "]: segfault at ",
            "[: segfault at ",
            "a[1]: segfault at",
            "a[1]: segfault at zz ip zz sp zz error",
            "a[1]: segfault at 1 ip 2 sp 3 error 4 in [",
            "a[1]: segfault at 1 ip 2 sp 3 error 4 in x] likely on CPU ",
            "a[1]: segfault at 1 likely on CPU 99999999999 (core x, socket",
            "traps: ",
            "traps: a[1] trap ",
            "x[1]: unhandled fault",
            "out of memory: Killed process ",
            "Out of memory: Killed process 1 (",
            "EXT4-fs error (device ",
            "XFS (",
            "mce: [Hardware Error]: CPU",
            "\u{fffd}\u{0}[\u{1}]: segfault at \u{7f}",
        ] {
            let _parsed = parse_line(line);
        }
        // A plain line is not a fault.
        assert_eq!(
            parse_line("usb 1-1: new high-speed USB device number 2 using xhci_hcd"),
            Parsed::NotAFault
        );
    }
}
