//! Confine operator-chosen filesystem paths to home / Late data.
//! Reject `..` before resolution so `/home/me/../../etc/passwd` cannot sneak through.

use crate::error::{LateError, Result};
use std::path::{Component, Path, PathBuf};

pub fn safe_export_stem(name: &str) -> Result<String> {
    let stem = name.trim();
    if stem.is_empty() || stem.len() > 120 {
        return Err(LateError::Config(
            "export name must be 1–120 characters".into(),
        ));
    }
    if stem == "." || stem == ".." || stem.starts_with('.') {
        return Err(LateError::Config("export name rejected".into()));
    }
    if !stem
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(LateError::Config(
            "export name may only contain letters, digits, '.', '-', '_'".into(),
        ));
    }
    Ok(stem.to_string())
}

/// Resolve `path` so it stays under one of `roots`.
///
/// When `file_must_exist` is true, the file is canonicalized (pcap/import).
/// When false, the parent directory must exist and the leaf is joined after
/// canonicalize (SFTP download destination).
pub fn confine_under_roots(
    path: &Path,
    roots: &[PathBuf],
    file_must_exist: bool,
) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        return Err(LateError::Config("empty path".into()));
    }
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(LateError::Config("path must not contain ..".into()));
    }
    if path.to_string_lossy().contains('\0') {
        return Err(LateError::Config("path rejected".into()));
    }
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let parent = abs
        .parent()
        .ok_or_else(|| LateError::Config("path has no parent directory".into()))?;
    if !parent.exists() {
        return Err(LateError::Config(format!(
            "directory not found: {}",
            parent.display()
        )));
    }
    let parent = parent.canonicalize()?;
    let name = abs
        .file_name()
        .ok_or_else(|| LateError::Config("path has no file name".into()))?;
    if name == "." || name == ".." {
        return Err(LateError::Config("path rejected".into()));
    }
    let dest = parent.join(name);
    if dest
        .symlink_metadata()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(LateError::Config(
            "refusing to write through a symbolic link".into(),
        ));
    }
    let check = if file_must_exist {
        if !dest.is_file() {
            return Err(LateError::Config(format!(
                "file not found: {}",
                dest.display()
            )));
        }
        dest.canonicalize()?
    } else {
        dest.clone()
    };
    ensure_under(&check, roots)?;
    Ok(if file_must_exist { check } else { dest })
}

pub fn confine_dir(path: &Path, roots: &[PathBuf]) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        return Err(LateError::Config("empty path".into()));
    }
    if path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(LateError::Config("path must not contain ..".into()));
    }
    if !path.is_dir() {
        return Err(LateError::Config(format!(
            "not a directory: {}",
            path.display()
        )));
    }
    let canon = path.canonicalize()?;
    ensure_under(&canon, roots)?;
    Ok(canon)
}

fn ensure_under(path: &Path, roots: &[PathBuf]) -> Result<()> {
    for root in roots {
        let root = if root.exists() {
            root.canonicalize().unwrap_or_else(|_| root.clone())
        } else {
            continue;
        };
        if path.starts_with(&root) {
            return Ok(());
        }
    }
    Err(LateError::Config(
        "path must be under your home directory or Late data directory".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_stem_rejects_traversal() {
        assert!(safe_export_stem("../etc/passwd").is_err());
        assert!(safe_export_stem("a/b").is_err());
        assert!(safe_export_stem("a\\b").is_err());
        assert!(safe_export_stem("..").is_err());
        assert!(safe_export_stem(".hidden").is_err());
        assert!(safe_export_stem("ok-name_1.log").is_ok());
        assert_eq!(safe_export_stem("session").unwrap(), "session");
    }

    #[test]
    fn write_stays_under_temp_root() {
        let root = tempfile::tempdir().unwrap();
        let dest = root.path().join("out.txt");
        let got = confine_under_roots(&dest, &[root.path().to_path_buf()], false).unwrap();
        assert_eq!(got.file_name().unwrap(), "out.txt");
        assert!(got.starts_with(root.path().canonicalize().unwrap()));

        let escape = root.path().join("..").join("etc").join("passwd");
        assert!(confine_under_roots(&escape, &[root.path().to_path_buf()], false).is_err());

        let outside = PathBuf::from("/etc/passwd");
        assert!(confine_under_roots(&outside, &[root.path().to_path_buf()], false).is_err());
    }

    #[test]
    fn read_requires_file_inside_root() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("cap.pcap");
        std::fs::write(&file, b"hi").unwrap();
        let got = confine_under_roots(&file, &[root.path().to_path_buf()], true).unwrap();
        assert_eq!(got, file.canonicalize().unwrap());
        assert!(confine_under_roots(
            &root.path().join("missing.pcap"),
            &[root.path().to_path_buf()],
            true
        )
        .is_err());
    }
}
