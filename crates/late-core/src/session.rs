use crate::capture::{self, CaptureStore, DiffLine};
use crate::collections;
use crate::config::{load_settings, save_settings, AppSettings, LatePaths};
use crate::error::{LateError, Result};
use crate::http_api::{self, ApiRequest, ApiResponse};
use crate::import::{self, ImportResult};
use crate::inventory::InventoryStore;
use crate::known_hosts::KnownHosts;
use crate::local_pty;
use crate::pcap::{self, CaptureInfo, LiveCapture};
use crate::policy::PolicyEngine;
use crate::providers::ProviderVault;
use crate::redact::Redactor;
use crate::secrets::SecretStore;
use crate::serial;
use crate::sftp::{self, SftpEntry};
use crate::ssh::{self, SshConnectOpts};
use crate::types::*;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use chrono::Utc;
use parking_lot::Mutex;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppEvent {
    pub event: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OpenSession {
    pub device_id: Option<String>,
    pub kind: SessionKind,
    pub accept_unknown_host: bool,
    pub replace_host_key: bool,
    pub cols: u32,
    pub rows: u32,
    pub shell: Option<String>,
    pub path: Option<PathBuf>,
    pub iface: Option<String>,
    pub bpf: Option<String>,
}

#[derive(Clone)]
pub struct App {
    pub paths: LatePaths,
    pub settings: Arc<Mutex<AppSettings>>,
    pub inventory: InventoryStore,
    pub secrets: SecretStore,
    pub providers: ProviderVault,
    pub policy: PolicyEngine,
    pub events: broadcast::Sender<AppEvent>,
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    sessions: HashMap<String, LiveSession>,
    known: KnownHosts,
    pcaps: HashMap<String, OpenPcap>,
    live_caps: HashMap<String, LiveCapture>,
}

struct LiveSession {
    info: SessionInfo,
    kind: SessionKind,
    vendor: Vendor,
    device_id: Option<String>,
    input: Option<mpsc::Sender<Vec<u8>>>,
    resize: Option<mpsc::Sender<(u32, u32)>>,
    close: Option<mpsc::Sender<()>>,
    output: broadcast::Sender<Vec<u8>>,
    scrollback: Vec<u8>,
    redactor: Redactor,
    logging_path: Option<PathBuf>,
    reconnect: Option<OpenSession>,
    serial_break: Option<mpsc::Sender<()>>,
}

struct OpenPcap {
    packets: Vec<PacketSummary>,
    findings: Vec<PcapFinding>,
    path: PathBuf,
    #[allow(dead_code)]
    total: usize,
    #[allow(dead_code)]
    truncated: bool,
}

impl App {
    pub fn boot() -> Result<Self> {
        let paths = LatePaths::discover();
        paths.ensure()?;
        seed_bundled_policies(&paths)?;
        let settings = load_settings(&paths.settings())?;
        let bundled = first_bundled_policy_dir();
        let mut policy = PolicyEngine::load_dir(&bundled)?;
        policy.merge_dir(&paths.config.join("policies"))?;
        let known = KnownHosts::load(&paths)?;
        let (events, _) = broadcast::channel(1024);
        Ok(Self {
            inventory: InventoryStore::new(paths.clone()),
            secrets: SecretStore::new(paths.clone()),
            providers: ProviderVault::open(paths.clone())?,
            paths: paths.clone(),
            settings: Arc::new(Mutex::new(settings)),
            policy,
            events,
            inner: Arc::new(Mutex::new(Inner {
                sessions: HashMap::new(),
                known,
                pcaps: HashMap::new(),
                live_caps: HashMap::new(),
            })),
        })
    }

    pub fn settings(&self) -> AppSettings {
        self.settings.lock().clone()
    }

    pub fn set_settings(&self, settings: AppSettings) -> Result<()> {
        save_settings(&self.paths.settings(), &settings)?;
        *self.settings.lock() = settings;
        Ok(())
    }

    pub fn set_provider_key(&self, name: &str, key: &str) -> Result<()> {
        self.providers.set(name, key)
    }

    pub fn delete_provider_key(&self, name: &str) -> Result<()> {
        self.providers.delete(name)
    }

    pub fn provider_status(&self) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(self.providers.status()?)?)
    }

    pub fn list_sessions(&self) -> Vec<SessionInfo> {
        self.inner
            .lock()
            .sessions
            .values()
            .map(|s| s.info.clone())
            .collect()
    }

    pub fn subscribe(&self, session_id: &str) -> Result<broadcast::Receiver<Vec<u8>>> {
        let inner = self.inner.lock();
        let s = inner
            .sessions
            .get(session_id)
            .ok_or_else(|| LateError::NotFound(session_id.into()))?;
        Ok(s.output.subscribe())
    }

    pub fn write(&self, session_id: &str, data: &[u8]) -> Result<()> {
        let inner = self.inner.lock();
        let s = inner
            .sessions
            .get(session_id)
            .ok_or_else(|| LateError::NotFound(session_id.into()))?;
        if let Some(tx) = &s.input {
            let _ = tx.try_send(data.to_vec());
        }
        Ok(())
    }

    pub fn resize(&self, session_id: &str, cols: u32, rows: u32) -> Result<()> {
        let inner = self.inner.lock();
        if let Some(s) = inner.sessions.get(session_id) {
            if let Some(tx) = &s.resize {
                let _ = tx.try_send((cols, rows));
            }
        }
        Ok(())
    }

    pub fn close_session(&self, session_id: &str) -> Result<()> {
        let mut inner = self.inner.lock();
        if let Some(s) = inner.sessions.remove(session_id) {
            if let Some(tx) = s.close {
                let _ = tx.try_send(());
            }
            let _ = self.events.send(AppEvent {
                event: "session.closed".into(),
                session_id: session_id.into(),
                data: None,
                reason: Some("closed".into()),
            });
        }
        Ok(())
    }

    pub fn redacted_scrollback(&self, session_id: &str) -> Result<String> {
        let mut inner = self.inner.lock();
        let s = inner
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| LateError::NotFound(session_id.into()))?;
        let raw = String::from_utf8_lossy(&s.scrollback).into_owned();
        Ok(s.redactor.redact(&raw))
    }

    pub fn raw_scrollback(&self, session_id: &str) -> Result<String> {
        let inner = self.inner.lock();
        let s = inner
            .sessions
            .get(session_id)
            .ok_or_else(|| LateError::NotFound(session_id.into()))?;
        Ok(String::from_utf8_lossy(&s.scrollback).into_owned())
    }

    pub fn check_command(&self, session_id: &str, command: &str) -> Result<PolicyDecision> {
        let inner = self.inner.lock();
        let s = inner
            .sessions
            .get(session_id)
            .ok_or_else(|| LateError::NotFound(session_id.into()))?;
        Ok(self.policy.check(s.vendor, command))
    }

    pub fn check_policy(&self, vendor: Vendor, command: &str) -> PolicyDecision {
        self.policy.check(vendor, command)
    }

    pub fn send_break(&self, session_id: &str) -> Result<()> {
        let inner = self.inner.lock();
        let s = inner
            .sessions
            .get(session_id)
            .ok_or_else(|| LateError::NotFound(session_id.into()))?;
        if s.kind != SessionKind::Serial {
            return Err(LateError::Message("not a serial session".into()));
        }
        let tx = s
            .serial_break
            .clone()
            .ok_or_else(|| LateError::Message("no serial break channel".into()))?;
        drop(inner);
        tx.try_send(())
            .map_err(|_| LateError::Serial("serial break channel closed".into()))?;
        Ok(())
    }

    pub fn open_session(&self, req: OpenSession) -> Result<SessionInfo> {
        match req.kind {
            SessionKind::Ssh => {
                let id = req
                    .device_id
                    .as_deref()
                    .ok_or_else(|| LateError::Message("deviceId required".into()))?;
                self.open_ssh(
                    id,
                    req.accept_unknown_host,
                    req.replace_host_key,
                    req.cols,
                    req.rows,
                )
            }
            SessionKind::Serial => {
                let id = req
                    .device_id
                    .as_deref()
                    .ok_or_else(|| LateError::Message("deviceId required".into()))?;
                self.open_serial(id)
            }
            SessionKind::Local => self.open_local(
                req.shell.clone(),
                req.cols.max(1) as u16,
                req.rows.max(1) as u16,
            ),
            SessionKind::Sftp => {
                let id = req
                    .device_id
                    .as_deref()
                    .ok_or_else(|| LateError::Message("deviceId required".into()))?;
                self.open_sftp(id)
            }
            SessionKind::Pcap => {
                if let Some(path) = req.path.clone() {
                    let v = self.open_pcap(path)?;
                    let info = serde_json::from_value(
                        v.get("session").cloned().unwrap_or(serde_json::Value::Null),
                    )
                    .map_err(|e| LateError::Message(e.to_string()))?;
                    Ok(info)
                } else if let Some(iface) = req.iface.clone() {
                    let _ = self.start_pcap(&iface, req.bpf.clone(), None)?;
                    self.attach(
                        format!("pcap:{iface}"),
                        SessionKind::Pcap,
                        Vendor::Generic,
                        None,
                        None,
                        None,
                        None,
                        None,
                        broadcast::channel(8).1,
                        Some(req),
                    )
                } else {
                    Err(LateError::Message(
                        "pcap session needs path or iface".into(),
                    ))
                }
            }
            SessionKind::Api => {
                let id = req
                    .device_id
                    .as_deref()
                    .ok_or_else(|| LateError::Message("deviceId required".into()))?;
                self.open_api(id)
            }
        }
    }

    pub fn reconnect(&self, session_id: &str) -> Result<SessionInfo> {
        let spec = {
            let inner = self.inner.lock();
            let s = inner
                .sessions
                .get(session_id)
                .ok_or_else(|| LateError::NotFound(session_id.into()))?;
            s.reconnect
                .clone()
                .ok_or_else(|| LateError::Message("session cannot reconnect".into()))?
        };
        self.close_session(session_id)?;
        self.open_session(spec)
    }

    pub fn accept_host_key(&self, host: &str, fingerprint: &str) -> Result<()> {
        let fp = fingerprint.trim();
        if host.trim().is_empty() || fp.is_empty() || fp == "unknown fingerprint" {
            return Err(LateError::Message(
                "host fingerprint is required to trust this host".into(),
            ));
        }
        let mut inner = self.inner.lock();
        inner.known.pin(host.trim(), fp);
        inner.known.save(&self.paths)
    }

    pub fn open_ssh(
        &self,
        device_id: &str,
        accept_unknown_host: bool,
        replace_host_key: bool,
        cols: u32,
        rows: u32,
    ) -> Result<SessionInfo> {
        let device = self.inventory.get(device_id)?;
        let profile_id = device
            .auth_profile_id
            .clone()
            .ok_or_else(|| LateError::Message(
                "this SSH session has no username/password — edit the session and save login like SecureCRT".into(),
            ))?;
        let profile = self.inventory.get_auth(&profile_id)?;
        let mut known = self.inner.lock().known.clone();
        let (io, _fp) = ssh::open_ssh(
            &device,
            &profile,
            &self.secrets,
            &mut known,
            &self.paths,
            SshConnectOpts {
                accept_unknown_host,
                replace_host_key,
            },
            cols.max(1),
            rows.max(1),
        )?;
        {
            let mut inner = self.inner.lock();
            inner.known = known;
            inner.known.save(&self.paths)?;
        }
        self.attach(
            device.name.clone(),
            SessionKind::Ssh,
            device.vendor,
            Some(device.id.clone()),
            device.accent.clone(),
            Some(io.tx),
            Some(io.resize),
            Some(io.close),
            io.rx,
            Some(OpenSession {
                device_id: Some(device.id),
                kind: SessionKind::Ssh,
                accept_unknown_host,
                replace_host_key,
                cols,
                rows,
                shell: None,
                path: None,
                iface: None,
                bpf: None,
            }),
        )
    }

    pub fn open_serial(&self, device_id: &str) -> Result<SessionInfo> {
        let device = self.inventory.get(device_id)?;
        let path = device
            .serial_path
            .clone()
            .ok_or_else(|| LateError::Serial("no serial path".into()))?;
        let io = serial::open_serial(&path, device.baud.unwrap_or(9600))?;
        let break_tx = io.break_tx;
        let info = self.attach(
            device.name.clone(),
            SessionKind::Serial,
            device.vendor,
            Some(device.id.clone()),
            device.accent.clone(),
            Some(io.tx),
            None,
            Some(io.close),
            io.rx,
            Some(OpenSession {
                device_id: Some(device.id),
                kind: SessionKind::Serial,
                accept_unknown_host: false,
                replace_host_key: false,
                cols: 80,
                rows: 24,
                shell: None,
                path: None,
                iface: None,
                bpf: None,
            }),
        )?;
        if let Some(s) = self.inner.lock().sessions.get_mut(&info.id) {
            s.serial_break = Some(break_tx);
        }
        Ok(info)
    }

    pub fn open_local(&self, shell: Option<String>, cols: u16, rows: u16) -> Result<SessionInfo> {
        let io = local_pty::open_local(shell.as_deref(), cols.max(1), rows.max(1))?;
        let resize = io.resize;
        let (r32_tx, mut r32_rx) = mpsc::channel::<(u32, u32)>(8);
        std::thread::spawn(move || {
            while let Some((c, r)) = r32_rx.blocking_recv() {
                let _ = resize.blocking_send((c as u16, r as u16));
            }
        });
        self.attach(
            format!("local:{}", shell.as_deref().unwrap_or("shell")),
            SessionKind::Local,
            Vendor::Linux,
            None,
            None,
            Some(io.tx),
            Some(r32_tx),
            Some(io.close),
            io.rx,
            Some(OpenSession {
                device_id: None,
                kind: SessionKind::Local,
                accept_unknown_host: false,
                replace_host_key: false,
                cols: cols as u32,
                rows: rows as u32,
                shell,
                path: None,
                iface: None,
                bpf: None,
            }),
        )
    }

    pub fn open_sftp(&self, device_id: &str) -> Result<SessionInfo> {
        let device = self.inventory.get(device_id)?;
        self.attach(
            format!("sftp:{}", device.name),
            SessionKind::Sftp,
            device.vendor,
            Some(device.id.clone()),
            device.accent.clone(),
            None,
            None,
            None,
            broadcast::channel(8).1,
            Some(OpenSession {
                device_id: Some(device.id),
                kind: SessionKind::Sftp,
                accept_unknown_host: false,
                replace_host_key: false,
                cols: 80,
                rows: 24,
                shell: None,
                path: None,
                iface: None,
                bpf: None,
            }),
        )
    }

    pub fn sftp_list(&self, session_id: &str, path: &str) -> Result<Vec<SftpEntry>> {
        let (device, profile) = self.sftp_ctx(session_id)?;
        sftp::list_dir(
            &profile,
            &self.secrets,
            device.host.as_deref().unwrap_or(""),
            device.port.unwrap_or(22),
            path,
        )
    }

    pub fn sftp_download(&self, session_id: &str, remote: &str, local: &str) -> Result<()> {
        let (device, profile) = self.sftp_ctx(session_id)?;
        sftp::download(
            &profile,
            &self.secrets,
            device.host.as_deref().unwrap_or(""),
            device.port.unwrap_or(22),
            remote,
            local,
        )
    }

    pub fn sftp_upload(&self, session_id: &str, local: &str, remote: &str) -> Result<()> {
        let (device, profile) = self.sftp_ctx(session_id)?;
        sftp::upload(
            &profile,
            &self.secrets,
            device.host.as_deref().unwrap_or(""),
            device.port.unwrap_or(22),
            local,
            remote,
        )
    }

    pub fn sftp_mkdir(&self, session_id: &str, path: &str) -> Result<()> {
        let (device, profile) = self.sftp_ctx(session_id)?;
        sftp::mkdir(
            &profile,
            &self.secrets,
            device.host.as_deref().unwrap_or(""),
            device.port.unwrap_or(22),
            path,
        )
    }

    pub fn sftp_remove(&self, session_id: &str, path: &str, dir: bool) -> Result<()> {
        let (device, profile) = self.sftp_ctx(session_id)?;
        sftp::remove(
            &profile,
            &self.secrets,
            device.host.as_deref().unwrap_or(""),
            device.port.unwrap_or(22),
            path,
            dir,
        )
    }

    fn sftp_ctx(&self, session_id: &str) -> Result<(Device, AuthProfile)> {
        let inner = self.inner.lock();
        let s = inner
            .sessions
            .get(session_id)
            .ok_or_else(|| LateError::NotFound(session_id.into()))?;
        if s.kind != SessionKind::Sftp {
            return Err(LateError::Message("not an sftp session".into()));
        }
        let did = s
            .device_id
            .clone()
            .ok_or_else(|| LateError::Message("no device".into()))?;
        drop(inner);
        let device = self.inventory.get(&did)?;
        let pid = device
            .auth_profile_id
            .clone()
            .ok_or_else(|| LateError::Message("no auth profile".into()))?;
        let profile = self.inventory.get_auth(&pid)?;
        Ok((device, profile))
    }

    pub fn session_device_id(&self, session_id: &str) -> Result<String> {
        let inner = self.inner.lock();
        let s = inner
            .sessions
            .get(session_id)
            .ok_or_else(|| LateError::NotFound(session_id.into()))?;
        s.device_id
            .clone()
            .ok_or_else(|| LateError::Message("session has no device".into()))
    }

    pub fn start_pcap(
        &self,
        iface: &str,
        bpf: Option<String>,
        count: Option<u32>,
    ) -> Result<serde_json::Value> {
        let dir = self.paths.data.join("pcap");
        std::fs::create_dir_all(&dir)?;
        let file = dir.join(format!("live-{}.pcap", Utc::now().format("%Y%m%d-%H%M%S")));
        let cap = LiveCapture::start(iface, file.clone(), bpf.as_deref(), count)?;
        let info = CaptureInfo {
            id: cap.id.clone(),
            iface: iface.into(),
            file,
            running: true,
            kind: "local".into(),
        };
        self.inner.lock().live_caps.insert(cap.id.clone(), cap);
        Ok(serde_json::to_value(info)?)
    }

    pub fn stop_pcap(&self, id: &str) -> Result<serde_json::Value> {
        let mut inner = self.inner.lock();
        let key = if inner.live_caps.contains_key(id) {
            id.to_string()
        } else if inner.live_caps.len() == 1 {
            inner.live_caps.keys().next().cloned().unwrap()
        } else {
            return Err(LateError::NotFound(id.into()));
        };
        let mut cap = inner
            .live_caps
            .remove(&key)
            .ok_or_else(|| LateError::NotFound(id.into()))?;
        drop(inner);
        cap.stop()?;
        std::thread::sleep(std::time::Duration::from_millis(150));
        if !cap.file.is_file() || cap.file.metadata().map(|m| m.len()).unwrap_or(0) < 24 {
            return Err(LateError::Pcap(format!(
                "no capture file at {} — dumpcap/tcpdump never wrote packets",
                cap.file.display()
            )));
        }
        self.open_pcap(cap.file)
    }

    pub fn open_pcap(&self, path: PathBuf) -> Result<serde_json::Value> {
        let parsed = pcap::parse_pcap(&path)?;
        let info = self.attach(
            format!("pcap:{}", path.display()),
            SessionKind::Pcap,
            Vendor::Generic,
            None,
            None,
            None,
            None,
            None,
            broadcast::channel(8).1,
            Some(OpenSession {
                device_id: None,
                kind: SessionKind::Pcap,
                accept_unknown_host: false,
                replace_host_key: false,
                cols: 80,
                rows: 24,
                shell: None,
                path: Some(path.clone()),
                iface: None,
                bpf: None,
            }),
        )?;
        self.inner.lock().pcaps.insert(
            info.id.clone(),
            OpenPcap {
                packets: parsed.packets.clone(),
                findings: parsed.findings.clone(),
                path: path.clone(),
                total: parsed.total,
                truncated: parsed.truncated,
            },
        );
        Ok(serde_json::json!({
            "session": info,
            "packets": pcap::ui_packets(&parsed.packets),
            "findings": parsed.findings,
            "total": parsed.total,
            "truncated": parsed.truncated,
            "path": path,
        }))
    }

    pub fn capture_ssh_pcap(
        &self,
        device_id: Option<&str>,
        session_id: Option<&str>,
        iface: &str,
        count: u32,
        bpf: Option<&str>,
        auth_profile_id: Option<&str>,
    ) -> Result<serde_json::Value> {
        let (device, profile) = self.resolve_ssh_device(device_id, session_id, auth_profile_id)?;
        let bytes = crate::remote_pcap::capture(
            &device,
            &profile,
            &self.secrets,
            iface,
            count,
            bpf.unwrap_or(""),
        )?;
        let dir = self.paths.data.join("pcap");
        std::fs::create_dir_all(&dir)?;
        let file = dir.join(format!("ssh-{}.pcap", Utc::now().format("%Y%m%d-%H%M%S")));
        std::fs::write(&file, &bytes)?;
        self.open_pcap(file)
    }

    pub fn start_ssh_pcap(
        &self,
        device_id: Option<&str>,
        session_id: Option<&str>,
        iface: &str,
        bpf: Option<&str>,
        auth_profile_id: Option<&str>,
    ) -> Result<serde_json::Value> {
        if !self.inner.lock().live_caps.is_empty() {
            return Err(LateError::Message(
                "a capture is already running — stop it first".into(),
            ));
        }
        let (device, profile) = self.resolve_ssh_device(device_id, session_id, auth_profile_id)?;
        let dir = self.paths.data.join("pcap");
        std::fs::create_dir_all(&dir)?;
        let file = dir.join(format!("ssh-{}.pcap", Utc::now().format("%Y%m%d-%H%M%S")));
        let cap = crate::remote_pcap::start_live(
            &device,
            &profile,
            &self.secrets,
            iface,
            bpf.unwrap_or(""),
            file.clone(),
        )?;
        let info = CaptureInfo {
            id: cap.id.clone(),
            iface: iface.into(),
            file: file.clone(),
            running: true,
            kind: "ssh".into(),
        };
        self.inner.lock().live_caps.insert(cap.id.clone(), cap);
        Ok(serde_json::to_value(info)?)
    }

    pub fn list_pcaps(&self) -> Result<serde_json::Value> {
        let dir = self.paths.data.join("pcap");
        std::fs::create_dir_all(&dir)?;
        let mut files = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for ent in rd.flatten() {
                let path = ent.path();
                let ext = path
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if ext != "pcap" && ext != "pcapng" && ext != "cap" {
                    continue;
                }
                let meta = match ent.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                files.push(serde_json::json!({
                    "name": path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                    "path": path,
                    "bytes": meta.len(),
                    "mtime": mtime,
                }));
            }
        }
        files.sort_by(|a, b| {
            b.get("mtime")
                .and_then(|v| v.as_u64())
                .cmp(&a.get("mtime").and_then(|v| v.as_u64()))
        });
        Ok(serde_json::json!({ "dir": dir, "files": files }))
    }

    /// Resolve SSH host + login for packet capture.
    /// Always uses inventory + a *new* SSH (`ssh -T` / separate PTY), never the serial session.
    fn resolve_ssh_device(
        &self,
        device_id: Option<&str>,
        session_id: Option<&str>,
        auth_profile_id: Option<&str>,
    ) -> Result<(Device, AuthProfile)> {
        let device = if let Some(sid) = session_id.filter(|s| !s.is_empty()) {
            self.ssh_ctx(sid)?.0
        } else {
            let did = device_id
                .filter(|s| !s.is_empty())
                .ok_or_else(|| LateError::Message("deviceId or sessionId required".into()))?;
            self.inventory.get(did)?
        };
        if device.kind != DeviceKind::Ssh {
            return Err(LateError::Message(format!(
                "SSH capture requires an SSH device ('{}' is {})",
                device.name,
                device.kind.as_str()
            )));
        }
        let pid = auth_profile_id
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .or_else(|| device.auth_profile_id.clone())
            .ok_or_else(|| {
                LateError::Message(format!(
                    "device '{}' has no auth_profile_id — edit it and save SSH login",
                    device.name
                ))
            })?;
        let profile = self.inventory.get_auth(&pid)?;
        Ok((device, profile))
    }

    fn ssh_ctx(&self, session_id: &str) -> Result<(Device, AuthProfile)> {
        let inner = self.inner.lock();
        let s = inner
            .sessions
            .get(session_id)
            .ok_or_else(|| LateError::NotFound(session_id.into()))?;
        if s.kind != SessionKind::Ssh {
            return Err(LateError::Message(
                "SSH capture needs an SSH session, not serial — it opens a separate ssh -T and leaves the serial PTY alone".into(),
            ));
        }
        let did = s
            .device_id
            .clone()
            .ok_or_else(|| LateError::Message("no device".into()))?;
        drop(inner);
        let device = self.inventory.get(&did)?;
        let pid = device
            .auth_profile_id
            .clone()
            .ok_or_else(|| LateError::Message("no auth profile".into()))?;
        let profile = self.inventory.get_auth(&pid)?;
        Ok((device, profile))
    }

    pub fn open_pcap_in_wireshark(&self, session_id: Option<&str>, path: Option<&str>) -> Result<()> {
        let file = if let Some(p) = path.filter(|s| !s.is_empty()) {
            PathBuf::from(p)
        } else if let Some(sid) = session_id.filter(|s| !s.is_empty()) {
            let inner = self.inner.lock();
            inner
                .pcaps
                .get(sid)
                .or_else(|| inner.pcaps.values().next())
                .map(|p| p.path.clone())
                .ok_or_else(|| LateError::NotFound("pcap".into()))?
        } else {
            return Err(LateError::Message("no capture path".into()));
        };
        pcap::open_in_wireshark(&file)
    }

    pub fn pcap_query(&self, session_id: &str, q: &str) -> Result<serde_json::Value> {
        let inner = self.inner.lock();
        let p = inner
            .pcaps
            .get(session_id)
            .or_else(|| inner.pcaps.values().next())
            .ok_or_else(|| LateError::NotFound("pcap".into()))?;
        Ok(pcap::query_analysis(&p.packets, &p.findings, q))
    }

    pub fn pcap_filter(&self, session_id: &str, expr: &str) -> Result<Vec<PacketSummary>> {
        let inner = self.inner.lock();
        let p = inner
            .pcaps
            .get(session_id)
            .or_else(|| inner.pcaps.values().next())
            .ok_or_else(|| LateError::NotFound("pcap".into()))?;
        Ok(pcap::filter_packets(&p.packets, expr))
    }

    pub fn pcap_packets(&self, session_id: &str) -> Result<Vec<PacketSummary>> {
        let inner = self.inner.lock();
        let p = inner
            .pcaps
            .get(session_id)
            .or_else(|| inner.pcaps.values().next())
            .ok_or_else(|| LateError::NotFound("pcap".into()))?;
        Ok(pcap::ui_packets(&p.packets))
    }

    pub fn pcap_findings(&self, session_id: &str) -> Result<Vec<PcapFinding>> {
        let inner = self.inner.lock();
        let p = inner
            .pcaps
            .get(session_id)
            .or_else(|| inner.pcaps.values().next())
            .ok_or_else(|| LateError::NotFound("pcap".into()))?;
        Ok(p.findings.clone())
    }

    pub fn pcap_interfaces(&self) -> Vec<String> {
        pcap::list_interfaces()
    }

    pub fn serial_ports(&self) -> Vec<String> {
        serial::list_serial_ports()
    }

    pub async fn api_send(
        &self,
        device_id: &str,
        mut req: ApiRequest,
        agent: bool,
    ) -> Result<ApiResponse> {
        let device = self.inventory.get(device_id)?;
        let base = device
            .api_base_url
            .clone()
            .ok_or_else(|| LateError::Http("no API base URL".into()))?;
        if !req.url.starts_with("http") {
            req.url = format!(
                "{}{}",
                base.trim_end_matches('/'),
                if req.url.starts_with('/') {
                    req.url.clone()
                } else {
                    format!("/{}", req.url)
                }
            );
        }
        if !http_api::host_pinned(&req.url, &base) {
            return Err(LateError::Http(
                "request host is not pinned to the device".into(),
            ));
        }
        let controller = ApiController::parse(device.api_controller.as_deref().unwrap_or("generic"));
        if agent && !http_api::method_allowed_for_agent(&req.method, controller) {
            return Err(LateError::PolicyDenied(
                "agent may only issue GET to host-pinned controllers (FortiManager JSON-RPC is human-only)"
                    .into(),
            ));
        }
        let mut extra = HashMap::new();
        if let Some(pid) = &device.auth_profile_id {
            if let Some(secret) = self.secrets.get(pid)? {
                extra.insert("Authorization".into(), format!("Bearer {secret}"));
            }
        }
        http_api::send_request(req, extra).await
    }

    pub fn open_api(&self, device_id: &str) -> Result<SessionInfo> {
        let device = self.inventory.get(device_id)?;
        self.attach(
            format!("api:{}", device.name),
            SessionKind::Api,
            device.vendor,
            Some(device.id.clone()),
            device.accent.clone(),
            None,
            None,
            None,
            broadcast::channel(8).1,
            Some(OpenSession {
                device_id: Some(device.id),
                kind: SessionKind::Api,
                accept_unknown_host: false,
                replace_host_key: false,
                cols: 80,
                rows: 24,
                shell: None,
                path: None,
                iface: None,
                bpf: None,
            }),
        )
    }

    pub fn save_capture(&self, session_id: &str, name: &str) -> Result<CaptureRecord> {
        let output = self.raw_scrollback(session_id)?;
        let inner = self.inner.lock();
        let s = inner
            .sessions
            .get(session_id)
            .ok_or_else(|| LateError::NotFound(session_id.into()))?;
        let rec = CaptureRecord {
            id: Uuid::new_v4().to_string(),
            name: name.into(),
            device_id: s.device_id.clone(),
            command: s.info.name.clone(),
            output,
            created_at: Utc::now(),
        };
        drop(inner);
        let mut store = CaptureStore::load(&self.paths)?;
        store.add(rec.clone());
        store.save(&self.paths)?;
        Ok(rec)
    }

    pub fn list_captures(&self) -> Result<Vec<CaptureRecord>> {
        Ok(CaptureStore::load(&self.paths)?.captures)
    }

    pub fn diff_captures(&self, a: &str, b: &str) -> Result<Vec<DiffLine>> {
        let store = CaptureStore::load(&self.paths)?;
        let ca = store
            .captures
            .iter()
            .find(|c| c.id == a)
            .ok_or_else(|| LateError::NotFound(format!("capture {a}")))?;
        let cb = store
            .captures
            .iter()
            .find(|c| c.id == b)
            .ok_or_else(|| LateError::NotFound(format!("capture {b}")))?;
        Ok(capture::diff_outputs(&ca.output, &cb.output))
    }

    pub fn export_capture(
        &self,
        session_id: &str,
        name: &str,
        passphrase: Option<&str>,
        redacted: bool,
    ) -> Result<PathBuf> {
        let body = if redacted {
            self.redacted_scrollback(session_id)?
        } else {
            self.raw_scrollback(session_id)?
        };
        capture::export_session(&self.paths, name, &body, passphrase)
    }

    pub fn list_collections(&self) -> Result<CollectionsFile> {
        collections::load(&self.paths)
    }

    pub fn upsert_collection(&self, col: CommandCollection) -> Result<CommandCollection> {
        let mut col = col;
        if col.id.is_empty() {
            col.id = Uuid::new_v4().to_string();
        }
        collections::upsert(&self.paths, col)
    }

    pub fn delete_collection(&self, id: &str) -> Result<()> {
        collections::delete(&self.paths, id)
    }

    pub fn import_file(&self, path: &Path, commit: bool) -> Result<ImportResult> {
        let result = import::import_file(path)?;
        if commit {
            for d in &result.devices {
                self.inventory.upsert_device(d.clone())?;
            }
        }
        Ok(result)
    }

    pub fn list_serial_ports(&self) -> Vec<String> {
        serial::list_serial_ports()
    }

    pub fn list_shells(&self) -> Vec<(String, String)> {
        local_pty::discover_shells()
    }

    #[allow(clippy::too_many_arguments)]
    fn attach(
        &self,
        name: String,
        kind: SessionKind,
        vendor: Vendor,
        device_id: Option<String>,
        accent: Option<String>,
        input: Option<mpsc::Sender<Vec<u8>>>,
        resize: Option<mpsc::Sender<(u32, u32)>>,
        close: Option<mpsc::Sender<()>>,
        mut rx: broadcast::Receiver<Vec<u8>>,
        reconnect: Option<OpenSession>,
    ) -> Result<SessionInfo> {
        let id = Uuid::new_v4().to_string();
        let (out_tx, _) = broadcast::channel::<Vec<u8>>(512);
        let info = SessionInfo {
            id: id.clone(),
            device_id: device_id.clone(),
            name,
            kind,
            vendor,
            connected: true,
            created_at: Utc::now(),
            accent,
        };
        let live = LiveSession {
            info: info.clone(),
            kind,
            vendor,
            device_id,
            input,
            resize,
            close,
            output: out_tx.clone(),
            scrollback: Vec::new(),
            redactor: Redactor::new(),
            logging_path: None,
            reconnect,
            serial_break: None,
        };
        self.inner.lock().sessions.insert(id.clone(), live);
        let inner = self.inner.clone();
        let events = self.events.clone();
        let sid = id.clone();
        tokio::spawn(async move {
            while let Ok(buf) = rx.recv().await {
                let mut g = inner.lock();
                if let Some(s) = g.sessions.get_mut(&sid) {
                    s.scrollback.extend_from_slice(&buf);
                    let max = 2_000_000;
                    if s.scrollback.len() > max {
                        s.scrollback.drain(0..s.scrollback.len() - max);
                    }
                    if let Some(path) = &s.logging_path {
                        let _ = std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(path)
                            .and_then(|mut f| {
                                use std::io::Write;
                                f.write_all(&buf)
                            });
                    }
                    let _ = s.output.send(buf.clone());
                    drop(g);
                    let _ = events.send(AppEvent {
                        event: "session.data".into(),
                        session_id: sid.clone(),
                        data: Some(STANDARD.encode(&buf)),
                        reason: None,
                    });
                } else {
                    break;
                }
            }
            let serial_dropped = {
                let g = inner.lock();
                g.sessions
                    .get(&sid)
                    .map(|s| s.kind == SessionKind::Serial)
                    .unwrap_or(false)
            };
            if serial_dropped {
                let _ = events.send(AppEvent {
                    event: "session.closed".into(),
                    session_id: sid,
                    data: None,
                    reason: Some("dropped".into()),
                });
            }
        });
        Ok(info)
    }

    pub fn set_logging(&self, session_id: &str, path: Option<PathBuf>) -> Result<()> {
        let mut inner = self.inner.lock();
        let s = inner
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| LateError::NotFound(session_id.into()))?;
        s.logging_path = path;
        Ok(())
    }
}

fn bundled_policy_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(p) = std::env::var("LATE_POLICIES_DIR") {
        dirs.push(PathBuf::from(p));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            dirs.push(dir.join("policies"));
            dirs.push(dir.join("../policies"));
        }
    }
    dirs.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../policies"));
    dirs
}

pub fn first_bundled_policy_dir() -> PathBuf {
    bundled_policy_dirs()
        .into_iter()
        .find(|p| p.is_dir())
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../policies"))
}

fn seed_bundled_policies(paths: &LatePaths) -> Result<()> {
    let dest = paths.config.join("policies");
    std::fs::create_dir_all(&dest)?;
    for bundled in bundled_policy_dirs() {
        if !bundled.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(&bundled)? {
            let entry = entry?;
            let to = dest.join(entry.file_name());
            if !to.exists() && entry.path().is_file() {
                let _ = std::fs::copy(entry.path(), to);
            }
        }
        break;
    }
    Ok(())
}
