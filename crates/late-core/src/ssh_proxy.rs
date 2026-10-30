//! How an SSH session reaches a host that is not directly routable.
//!
//! Containerlab management addresses sit behind the lab host. Ásbrú used a
//! local SOCKS tunnel (or its own jump). Late uses the same tunnel when the
//! launcher still has it open, and OpenSSH `ProxyJump` when the device's
//! jump host is set and no tunnel was passed in.

use crate::error::{LateError, Result};
use std::path::Path;
use std::sync::OnceLock;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SshProxy {
    Direct,
    /// `nc` SOCKS5 to a tunnel the launcher already authenticated.
    Socks { port: u16 },
    /// OpenSSH control socket the launcher already authenticated.
    Control { path: String, jump: String },
    /// `ssh -J`. The jump password, if any, is typed in the terminal.
    Jump { spec: String },
}

impl SshProxy {
    pub fn is_direct(&self) -> bool {
        matches!(self, Self::Direct)
    }

    /// Jump authentication already happened (SOCKS or ControlMaster).
    /// Device-password askpass is safe; it will not be fed to the jump prompt.
    pub fn is_preauthenticated(&self) -> bool {
        matches!(self, Self::Socks { .. } | Self::Control { .. })
    }

    pub fn cache_tag(&self) -> String {
        match self {
            Self::Direct => "direct".into(),
            Self::Socks { port } => format!("socks:{port}"),
            Self::Control { path, jump } => format!("cm:{jump}:{path}"),
            Self::Jump { spec } => format!("jump:{spec}"),
        }
    }

    /// Extra argv for `ssh`. Empty for a direct connection.
    /// Proxy forms pass `-F` null so the user's ssh_config cannot add a second jump.
    pub fn ssh_args(&self) -> Vec<String> {
        let null = null_ssh_config();
        match self {
            Self::Direct => Vec::new(),
            Self::Socks { port } => vec![
                "-F".into(),
                null.into(),
                "-o".into(),
                format!("ProxyCommand=nc -X 5 -x 127.0.0.1:{port} %h %p"),
            ],
            Self::Control { path, jump } => vec![
                "-F".into(),
                null.into(),
                "-o".into(),
                format!(
                    "ProxyCommand=ssh -F {null} -o ControlPath={path} -o ControlMaster=no -W %h:%p {jump}"
                ),
            ],
            Self::Jump { spec } => vec!["-F".into(), null.into(), "-J".into(), spec.clone()],
        }
    }
}

fn null_ssh_config() -> &'static str {
    if cfg!(windows) { "NUL" } else { "/dev/null" }
}

/// `true` when `nc` advertises SOCKS (`-X` / "socks"). Cached for the process.
pub fn nc_supports_socks() -> bool {
    static FLAG: OnceLock<bool> = OnceLock::new();
    *FLAG.get_or_init(|| {
        let Ok(out) = std::process::Command::new("nc").arg("-h").output() else {
            return false;
        };
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let lower = text.to_ascii_lowercase();
        text.contains("-X") || lower.contains("socks")
    })
}

/// Prefer an already-open SOCKS tunnel, then a control socket, then `-J`.
pub fn resolve_ssh_proxy(
    jump: Option<&str>,
    socks_port: Option<u16>,
    control_path: Option<&str>,
    socks_usable: bool,
) -> Result<SshProxy> {
    let jump = jump.map(str::trim).filter(|s| !s.is_empty());
    let control = control_path.map(str::trim).filter(|s| !s.is_empty());
    if let Some(port) = socks_port.filter(|p| *p > 0) {
        if socks_usable {
            return Ok(SshProxy::Socks { port });
        }
        if control.is_none() && jump.is_none() {
            return Err(LateError::Ssh(
                "SOCKS jump needs nc with SOCKS support, or an OpenSSH control socket".into(),
            ));
        }
    }
    if let Some(path) = control {
        let jump = jump.ok_or_else(|| {
            LateError::Ssh("control socket needs a jump host (user@host)".into())
        })?;
        let path = validate_control_path(path)?;
        let jump = validate_jump_spec(jump)?;
        if !Path::new(&path).exists() {
            return Err(LateError::Ssh(format!("jump tunnel is not up ({path})")));
        }
        return Ok(SshProxy::Control { path, jump });
    }
    if let Some(spec) = jump {
        return Ok(SshProxy::Jump {
            spec: validate_jump_spec(spec)?,
        });
    }
    Ok(SshProxy::Direct)
}

pub fn validate_jump_spec(raw: &str) -> Result<String> {
    let s = raw.trim();
    if s.is_empty() || s.len() > 253 || s.starts_with('-') || s.starts_with('@') || s.ends_with('@')
    {
        return Err(LateError::Ssh(
            "jump host must look like user@bastion or bastion".into(),
        ));
    }
    if s.matches('@').count() > 1 {
        return Err(LateError::Ssh(
            "jump host must look like user@bastion or bastion".into(),
        ));
    }
    if !s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '@' | ':'))
    {
        return Err(LateError::Ssh(
            "jump host must look like user@bastion or bastion (no spaces)".into(),
        ));
    }
    Ok(s.to_string())
}

fn validate_control_path(raw: &str) -> Result<String> {
    if raw.len() > 400 || !raw.starts_with('/') {
        return Err(LateError::Ssh(
            "control path must be an absolute path".into(),
        ));
    }
    let bad = raw.chars().any(|c| {
        c.is_whitespace()
            || c.is_control()
            || matches!(
                c,
                '"' | '\''
                    | ';'
                    | '&'
                    | '|'
                    | '$'
                    | '`'
                    | '\\'
                    | '<'
                    | '>'
                    | '('
                    | ')'
                    | '!'
                    | '*'
                    | '?'
                    | '{'
                    | '}'
                    | '['
                    | ']'
                    | '#'
                    | '~'
            )
    });
    if bad || raw.split('/').any(|p| p == "..") {
        return Err(LateError::Ssh(
            "control path has unsupported characters".into(),
        ));
    }
    Ok(raw.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socks_wins_over_jump_and_is_not_a_shell() {
        let proxy = resolve_ssh_proxy(Some("lab@192.0.2.10"), Some(1080), None, true).unwrap();
        assert_eq!(proxy, SshProxy::Socks { port: 1080 });
        let args = proxy.ssh_args();
        assert!(args.iter().any(|a| a == "ProxyCommand=nc -X 5 -x 127.0.0.1:1080 %h %p"));
        assert!(!args.iter().any(|a| a.contains(';')));
        assert!(proxy.is_preauthenticated());
    }

    #[test]
    fn control_socket_used_when_nc_cannot_socks() {
        let dir = std::env::temp_dir().join(format!("late-cm-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("control");
        std::fs::write(&path, b"").unwrap();
        let proxy = resolve_ssh_proxy(
            Some("lab@192.0.2.10"),
            Some(1080),
            Some(path.to_str().unwrap()),
            false,
        )
        .unwrap();
        match &proxy {
            SshProxy::Control { jump, path: got } => {
                assert_eq!(jump, "lab@192.0.2.10");
                assert_eq!(got, path.to_str().unwrap());
            }
            other => panic!("expected control, got {other:?}"),
        }
        let joined = proxy.ssh_args().join(" ");
        assert!(joined.contains("ControlMaster=no"));
        assert!(!joined.contains("1080"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn jump_spec_rejects_shell_metacharacters() {
        let err = resolve_ssh_proxy(Some("lab@host;rm -rf /"), None, None, false).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("jump host"), "{msg}");
        assert!(validate_jump_spec("user@lab.example").is_ok());
        assert!(validate_jump_spec("192.0.2.10").is_ok());
        assert!(validate_jump_spec("-oProxyCommand=evil").is_err());
    }

    #[test]
    fn missing_tunnel_is_an_error() {
        let err = resolve_ssh_proxy(
            Some("lab@192.0.2.10"),
            None,
            Some("/tmp/late-missing-control-socket"),
            false,
        )
        .unwrap_err();
        assert!(err.to_string().contains("not up"), "{err}");
    }

    #[test]
    fn direct_when_nothing_set() {
        let proxy = resolve_ssh_proxy(None, None, None, false).unwrap();
        assert!(proxy.is_direct());
        assert!(proxy.ssh_args().is_empty());
    }
}
