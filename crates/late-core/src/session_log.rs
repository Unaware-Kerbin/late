//! Resolve SecureCRT-style session log paths under a confined folder.

use crate::confine;
use crate::error::{LateError, Result};
use chrono::Local;
use std::path::{Path, PathBuf};

/// Whether a new interactive session should start logging.
///
/// Default is off. Logging starts when the device flag, the Settings default,
/// or an explicit connect (`logSession`) flag is true.
pub fn should_auto_log(device_log_session: bool, global_default: bool, open_flag: bool) -> bool {
    open_flag || device_log_session || global_default
}

/// Empty / unset Settings `log_dir` means Late data `logs/`.
pub fn effective_log_dir(log_dir: &Path, late_data: &Path) -> PathBuf {
    if log_dir.as_os_str().is_empty() {
        late_data.join("logs")
    } else {
        log_dir.to_path_buf()
    }
}

/// Sanitize a session/device display name, then run it through `safe_export_stem`.
///
/// Dots are dropped (not kept) so names like `../etc/passwd` cannot become a
/// leading-dot stem that collapses to `"session"` or escapes path checks.
pub fn safe_log_stem(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
            out.push(c);
        } else if c.is_whitespace() || matches!(c, '/' | '\\' | ':' | '@' | '.') {
            if !out.is_empty() && !out.ends_with('_') {
                out.push('_');
            }
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    let candidate: String = if out.is_empty() {
        "session".into()
    } else {
        out.chars().take(80).collect()
    };
    confine::safe_export_stem(&candidate).unwrap_or_else(|_| "session".into())
}

/// `{safe_stem}_{YYYYMMDD_HHMMSS}.log`
pub fn log_file_name(session_name: &str) -> String {
    let stem = safe_log_stem(session_name);
    let stamp = Local::now().format("%Y%m%d_%H%M%S");
    format!("{stem}_{stamp}.log")
}

/// Resolve a log file path under `log_dir` (must stay under home / Late dirs).
pub fn resolve_log_path(
    log_dir: &Path,
    session_name: &str,
    roots: &[PathBuf],
) -> Result<PathBuf> {
    if log_dir.as_os_str().is_empty() {
        return Err(LateError::Config("session log folder is empty".into()));
    }
    let dir = if log_dir.is_dir() {
        confine::confine_dir(log_dir, roots)?
    } else {
        // Allow a not-yet-created folder whose parent is confined.
        confine::confine_under_roots(log_dir, roots, false)?
    };
    let name = log_file_name(session_name);
    let stem = name.trim_end_matches(".log");
    let _ = confine::safe_export_stem(stem)?;
    Ok(dir.join(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn default_off_does_not_log() {
        assert!(!should_auto_log(false, false, false));
    }

    #[test]
    fn device_flag_or_global_or_open_enables() {
        assert!(should_auto_log(true, false, false));
        assert!(should_auto_log(false, true, false));
        assert!(should_auto_log(false, false, true));
        assert!(should_auto_log(true, true, true));
    }

    #[test]
    fn empty_log_dir_uses_late_data_logs() {
        let data = PathBuf::from("/home/me/.local/share/late");
        assert_eq!(
            effective_log_dir(Path::new(""), &data),
            data.join("logs")
        );
        assert_eq!(
            effective_log_dir(Path::new("/home/me/logs"), &data),
            PathBuf::from("/home/me/logs")
        );
    }

    #[test]
    fn safe_stem_reuses_export_rules() {
        assert_eq!(safe_log_stem("edge-sw1"), "edge-sw1");
        assert_eq!(safe_log_stem("NYC Core"), "NYC_Core");
        assert_eq!(safe_log_stem("../etc/passwd"), "etc_passwd");
        assert_eq!(safe_log_stem("..."), "session");
        assert_eq!(safe_log_stem(""), "session");
        assert!(confine::safe_export_stem(&safe_log_stem("lab switch")).is_ok());
    }

    #[test]
    fn file_name_is_stem_yyyymmdd_hhmmss() {
        let name = log_file_name("lab switch");
        assert!(name.starts_with("lab_switch_"));
        assert!(name.ends_with(".log"));
        let body = name.trim_end_matches(".log");
        let parts: Vec<_> = body.rsplitn(2, '_').collect();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].len(), 6); // HHMMSS
        let date_time: Vec<_> = body.split('_').collect();
        assert!(date_time.len() >= 3);
        let date = date_time[date_time.len() - 2];
        assert_eq!(date.len(), 8);
        assert!(date.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn resolve_stays_under_log_dir() {
        let tmp = tempdir().unwrap();
        let logs = tmp.path().join("logs");
        fs::create_dir_all(&logs).unwrap();
        let roots = vec![tmp.path().to_path_buf()];
        let path = resolve_log_path(&logs, "core", &roots).unwrap();
        assert!(path.starts_with(&logs));
        assert!(path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("core_"));
        assert!(path.extension().and_then(|e| e.to_str()) == Some("log"));
    }

    #[test]
    fn resolve_rejects_escape() {
        let tmp = tempdir().unwrap();
        let roots = vec![tmp.path().to_path_buf()];
        let evil = PathBuf::from("/tmp");
        assert!(resolve_log_path(&evil, "x", &roots).is_err());
    }

    #[test]
    fn device_or_global_chooses_path_under_log_dir() {
        let tmp = tempdir().unwrap();
        let logs = tmp.path().join("session-logs");
        fs::create_dir_all(&logs).unwrap();
        let roots = vec![tmp.path().to_path_buf()];
        assert!(!should_auto_log(false, false, false));
        assert!(should_auto_log(true, false, false));
        let path = resolve_log_path(&logs, "core-sw1", &roots).unwrap();
        assert!(path.starts_with(&logs));
        assert!(path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("core-sw1_"));
    }
}
