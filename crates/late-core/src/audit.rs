//! Append-only security event log. Never write secrets, tokens, or command bodies.

use crate::config::LatePaths;
use crate::error::Result;
use serde_json::json;
use std::fs::OpenOptions;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

const SKIP: &[&str] = &[
    "session.input",
    "session.write",
    "session.resize",
    "session.scrollback",
    "session.list",
    "pcap.packets",
    "pcap.query",
    "pcap.findings",
    "pcap.interfaces",
    "pcap.ifs",
    "pcap.nics",
    "serial.list",
    "serial.ports",
    "serial.enumerate",
    "inventory.list",
    "auth.list",
    "settings.get",
    "collections.list",
    "capture.list",
    "sftp.list",
    "sftp.ls",
    "sftp.local",
    "sftp.local_list",
    "fs.list",
];

pub fn should_log(method: &str) -> bool {
    let m = method.to_ascii_lowercase();
    !SKIP.iter().any(|s| m == *s)
}

pub fn append(paths: &LatePaths, method: &str, ok: bool) -> Result<()> {
    if !should_log(method) {
        return Ok(());
    }
    let dir = &paths.data;
    crate::fsutil::mkdir_private(dir)?;
    let path = dir.join("audit.jsonl");
    let mut opts = OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    opts.mode(0o600);
    let mut f = opts.open(&path)?;
    let line = json!({
        "ts": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "event": "rpc",
        "method": method,
        "ok": ok,
    });
    writeln!(f, "{line}")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_keystroke_noise() {
        assert!(!should_log("session.input"));
        assert!(!should_log("session.list"));
        assert!(should_log("session.open"));
        assert!(should_log("auth.upsert"));
        assert!(should_log("providers.set"));
        assert!(should_log("sftp.download"));
    }
}
