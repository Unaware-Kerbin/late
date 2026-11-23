//! RDP and VNC to a VM or a lab node.
//! A jump host, when set, is a loopback SSH forward. The viewer never sees a public bind.
//! Passwords stay in a mode 0600 profile file, not on the command line.

use crate::error::{LateError, Result};
use crate::ssh::{validate_jump, validate_scan_host};
use serde::Serialize;
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;
use zeroize::Zeroize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopProto {
    Rdp,
    Vnc,
}

impl DesktopProto {
    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "rdp" => Ok(Self::Rdp),
            "vnc" => Ok(Self::Vnc),
            other => Err(LateError::Message(format!("desktop protocol must be rdp or vnc, got {other}"))),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Rdp => "rdp",
            Self::Vnc => "vnc",
        }
    }

    fn default_port(self) -> u16 {
        match self {
            Self::Rdp => 3389,
            Self::Vnc => 5900,
        }
    }
}

pub struct DesktopRequest {
    pub protocol: DesktopProto,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: Option<String>,
    pub jump: Option<String>,
    pub name: String,
}

impl Drop for DesktopRequest {
    fn drop(&mut self) {
        if let Some(ref mut pw) = self.password {
            pw.zeroize();
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct DesktopInfo {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub server: String,
    pub target: String,
    pub tunneled: bool,
}

struct LiveDesktop {
    info: DesktopInfo,
    key: String,
    child: Option<Child>,
}

#[derive(Clone)]
pub struct Tunnels {
    inner: Arc<Mutex<HashMap<String, LiveDesktop>>>,
}

impl Tunnels {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn open(&self, mut req: DesktopRequest) -> Result<DesktopInfo> {
        if !validate_scan_host(&req.host) {
            return Err(LateError::Message("desktop host must be a hostname or IP".into()));
        }
        if req.port == 0 {
            req.port = req.protocol.default_port();
        }
        if req.port == 0 {
            return Err(LateError::Message("port is required".into()));
        }
        let jump = req
            .jump
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        if let Some(ref j) = jump {
            validate_jump(j)?;
        }
        let key = format!(
            "{}|{}:{}|{}",
            req.protocol.as_str(),
            req.host,
            req.port,
            jump.as_deref().unwrap_or("")
        );
        {
            let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(existing) = guard.values().find(|d| d.key == key) {
                return Ok(existing.info.clone());
            }
        }
        let (server, child) = if let Some(ref j) = jump {
            let local = free_loopback_port()?;
            let spec = forward_spec(local, &req.host, req.port)?;
            let mut cmd = Command::new("ssh");
            cmd.args([
                "-N",
                "-o",
                "ExitOnForwardFailure=yes",
                "-o",
                "BatchMode=yes",
                "-o",
                "StrictHostKeyChecking=accept-new",
                "-o",
                "UpdateHostKeys=no",
                "-o",
                "ConnectTimeout=8",
                "-L",
                &spec,
                j,
            ]);
            cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
            let mut child = cmd.spawn().map_err(|e| LateError::Message(format!("ssh: {e}")))?;
            if !wait_local(local, &mut child) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(LateError::Ssh(
                    "the lab host did not open the desktop forward".into(),
                ));
            }
            (format!("127.0.0.1:{local}"), Some(child))
        } else {
            (format!("{}:{}", req.host, req.port), None)
        };
        let password = req.password.take();
        launch_viewer(req.protocol, &server, &req.username, password.as_deref(), &req.name)?;
        if let Some(mut pw) = password {
            pw.zeroize();
        }
        let info = DesktopInfo {
            id: Uuid::new_v4().to_string(),
            name: if req.name.trim().is_empty() {
                req.host.clone()
            } else {
                req.name.trim().to_string()
            },
            protocol: req.protocol.as_str().to_string(),
            server,
            target: format!("{}:{}", req.host, req.port),
            tunneled: jump.is_some(),
        };
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                info.id.clone(),
                LiveDesktop {
                    info: info.clone(),
                    key,
                    child,
                },
            );
        Ok(info)
    }

    pub fn list(&self) -> Vec<DesktopInfo> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .map(|d| d.info.clone())
            .collect()
    }

    pub fn close(&self, id: &str) -> Result<()> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut live) = guard.remove(id) else {
            return Err(LateError::Message("desktop session not found".into()));
        };
        if let Some(child) = live.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        Ok(())
    }
}

pub fn forward_spec(local: u16, host: &str, port: u16) -> Result<String> {
    if !validate_scan_host(host) {
        return Err(LateError::Message("desktop host must be a hostname or IP".into()));
    }
    if local == 0 || port == 0 {
        return Err(LateError::Message("forward ports must be set".into()));
    }
    if host == "0.0.0.0" || host == "::" || host == "*" {
        return Err(LateError::Message("desktop forward stays on loopback".into()));
    }
    Ok(format!("127.0.0.1:{local}:{host}:{port}"))
}

pub fn profile_contents(protocol: DesktopProto, server: &str, username: &str, password: Option<&str>) -> String {
    let proto = match protocol {
        DesktopProto::Rdp => "RDP",
        DesktopProto::Vnc => "VNC",
    };
    let mut body = format!(
        "[remmina]\nprotocol={proto}\nserver={server}\nname=Late\ngroup=Late\ndisablepasswordstoring=1\n"
    );
    if !username.is_empty() {
        body.push_str(&format!("username={username}\n"));
    }
    if let Some(pw) = password.filter(|s| !s.is_empty()) {
        body.push_str(&format!("password={pw}\n"));
    }
    body
}

pub fn viewer_args(profile: &std::path::Path) -> Vec<String> {
    match std::env::consts::OS {
        "linux" => vec!["remmina".into(), "-c".into(), profile.display().to_string()],
        "windows" => vec!["mstsc".into(), profile.display().to_string()],
        _ => vec!["open".into(), profile.display().to_string()],
    }
}

fn launch_viewer(
    protocol: DesktopProto,
    server: &str,
    username: &str,
    password: Option<&str>,
    name: &str,
) -> Result<()> {
    match std::env::consts::OS {
        "linux" => launch_remmina(protocol, server, username, password),
        "macos" => launch_mac(protocol, server, username, password, name),
        "windows" => launch_windows(protocol, server, username),
        other => Err(LateError::Message(format!(
            "RDP and VNC are not set up for {other}"
        ))),
    }
}

fn launch_remmina(protocol: DesktopProto, server: &str, username: &str, password: Option<&str>) -> Result<()> {
    if Command::new("remmina").arg("--version").output().is_err() {
        return Err(LateError::Message(
            "Install Remmina on your computer to open RDP and VNC.".into(),
        ));
    }
    let path = write_profile(&profile_contents(protocol, server, username, password), "remmina")?;
    let mut child = Command::new("remmina")
        .arg("-c")
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| LateError::Message(format!("remmina: {e}")))?;
    shred_later(path);
    thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

fn launch_mac(protocol: DesktopProto, server: &str, username: &str, password: Option<&str>, name: &str) -> Result<()> {
    let _ = password;
    if protocol == DesktopProto::Vnc {
        let url = format!("vnc://{server}");
        Command::new("open")
            .arg(&url)
            .stdin(Stdio::null())
            .spawn()
            .map_err(|e| LateError::Message(format!("open: {e}")))?;
        return Ok(());
    }
    let path = write_profile(&rdp_file(server, username, name), "rdp")?;
    Command::new("open")
        .arg(&path)
        .stdin(Stdio::null())
        .spawn()
        .map_err(|e| LateError::Message(format!("open: {e}")))?;
    shred_later(path);
    Ok(())
}

fn launch_windows(protocol: DesktopProto, server: &str, username: &str) -> Result<()> {
    if protocol == DesktopProto::Vnc {
        return Err(LateError::Message(
            "Install a VNC viewer on your computer. RDP uses the built-in client.".into(),
        ));
    }
    let path = write_profile(&rdp_file(server, username, "Late"), "rdp")?;
    Command::new("mstsc")
        .arg(&path)
        .stdin(Stdio::null())
        .spawn()
        .map_err(|e| LateError::Message(format!("mstsc: {e}")))?;
    shred_later(path);
    Ok(())
}

fn rdp_file(server: &str, username: &str, name: &str) -> String {
    format!(
        "full address:s:{server}\nusername:s:{username}\nprompt for credentials:i:1\ndesktop name:s:{name}\n"
    )
}

fn write_profile(body: &str, ext: &str) -> Result<PathBuf> {
    let dir = std::env::temp_dir().join("late-desktop");
    fs::create_dir_all(&dir).map_err(|e| LateError::Message(e.to_string()))?;
    let path = dir.join(format!("{}.{}", Uuid::new_v4(), ext));
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| LateError::Message(e.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = f.set_permissions(fs::Permissions::from_mode(0o600));
    }
    f.write_all(body.as_bytes())
        .map_err(|e| LateError::Message(e.to_string()))?;
    Ok(path)
}

fn shred_later(path: PathBuf) {
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(2));
        if let Ok(meta) = fs::metadata(&path) {
            if let Ok(mut f) = OpenOptions::new().write(true).open(&path) {
                let zeros = vec![0u8; meta.len().min(1_048_576) as usize];
                let _ = f.write_all(&zeros);
            }
        }
        let _ = fs::remove_file(&path);
    });
}

fn free_loopback_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| LateError::Message(format!("could not reserve a local port: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| LateError::Message(e.to_string()))?
        .port();
    drop(listener);
    Ok(port)
}

fn wait_local(port: u16, child: &mut Child) -> bool {
    let start = Instant::now();
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    while start.elapsed() < Duration::from_secs(8) {
        if child.try_wait().ok().flatten().is_some() {
            return false;
        }
        if TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok() {
            return true;
        }
        thread::sleep(Duration::from_millis(80));
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_stays_on_loopback() {
        let spec = forward_spec(2345, "172.20.20.2", 3389).unwrap();
        assert_eq!(spec, "127.0.0.1:2345:172.20.20.2:3389");
        assert!(forward_spec(2345, "0.0.0.0", 3389).is_err());
    }

    #[test]
    fn profile_holds_password_but_argv_does_not() {
        let body = profile_contents(DesktopProto::Rdp, "127.0.0.1:3389", "admin", Some("s3cret"));
        assert!(body.contains("password=s3cret"));
        assert!(body.contains("protocol=RDP"));
        let path = std::path::Path::new("/tmp/late-desktop/x.remmina");
        let args = viewer_args(path);
        assert!(args.iter().all(|a| !a.contains("s3cret")));
        assert!(args.iter().any(|a| a.contains("x.remmina")));
        let empty = profile_contents(DesktopProto::Vnc, "127.0.0.1:5900", "", None);
        assert!(!empty.contains("password="));
        assert!(!empty.contains("username="));
    }
}
