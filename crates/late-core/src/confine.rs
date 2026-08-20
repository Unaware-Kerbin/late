//! Confine operator-chosen filesystem paths to home / Late data.
//! Reject `..` before resolution so `/home/me/../../etc/passwd` cannot sneak through.

use crate::config::LatePaths;
use crate::error::{LateError, Result};
use std::path::{Component, Path, PathBuf};

const BLOCKED_IDENTITY_NAMES: &[&str] = &["sidecar.token", "secrets.json", "provider-keys.json"];

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

/// Identity files live under the operator home (`~/keys`, `~/.ssh`, …).
/// Relative paths are resolved as `~/.ssh/<name>`. Late config/data are excluded.
pub fn operator_home_roots() -> Vec<PathBuf> {
    dirs::home_dir().into_iter().collect()
}

fn blocked_identity_basename(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| BLOCKED_IDENTITY_NAMES.iter().any(|b| *b == n))
        .unwrap_or(false)
}

fn under_late_paths(path: &Path, late: &LatePaths) -> bool {
    for root in [&late.config, &late.data] {
        let root = if root.exists() {
            root.canonicalize().unwrap_or_else(|_| root.clone())
        } else {
            continue;
        };
        if path.starts_with(&root) {
            return true;
        }
    }
    false
}

pub fn confine_identity_path(key: &str) -> Result<PathBuf> {
    confine_identity_path_in(key, &LatePaths::discover())
}

pub(crate) fn confine_identity_path_in(key: &str, late: &LatePaths) -> Result<PathBuf> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err(LateError::Config("empty identity file".into()));
    }
    let given = PathBuf::from(trimmed);
    if blocked_identity_basename(&given) {
        return Err(LateError::Config("identity file name rejected".into()));
    }
    let path = if given.is_absolute() {
        given
    } else {
        let home = dirs::home_dir().ok_or_else(|| {
            LateError::Config("home directory required for relative identity files".into())
        })?;
        home.join(".ssh").join(given)
    };
    if blocked_identity_basename(&path) {
        return Err(LateError::Config("identity file name rejected".into()));
    }
    let confined = confine_under_roots(&path, &operator_home_roots(), true)?;
    if blocked_identity_basename(&confined) || under_late_paths(&confined, late) {
        return Err(LateError::Config(
            "identity file must not be a Late secret or live under Late config/data".into(),
        ));
    }
    Ok(confined)
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
    fn slash_is_not_under_operator_roots() {
        let home = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let roots = [home.path().to_path_buf(), data.path().to_path_buf()];
        assert!(confine_under_roots(Path::new("/"), &roots, false).is_err());
        assert!(confine_dir(Path::new("/"), &roots).is_err());
        assert!(confine_under_roots(Path::new("/tmp"), &roots, false).is_err());
    }

    #[test]
    fn read_requires_file_inside_root() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("cap.pcap");
        std::fs::write(&file, b"hi").unwrap();
        let got = confine_under_roots(&file, &[root.path().to_path_buf()], true).unwrap();
        assert_eq!(got, file.canonicalize().unwrap());
        let roots = operator_home_roots();
        if !roots.is_empty() {
            assert!(confine_under_roots(Path::new("/etc/passwd"), &roots, true).is_err());
            assert!(confine_identity_path("/etc/passwd").is_err());
            assert!(confine_identity_path("../.ssh/id_ed25519").is_err());
        }

        assert!(confine_under_roots(
            &root.path().join("missing.pcap"),
            &[root.path().to_path_buf()],
            true
        )
        .is_err());
    }

    #[test]
    fn identity_rejects_late_dirs_and_secret_names() {
        assert!(confine_identity_path("sidecar.token").is_err());
        assert!(confine_identity_path("secrets.json").is_err());
        assert!(confine_identity_path("provider-keys.json").is_err());
        assert!(blocked_identity_basename(Path::new(
            "/home/me/keys/secrets.json"
        )));
        assert!(!blocked_identity_basename(Path::new(
            "/home/me/keys/id_ed25519"
        )));

        let cfg = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let late = LatePaths {
            config: cfg.path().to_path_buf(),
            data: data.path().to_path_buf(),
        };
        let bait = cfg.path().join("id_ed25519");
        std::fs::write(&bait, b"ssh-key").unwrap();
        assert!(under_late_paths(&bait.canonicalize().unwrap(), &late));

        let elsewhere = tempfile::tempdir().unwrap();
        let key = elsewhere.path().join("id_ed25519");
        std::fs::write(&key, b"ssh-key").unwrap();
        assert!(!under_late_paths(&key.canonicalize().unwrap(), &late));

        // Home confinement still applies; a Late-dir key is rejected even if
        // the file exists under a fake config root that happens to be under home.
        if let Some(home) = dirs::home_dir() {
            if let Ok(home_cfg) = tempfile::TempDir::new_in(&home) {
                let bait = home_cfg.path().join("id_ed25519");
                std::fs::write(&bait, b"ssh-key").unwrap();
                let late_home = LatePaths {
                    config: home_cfg.path().to_path_buf(),
                    data: data.path().to_path_buf(),
                };
                assert!(confine_identity_path_in(bait.to_str().unwrap(), &late_home).is_err());
            }
        }
    }
}
