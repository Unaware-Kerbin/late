//! Siemens Edgeshark (Ghostwire + Packetflix) lifecycle.
//! Operator-initiated only. Runs in its own compose project.

use crate::error::{LateError, Result};
use serde::{Deserialize, Serialize};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

pub const EDGESHARK_URL: &str = "http://127.0.0.1:5001";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgesharkStatus {
    pub running: bool,
    pub starting: bool,
    pub url: String,
    pub wireshark: bool,
    pub plugin: bool,
    pub detail: String,
}

#[derive(Default)]
struct Job {
    starting: bool,
    last_error: Option<String>,
}

fn job() -> &'static Mutex<Job> {
    static JOB: OnceLock<Mutex<Job>> = OnceLock::new();
    JOB.get_or_init(|| Mutex::new(Job::default()))
}

fn compose_marker(dir: &Path) -> bool {
    dir.join("docker/compose.edgeshark.yml").is_file()
}

fn repo_root() -> Result<PathBuf> {
    let mut cands = Vec::new();
    for key in ["LATE_ROOT", "LATE_ROOT"] {
        if let Ok(p) = std::env::var(key) {
            cands.push(PathBuf::from(p));
        }
    }
    if let Ok(here) = std::env::current_dir() {
        cands.push(here.clone());
        cands.extend(here.ancestors().map(Path::to_path_buf));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            cands.extend(dir.ancestors().map(Path::to_path_buf));
        }
    }
    for c in cands {
        if compose_marker(&c) {
            return Ok(c);
        }
    }
    Err(LateError::Message(
        "cannot find docker/compose.edgeshark.yml; start Late from the repo or set LATE_ROOT"
            .into(),
    ))
}

fn compose_file() -> Result<PathBuf> {
    let root = repo_root()?.canonicalize()?;
    let file = root.join("docker/compose.edgeshark.yml").canonicalize()?;
    if !file.starts_with(&root) {
        return Err(LateError::Message("compose path escaped repo root".into()));
    }
    Ok(file)
}

fn which(bin: &str) -> bool {
    Command::new("which")
        .arg(bin)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn plugin_installed() -> bool {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    [
        PathBuf::from("/usr/lib/x86_64-linux-gnu/wireshark/extcap/cshargextcap"),
        PathBuf::from("/usr/lib/wireshark/extcap/cshargextcap"),
        PathBuf::from("/usr/lib64/wireshark/extcap/cshargextcap"),
        home.join(".local/lib/wireshark/extcap/cshargextcap"),
        home.join(".config/wireshark/extcap/cshargextcap"),
    ]
    .iter()
    .any(|p| p.is_file())
}

fn port_up() -> bool {
    TcpStream::connect_timeout(
        &"127.0.0.1:5001".parse().unwrap(),
        Duration::from_millis(350),
    )
    .is_ok()
}

pub fn status() -> EdgesharkStatus {
    let j = job().lock().unwrap_or_else(|e| e.into_inner());
    let running = port_up();
    let mut detail = if running {
        "Edgeshark UI is listening on 127.0.0.1:5001".into()
    } else if j.starting {
        "starting Ghostwire + Packetflix…".into()
    } else {
        "stopped".into()
    };
    if let Some(err) = &j.last_error {
        detail = err.clone();
    }
    EdgesharkStatus {
        running,
        starting: j.starting,
        url: EDGESHARK_URL.into(),
        wireshark: which("wireshark") || which("wireshark-qt"),
        plugin: plugin_installed(),
        detail,
    }
}

pub fn start() -> Result<EdgesharkStatus> {
    let file = compose_file()?;
    {
        let mut j = job().lock().unwrap_or_else(|e| e.into_inner());
        j.starting = true;
        j.last_error = None;
    }
    let dir = file.parent().unwrap_or(Path::new(".")).to_path_buf();
    let out = Command::new("docker")
        .current_dir(&dir)
        .args([
            "compose",
            "-f",
            file.to_str().unwrap_or("compose.edgeshark.yml"),
            "-p",
            "late-edgeshark",
            "up",
            "-d",
            "--pull",
            "missing",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    {
        let mut j = job().lock().unwrap_or_else(|e| e.into_inner());
        j.starting = false;
        match &out {
            Ok(o) if o.status.success() => j.last_error = None,
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr);
                j.last_error = Some(format!(
                    "docker compose up failed: {}",
                    err.chars().take(500).collect::<String>()
                ));
            }
            Err(e) => j.last_error = Some(format!("docker compose: {e}")),
        }
    }
    if let Ok(o) = &out {
        if !o.status.success() {
            return Err(LateError::Message(
                job()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .last_error
                    .clone()
                    .unwrap_or_else(|| "docker compose up failed".into()),
            ));
        }
    }
    if let Err(e) = out {
        return Err(LateError::Message(format!("docker compose: {e}")));
    }
    for _ in 0..40 {
        if port_up() {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    let st = status();
    if !st.running {
        return Err(LateError::Message(
            st.detail.clone() + " (compose returned ok but :5001 is not listening yet)",
        ));
    }
    Ok(st)
}

pub fn stop() -> Result<EdgesharkStatus> {
    let file = compose_file()?;
    let dir = file.parent().unwrap_or(Path::new(".")).to_path_buf();
    let out = Command::new("docker")
        .current_dir(&dir)
        .args([
            "compose",
            "-f",
            file.to_str().unwrap_or("compose.edgeshark.yml"),
            "-p",
            "late-edgeshark",
            "down",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    let mut j = job().lock().unwrap_or_else(|e| e.into_inner());
    j.starting = false;
    match out {
        Ok(o) if o.status.success() => j.last_error = None,
        Ok(o) => {
            j.last_error = Some(
                String::from_utf8_lossy(&o.stderr)
                    .chars()
                    .take(400)
                    .collect(),
            );
        }
        Err(e) => j.last_error = Some(format!("docker compose: {e}")),
    }
    Ok(status())
}
