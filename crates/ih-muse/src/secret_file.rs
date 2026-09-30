//! Reading of small credential files (Poet tokens, Redis passwords) mounted beside a Muse.

use std::fmt;
use std::fs;
use std::os::unix::fs::PermissionsExt;

/// Largest credential file a Muse accepts.
pub const MAX_SECRET_FILE_BYTES: u64 = 4096;

/// Why a credential file was refused. The message never contains the secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretFileError {
    /// The path (after following symlinks) does not exist or cannot be inspected.
    Missing { label: String },
    /// The path resolves to something other than a regular file.
    NotRegular { label: String },
    /// The file is empty or larger than [`MAX_SECRET_FILE_BYTES`].
    Size { label: String, bytes: u64 },
    /// Other users (the "world" permission bits) may access the file.
    WorldAccessible { label: String, mode: u32 },
    /// The content is not UTF-8 or contains CR, LF or NUL.
    Content { label: String },
}

impl fmt::Display for SecretFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { label } => write!(f, "{label} file does not exist or is unreadable"),
            Self::NotRegular { label } => write!(f, "{label} file is not a regular file"),
            Self::Size { label, bytes } => write!(
                f,
                "{label} file must hold 1 to {MAX_SECRET_FILE_BYTES} bytes (has {bytes})"
            ),
            Self::WorldAccessible { label, mode } => write!(
                f,
                "{label} file must not be accessible by other users: mode is {mode:04o}, \
                 expected no world bits (for example 0400, 0440 or 0600)"
            ),
            Self::Content { label } => {
                write!(f, "{label} file must be UTF-8 without CR, LF or NUL bytes")
            }
        }
    }
}

impl std::error::Error for SecretFileError {}

/// Read a single-line credential from `path`.
///
/// Symlinks are followed (Kubernetes Secret volumes are symlinks into `..data`), and
/// the target must be a regular file. Owner and group bits are allowed because Secret
/// volumes are `root:<fsGroup>` with mode `0440`; only world bits (`mode & 0o007`) are
/// refused.
pub fn read_secret_file(path: &str, label: &str) -> Result<String, SecretFileError> {
    let label = label.to_owned();
    let metadata = fs::metadata(path).map_err(|_| SecretFileError::Missing {
        label: label.clone(),
    })?;
    if !metadata.is_file() {
        return Err(SecretFileError::NotRegular { label });
    }
    let bytes = metadata.len();
    if bytes == 0 || bytes > MAX_SECRET_FILE_BYTES {
        return Err(SecretFileError::Size { label, bytes });
    }
    let mode = metadata.permissions().mode() & 0o7777;
    if mode & 0o007 != 0 {
        return Err(SecretFileError::WorldAccessible { label, mode });
    }
    let secret = fs::read_to_string(path).map_err(|_| SecretFileError::Content {
        label: label.clone(),
    })?;
    if secret.contains(['\r', '\n', '\0']) {
        return Err(SecretFileError::Content { label });
    }
    Ok(secret)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn secret_with_mode(dir: &tempfile::TempDir, name: &str, mode: u32) -> String {
        let path = dir.path().join(name);
        fs::write(&path, "s3cret-token").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        path.to_str().unwrap().to_owned()
    }

    #[test]
    fn owner_and_group_readable_files_are_accepted() {
        let dir = tempfile::tempdir().unwrap();
        for mode in [0o400, 0o600, 0o440, 0o640] {
            let path = secret_with_mode(&dir, &format!("t{mode:o}"), mode);
            assert_eq!(
                read_secret_file(&path, "graph token").unwrap(),
                "s3cret-token",
                "mode {mode:o}"
            );
        }
    }

    #[test]
    fn world_accessible_files_are_refused_with_the_mode_and_no_secret() {
        let dir = tempfile::tempdir().unwrap();
        for mode in [0o444, 0o604, 0o602, 0o601] {
            let path = secret_with_mode(&dir, &format!("t{mode:o}"), mode);
            let error = read_secret_file(&path, "graph token").unwrap_err();
            assert_eq!(
                error,
                SecretFileError::WorldAccessible {
                    label: "graph token".into(),
                    mode
                }
            );
            let text = error.to_string();
            assert!(text.contains(&format!("{mode:04o}")), "{text}");
            assert!(!text.contains("s3cret"), "{text}");
        }
    }

    #[test]
    fn a_kubernetes_style_symlink_to_a_group_readable_file_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let target = secret_with_mode(&dir, "..data-token", 0o440);
        let link = dir.path().join("token");
        symlink(&target, &link).unwrap();
        assert_eq!(
            read_secret_file(link.to_str().unwrap(), "graph token").unwrap(),
            "s3cret-token"
        );
    }

    #[test]
    fn directories_missing_empty_oversized_and_multiline_files_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let label = "graph token".to_owned();
        assert_eq!(
            read_secret_file(dir.path().to_str().unwrap(), &label).unwrap_err(),
            SecretFileError::NotRegular {
                label: label.clone()
            }
        );
        let missing = dir.path().join("missing");
        assert_eq!(
            read_secret_file(missing.to_str().unwrap(), &label).unwrap_err(),
            SecretFileError::Missing {
                label: label.clone()
            }
        );
        let empty = dir.path().join("empty");
        fs::write(&empty, "").unwrap();
        fs::set_permissions(&empty, fs::Permissions::from_mode(0o400)).unwrap();
        assert!(matches!(
            read_secret_file(empty.to_str().unwrap(), &label),
            Err(SecretFileError::Size { bytes: 0, .. })
        ));
        let big = dir.path().join("big");
        fs::write(&big, "x".repeat(4097)).unwrap();
        fs::set_permissions(&big, fs::Permissions::from_mode(0o400)).unwrap();
        assert!(matches!(
            read_secret_file(big.to_str().unwrap(), &label),
            Err(SecretFileError::Size { bytes: 4097, .. })
        ));
        let multi = dir.path().join("multi");
        fs::write(&multi, "token\n").unwrap();
        fs::set_permissions(&multi, fs::Permissions::from_mode(0o400)).unwrap();
        assert_eq!(
            read_secret_file(multi.to_str().unwrap(), &label).unwrap_err(),
            SecretFileError::Content { label }
        );
    }
}
