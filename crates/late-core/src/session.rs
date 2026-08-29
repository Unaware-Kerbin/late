use crate::capture::{self, CaptureStore, DiffLine};
use crate::collections;
use crate::config::{load_settings, save_settings, AppSettings, LatePaths};
use crate::confine;
use crate::error::{LateError, Result};
use crate::http_api::{self, ApiRequest, ApiResponse};
use crate::import::{self, ImportResult};
use crate::inventory::InventoryStore;
use crate::known_hosts::KnownHosts;
use crate::local_pty;
use crate::pcap::{self, CaptureInfo, LiveCapture};
use crate::policy::{self, PolicyEngine, PolicyView};
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
use zeroize::Zeroize;

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
    pub policy: Arc<Mutex<PolicyEngine>>,
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
    auth_profile_id: Option<String>,
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
        Self::boot_with(LatePaths::discover())
    }

    pub fn boot_with(paths: LatePaths) -> Result<Self> {
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
            policy: Arc::new(Mutex::new(policy)),
            events,
            inner: Arc::new(Mutex::new(Inner {
                sessions: HashMap::new(),
                known,
                pcaps: HashMap::new(),
                live_caps: HashMap::new(),
            })),
        })
    }

    #[cfg(test)]
    fn insert_test_session(&self, info: SessionInfo) {
        self.insert_test_session_with_auth(info, None);
    }

    #[cfg(test)]
    fn insert_test_session_with_auth(&self, info: SessionInfo, auth_profile_id: Option<String>) {
        let (out_tx, _) = broadcast::channel(8);
        self.inner.lock().sessions.insert(
            info.id.clone(),
            LiveSession {
                kind: info.kind,
                vendor: info.vendor,
                device_id: info.device_id.clone(),
                auth_profile_id,
                info,
                input: None,
                resize: None,
                close: None,
                output: out_tx,
                scrollback: Vec::new(),
                redactor: Redactor::new(),
                logging_path: None,
                reconnect: None,
                serial_break: None,
            },
        );
    }

    pub fn settings(&self) -> AppSettings {
        self.settings.lock().clone()
    }

    pub fn set_settings(&self, settings: AppSettings) -> Result<()> {
        self.validate_settings_dir(&settings.pcap_dir)?;
        self.validate_settings_dir(&settings.log_dir)?;
        let mut settings = settings;
        settings.mcp_cwd = settings.mcp_cwd.trim().to_string();
        settings.mcp_command = settings.mcp_command.trim().to_string();
        settings.mcp_args = settings.mcp_args.trim().to_string();
        settings.mcp_url = settings.mcp_url.trim().to_string();
        crate::config::validate_mcp_http_url(&settings.mcp_url)?;
        // Agent pane can pick This computer before the Settings folder is filled in.
        if !settings.mcp_cwd.trim().is_empty() {
            self.validate_settings_dir(&PathBuf::from(settings.mcp_cwd.trim()))?;
        }
        let cmd = settings.mcp_command.trim();
        if cmd.starts_with('/') || cmd.contains(":\\") || cmd.starts_with("\\\\") {
            self.validate_settings_dir(&PathBuf::from(cmd))?;
        }
        if !settings.cloud_chat_enabled && settings.default_backend.eq_ignore_ascii_case("cursor") {
            settings.default_backend = "local".into();
        }
        crate::config::remember_remote_inference_urls(&mut settings);
        let prev_cloud = self.settings.lock().cloud_chat_enabled;
        let prev_mcp = self.settings.lock().mcp_enabled;
        save_settings(&self.paths.settings(), &settings)?;
        *self.settings.lock() = settings.clone();
        if prev_cloud != settings.cloud_chat_enabled {
            let _ = crate::audit::append_detail(
                &self.paths,
                "settings.cloud_chat",
                true,
                serde_json::json!({ "enabled": settings.cloud_chat_enabled }),
            );
        }
        if prev_mcp != settings.mcp_enabled {
            let _ = crate::audit::append_detail(
                &self.paths,
                "settings.mcp",
                true,
                serde_json::json!({ "enabled": settings.mcp_enabled }),
            );
        }
        Ok(())
    }

    fn validate_settings_dir(&self, path: &PathBuf) -> Result<()> {
        if path.as_os_str().is_empty() {
            return Ok(());
        }
        let mut roots = Vec::new();
        if let Some(home) = dirs::home_dir() {
            roots.push(home);
        }
        roots.push(self.paths.data.clone());
        roots.push(self.paths.config.clone());
        if path.is_dir() {
            confine::confine_dir(path, &roots)?;
        } else {
            confine::confine_under_roots(path, &roots, false)?;
        }
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
        let vendor = {
            let mut inner = self.inner.lock();
            let s = inner
                .sessions
                .get_mut(session_id)
                .ok_or_else(|| LateError::NotFound(session_id.into()))?;
            let text = String::from_utf8_lossy(&s.scrollback);
            let vendor = Vendor::infer_from_text(&text, s.vendor);
            s.vendor = vendor;
            vendor
        };
        Ok(self.policy.lock().check(vendor, command))
    }

    pub fn check_policy(&self, vendor: Vendor, command: &str) -> PolicyDecision {
        self.policy.lock().check(vendor, command)
    }

    pub fn policy_list(&self) -> Vec<PolicyView> {
        let dir = self.paths.config.join("policies");
        let engine = self.policy.lock();
        Vendor::ALL
            .iter()
            .map(|v| engine.view(*v, &dir))
            .collect()
    }

    pub fn policy_get(&self, vendor: Vendor) -> PolicyView {
        let dir = self.paths.config.join("policies");
        self.policy.lock().view(vendor, &dir)
    }

    pub fn policy_set_allow(&self, vendor: Vendor, allow: Vec<String>) -> Result<PolicyView> {
        if vendor == Vendor::Linux {
            return Err(LateError::Message(
                "Linux has no permit list. Every command still needs Approve. Always-allow stays off."
                    .into(),
            ));
        }
        let dir = self.paths.config.join("policies");
        std::fs::create_dir_all(&dir)?;
        let path = policy::overlay_path(&dir, vendor);
        let current = if path.is_file() {
            let raw = std::fs::read_to_string(&path)?;
            serde_yaml::from_str(&raw)?
        } else {
            self.policy.lock().get(vendor)
        };
        let saved = policy::apply_allow_list(current, allow)?;
        policy::write_vendor_policy(&path, &saved)?;
        {
            let bundled = first_bundled_policy_dir();
            let mut engine = self.policy.lock();
            engine.reload(&bundled, &dir)?;
        }
        Ok(self.policy_get(vendor))
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
        let (h, p) = crate::known_hosts::parse_host_port_key(host.trim())?;
        let observed = crate::ssh::probe_fingerprint(&h, p)?;
        if observed != fp {
            return Err(LateError::Ssh(
                "fingerprint does not match the host key Late just scanned".into(),
            ));
        }
        let hk = crate::known_hosts::host_port_key(&h, p);
        let mut inner = self.inner.lock();
        inner.known.pin(&hk, &observed);
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
        let info = self.attach(
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
        )?;
        if let Some(s) = self.inner.lock().sessions.get_mut(&info.id) {
            s.auth_profile_id = Some(profile_id);
        }
        Ok(info)
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
            format!("scp:{}", device.name),
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

    fn operator_fs_roots(&self) -> Vec<PathBuf> {
        let mut roots = Vec::new();
        if let Some(home) = dirs::home_dir() {
            roots.push(home);
        }
        roots.push(self.paths.data.clone());
        roots.push(self.paths.config.clone());
        roots
    }

    pub fn sftp_download(
        &self,
        session_id: &str,
        remote: &str,
        local: &str,
        recursive: bool,
    ) -> Result<()> {
        let (device, profile) = self.sftp_ctx(session_id)?;
        let dest =
            confine::confine_under_roots(Path::new(local), &self.operator_fs_roots(), false)?;
        sftp::download(
            &profile,
            &self.secrets,
            device.host.as_deref().unwrap_or(""),
            device.port.unwrap_or(22),
            remote,
            &dest.to_string_lossy(),
            recursive,
        )
    }

    pub fn sftp_upload(
        &self,
        session_id: &str,
        local: &str,
        remote: &str,
        recursive: bool,
    ) -> Result<()> {
        let (device, profile) = self.sftp_ctx(session_id)?;
        let local_path = Path::new(local);
        let recursive = recursive || local_path.is_dir();
        let src = if recursive {
            confine::confine_dir(local_path, &self.operator_fs_roots())?
        } else {
            confine::confine_under_roots(local_path, &self.operator_fs_roots(), true)?
        };
        sftp::upload(
            &profile,
            &self.secrets,
            device.host.as_deref().unwrap_or(""),
            device.port.unwrap_or(22),
            &src.to_string_lossy(),
            remote,
            recursive,
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
        crate::fsutil::mkdir_private(&dir)?;
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
        let _ = crate::fsutil::chmod_private(&cap.file);
        self.open_pcap(cap.file)
    }

    pub fn open_pcap(&self, path: PathBuf) -> Result<serde_json::Value> {
        let path = confine::confine_under_roots(&path, &self.operator_fs_roots(), true)?;
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
        crate::fsutil::mkdir_private(&dir)?;
        let file = dir.join(format!("ssh-{}.pcap", Utc::now().format("%Y%m%d-%H%M%S")));
        std::fs::write(&file, &bytes)?;
        let _ = crate::fsutil::chmod_private(&file);
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
        crate::fsutil::mkdir_private(&dir)?;
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
        crate::fsutil::mkdir_private(&dir)?;
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

    pub fn open_pcap_in_wireshark(
        &self,
        session_id: Option<&str>,
        path: Option<&str>,
    ) -> Result<()> {
        let file = if let Some(p) = path.filter(|s| !s.is_empty()) {
            confine::confine_under_roots(Path::new(p), &self.operator_fs_roots(), true)?
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
        let controller =
            ApiController::parse(device.api_controller.as_deref().unwrap_or("generic"));
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
        let insecure_tls = self.settings.lock().api_insecure_tls;
        http_api::send_request(req, extra, insecure_tls)
            .await
            .map(|mut resp| {
                if agent {
                    resp.body = resp.redacted_body.clone();
                    resp.headers.retain(|k, _| {
                        let l = k.to_ascii_lowercase();
                        !l.contains("auth") && l != "set-cookie" && l != "cookie"
                    });
                }
                resp
            })
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

    fn stage_vendor(&self, device_id: Option<&str>, session_id: Option<&str>) -> Result<Vendor> {
        self.stage_vendor_hint(device_id, session_id, None)
    }

    fn stage_vendor_hint(
        &self,
        device_id: Option<&str>,
        session_id: Option<&str>,
        saved_vendor: Option<&str>,
    ) -> Result<Vendor> {
        if let Some(id) = device_id.filter(|s| !s.is_empty()) {
            let inv = self.inventory.load()?;
            if let Some(d) = inv.devices.iter().find(|d| d.id == id) {
                return Ok(d.vendor);
            }
        }
        if let Some(id) = session_id.filter(|s| !s.is_empty()) {
            if let Some(s) = self.list_sessions().into_iter().find(|s| s.id == id) {
                if s.vendor != Vendor::Generic {
                    return Ok(s.vendor);
                }
            }
        }
        let saved = saved_vendor.map(Vendor::parse).unwrap_or(Vendor::Generic);
        if saved != Vendor::Generic {
            return Ok(saved);
        }
        if let Some(id) = session_id.filter(|s| !s.is_empty()) {
            if let Some(s) = self.list_sessions().into_iter().find(|s| s.id == id) {
                return Ok(s.vendor);
            }
        }
        Ok(Vendor::Generic)
    }

    pub fn stage_render(
        &self,
        format: &str,
        intent: &str,
        body: Option<&str>,
        device_id: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<crate::stage::StageArtifact> {
        let vendor = self.stage_vendor(device_id, session_id)?;
        let mut art = crate::stage::render(
            crate::stage::StageFormat::parse(format)?,
            vendor,
            intent,
            body,
        )?;
        art.device_id = device_id.filter(|s| !s.is_empty()).map(|s| s.to_string());
        art.session_id = session_id.filter(|s| !s.is_empty()).map(|s| s.to_string());
        Ok(art)
    }

    pub fn stage_save(
        &self,
        format: &str,
        intent: &str,
        body: Option<&str>,
        device_id: Option<&str>,
        session_id: Option<&str>,
        id: Option<&str>,
    ) -> Result<crate::stage::StageArtifact> {
        let existing = id
            .filter(|s| !s.is_empty())
            .and_then(|i| self.stage_get(i).ok());
        let device = device_id
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .or_else(|| existing.as_ref().and_then(|e| e.device_id.clone()));
        let session = session_id
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .or_else(|| existing.as_ref().and_then(|e| e.session_id.clone()));
        let vendor = self.stage_vendor_hint(
            device.as_deref(),
            session.as_deref(),
            existing.as_ref().map(|e| e.vendor.as_str()),
        )?;
        let mut art = crate::stage::render(
            crate::stage::StageFormat::parse(format)?,
            vendor,
            intent,
            body,
        )?;
        art.device_id = device;
        art.session_id = session;
        if let Some(id) = id.filter(|s| !s.is_empty()) {
            art.id = id.to_string();
        }
        crate::stage::save(&self.paths, art)
    }

    pub fn stage_get(&self, id: &str) -> Result<crate::stage::StageArtifact> {
        crate::stage::get(&self.paths, id)
    }

    pub fn stage_list(&self) -> Result<Vec<crate::stage::StageMeta>> {
        crate::stage::list(&self.paths)
    }

    pub fn stage_delete(&self, id: &str) -> Result<()> {
        crate::stage::delete(&self.paths, id)
    }

    fn resolve_push_target(
        &self,
        device_id: Option<&str>,
        session_id: Option<&str>,
        art: &crate::stage::StageArtifact,
    ) -> Result<Option<crate::stage::PushTarget>> {
        use crate::stage::StageFormat;
        // PATH Push uses the inventory device. Prefer the Device dropdown; if the
        // helper only stored a live SSH session, use that session's inventory row.
        // Serial consoles are never the Ansible/Netmiko target.
        let from_session = self.ssh_session_device_id(
            session_id
                .filter(|s| !s.is_empty())
                .or(art.session_id.as_deref()),
        );
        let id = device_id
            .filter(|s| !s.is_empty())
            .or(art.device_id.as_deref())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .or(from_session);
        match art.format {
            StageFormat::Cli => Ok(None),
            StageFormat::Chef | StageFormat::Salt => match id {
                Some(id) => Ok(Some(self.require_push_target(&id)?)),
                None => Ok(None),
            },
            StageFormat::Ansible | StageFormat::Netmiko => {
                let id = id.ok_or_else(|| {
                    LateError::Message(
                        "Pick an SSH inventory device on your computer (hostname/IP). Push session is only for Push CLI into an open terminal."
                            .into(),
                    )
                })?;
                Ok(Some(self.require_push_target(&id)?))
            }
        }
    }

    fn live_ssh_auth_hints(&self) -> Vec<(Option<String>, Option<String>)> {
        self.inner
            .lock()
            .sessions
            .values()
            .filter(|s| s.kind == SessionKind::Ssh && s.info.connected)
            .map(|s| (s.device_id.clone(), s.auth_profile_id.clone()))
            .collect()
    }

    /// Device auth profile, else the profile an open SSH session already used for this host.
    fn auth_profile_for_push(&self, device: &Device) -> Result<AuthProfile> {
        if let Some(pid) = device.auth_profile_id.as_deref().filter(|s| !s.is_empty()) {
            if let Ok(p) = self.inventory.get_auth(pid) {
                return Ok(p);
            }
        }
        let hints = self.live_ssh_auth_hints();
        for (did, pid) in &hints {
            if did.as_deref() != Some(device.id.as_str()) {
                continue;
            }
            if let Some(pid) = pid.as_deref().filter(|s| !s.is_empty()) {
                if let Ok(p) = self.inventory.get_auth(pid) {
                    return Ok(p);
                }
            }
        }
        let host = device.host.as_deref().filter(|s| !s.is_empty());
        if let Some(host) = host {
            let port = device.port.unwrap_or(22);
            for (did, pid) in &hints {
                let Some(did) = did.as_deref() else { continue };
                let Ok(other) = self.inventory.get(did) else {
                    continue;
                };
                if other.host.as_deref() != Some(host) || other.port.unwrap_or(22) != port {
                    continue;
                }
                let pid = pid
                    .as_deref()
                    .or(other.auth_profile_id.as_deref())
                    .filter(|s| !s.is_empty());
                if let Some(pid) = pid {
                    if let Ok(p) = self.inventory.get_auth(pid) {
                        return Ok(p);
                    }
                }
            }
        }
        Err(LateError::Message(
            "Add an auth profile with a username and SSH key or agent on your computer.".into(),
        ))
    }

    fn require_push_target(&self, device_id: &str) -> Result<crate::stage::PushTarget> {
        let device = self.inventory.get(device_id)?;
        if device.jump_host.as_deref().is_some_and(|s| !s.is_empty()) {
            return Err(LateError::Message(
                "Jump hosts are not used for generated inventory. Fill inventory on your computer if you need a jump host."
                    .into(),
            ));
        }
        let host = device
            .host
            .clone()
            .filter(|s| !s.is_empty() && !s.contains("://"))
            .ok_or_else(|| {
                LateError::Message(
                    if device.kind == DeviceKind::Serial {
                        "This inventory device is serial-only (no SSH host). Pick an SSH inventory device (hostname/IP) on your computer, not a serial session. Push CLI types into an open serial terminal."
                            .into()
                    } else {
                        "Fill inventory on your computer: this device has no SSH host (hostname/IP). PATH Push does not use an open serial session."
                            .into()
                    },
                )
            })?;
        let profile = self.auth_profile_for_push(&device)?;
        if profile.username.trim().is_empty() {
            return Err(LateError::Message(
                "Auth profile has no username. Set it on your computer.".into(),
            ));
        }
        let key_path = crate::ssh::confined_identity(&profile)?;
        let vault_password = if key_path.is_none() && !profile.use_agent {
            self.secrets.get(&profile.id)?.filter(|s| !s.is_empty())
        } else {
            None
        };
        let has_vault_password = vault_password.is_some();
        if let Some(mut pw) = vault_password {
            pw.zeroize();
        }
        if key_path.is_none() && !profile.use_agent && !has_vault_password {
            return Err(LateError::Message(
                "Late will not put a password in generated files. Add an SSH key or enable the agent on the auth profile on your computer."
                    .into(),
            ));
        }
        Ok(crate::stage::PushTarget {
            name: device.name,
            vendor: device.vendor,
            host,
            port: device.port.unwrap_or(22),
            username: profile.username,
            key_path,
            use_agent: profile.use_agent,
            has_vault_password,
            auth_profile_id: Some(profile.id),
        })
    }

    fn ssh_session_device_id(&self, session_id: Option<&str>) -> Option<String> {
        let sid = session_id.filter(|s| !s.is_empty())?;
        let inner = self.inner.lock();
        let s = inner.sessions.get(sid)?;
        if s.kind != SessionKind::Ssh {
            return None;
        }
        s.device_id.clone().filter(|d| !d.is_empty())
    }

    fn stage_prepare(
        &self,
        id: Option<&str>,
        format: &str,
        intent: &str,
        body: Option<&str>,
        device_id: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<crate::stage::StageArtifact> {
        let mut art = if let Some(id) = id.filter(|s| !s.is_empty()) {
            if body.map(|s| !s.trim().is_empty()).unwrap_or(false) {
                self.stage_save(format, intent, body, device_id, session_id, Some(id))?
            } else {
                self.stage_get(id)?
            }
        } else {
            self.stage_save(format, intent, body, device_id, session_id, None)?
        };
        if crate::stage::looks_like_placeholder_cli(&art.body) {
            let body_owned = art.body.clone();
            let intent_owned = if intent.trim().is_empty() {
                art.intent.clone()
            } else {
                intent.to_string()
            };
            let id_owned = art.id.clone();
            art = self.stage_save(
                format,
                &intent_owned,
                Some(&body_owned),
                device_id.or(art.device_id.as_deref()),
                session_id.or(art.session_id.as_deref()),
                Some(&id_owned),
            )?;
        }
        Ok(art)
    }

    pub fn stage_plan(
        &self,
        id: Option<&str>,
        format: &str,
        intent: &str,
        body: Option<&str>,
        device_id: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<crate::stage::StagePlan> {
        let art = self.stage_prepare(id, format, intent, body, device_id, session_id)?;
        let target = self.resolve_push_target(device_id, session_id, &art)?;
        crate::stage::plan_push(&self.paths, &art, target.as_ref())
    }

    pub fn stage_push(
        &self,
        id: Option<&str>,
        format: &str,
        intent: &str,
        body: Option<&str>,
        device_id: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<crate::stage::StagePushResult> {
        let art = self.stage_prepare(id, format, intent, body, device_id, session_id)?;
        if art.format == crate::stage::StageFormat::Cli {
            let sid = session_id
                .filter(|s| !s.is_empty())
                .or(art.session_id.as_deref())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| LateError::Message("Pick an open SSH or serial session.".into()))?;
            self.stage_push_cli(sid, &art.body)
        } else {
            let target = self.resolve_push_target(device_id, session_id, &art)?;
            let mut runtime_password = None;
            if let Some(t) = target.as_ref() {
                if t.has_vault_password {
                    if let Some(pid) = t.auth_profile_id.as_deref().filter(|s| !s.is_empty()) {
                        runtime_password = self.secrets.get(pid)?.filter(|s| !s.is_empty());
                    }
                    if runtime_password.is_none() {
                        return Err(LateError::Message(
                            "Late will not put a password in generated files. Add an SSH key or enable the agent on the auth profile on your computer."
                                .into(),
                        ));
                    }
                }
            }
            let result = crate::stage::run_push(
                &self.paths,
                &art,
                target.as_ref(),
                runtime_password.as_deref(),
            );
            if let Some(ref mut pw) = runtime_password {
                pw.zeroize();
            }
            result
        }
    }

    fn stage_push_cli(
        &self,
        session_id: &str,
        body: &str,
    ) -> Result<crate::stage::StagePushResult> {
        {
            let inner = self.inner.lock();
            let s = inner
                .sessions
                .get(session_id)
                .ok_or_else(|| LateError::NotFound(session_id.into()))?;
            if s.kind != SessionKind::Ssh && s.kind != SessionKind::Serial {
                return Err(LateError::Message(
                    "CLI Push needs an open SSH or serial session.".into(),
                ));
            }
            if s.input.is_none() || !s.info.connected {
                return Err(LateError::Message(
                    "Open the SSH or serial session on your computer first. CLI Push types into that terminal.".into(),
                ));
            }
        }
        let mut lines: Vec<String> = Vec::new();
        for line in body.replace('\r', "").split('\n') {
            let t = line.trim();
            if t.is_empty() || t.starts_with('!') || t.starts_with('#') {
                continue;
            }
            let decision = self.check_command(session_id, t)?;
            if !decision.allowed {
                return Err(LateError::Message(format!(
                    "Permit list denied `{t}`: {}. Push did not send any lines.",
                    decision.reason
                )));
            }
            lines.push(line.to_string());
        }
        if lines.is_empty() {
            return Err(LateError::Message(
                "Nothing to push. This draft has no CLI lines (comments or placeholder only). Render it, or use ansible format for a playbook.".into(),
            ));
        }
        for line in &lines {
            self.write(session_id, format!("{line}\r").as_bytes())?;
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
        Ok(crate::stage::StagePushResult {
            ok: true,
            format: crate::stage::StageFormat::Cli,
            display: format!("typed {} line(s) into session {session_id}", lines.len()),
            exit_code: None,
            stdout_tail: String::new(),
            stderr_tail: String::new(),
        })
    }

    pub fn import_file(&self, path: &Path, commit: bool) -> Result<ImportResult> {
        let path = confine::confine_under_roots(path, &self.operator_fs_roots(), true)?;
        let result = import::import_file(&path)?;
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
            auth_profile_id: None,
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
                        let _ = crate::fsutil::append_private(path, &buf);
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
            let src = entry.path();
            if !src.is_file() {
                continue;
            }
            let to = dest.join(entry.file_name());
            if !to.exists() {
                let _ = std::fs::copy(&src, &to);
            } else {
                let _ = crate::policy::absorb_bundled_allow(&to, &src);
            }
        }
        break;
    }
    Ok(())
}

#[cfg(test)]
mod stage_push_tests {
    use super::*;
    use crate::types::{AuthProfile, Device, DeviceKind, Vendor};

    fn isolated_app() -> (App, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("late-stage-push-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let paths = LatePaths {
            config: dir.clone(),
            data: dir.clone(),
        };
        let app = App::boot_with(paths).unwrap();
        (app, dir)
    }

    fn boot_with_stale_aos_cx_policy() -> (App, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("late-stage-stale-{}", Uuid::new_v4()));
        let dest = dir.join("policies");
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(
            dest.join("aos_cx.yaml"),
            "vendor: aos_cx\nallow: [show, configure, vlan, end]\ndeny: [erase, reload, start-shell]\n",
        )
        .unwrap();
        let paths = LatePaths {
            config: dir.clone(),
            data: dir.clone(),
        };
        let app = App::boot_with(paths).unwrap();
        (app, dir)
    }

    fn ssh_lab(app: &App) -> Device {
        let profile = app
            .inventory
            .upsert_auth(AuthProfile {
                id: String::new(),
                name: "lab login".into(),
                username: "admin".into(),
                key_path: None,
                use_agent: true,
                has_password: false,
            })
            .unwrap();
        let mut d = Device::new_ssh("edge-sw", "192.0.2.10", Vendor::Linux);
        d.auth_profile_id = Some(profile.id);
        app.inventory.upsert_device(d).unwrap()
    }

    fn serial_console(app: &App) -> Device {
        let mut d = Device::new_ssh("edge-console", "192.0.2.10", Vendor::Linux);
        d.kind = DeviceKind::Serial;
        d.host = None;
        d.port = None;
        d.serial_path = Some("/dev/ttyUSB0".into());
        d.auth_profile_id = None;
        app.inventory.upsert_device(d).unwrap()
    }

    fn ssh_aruba_password(app: &App, secret: Option<&str>) -> Device {
        let profile = app
            .inventory
            .upsert_auth(AuthProfile {
                id: String::new(),
                name: "aruba login".into(),
                username: "admin".into(),
                key_path: None,
                use_agent: false,
                has_password: secret.is_some(),
            })
            .unwrap();
        if let Some(pw) = secret {
            app.secrets.set(&profile.id, pw).unwrap();
        }
        let mut d = Device::new_ssh("lab-aruba", "192.0.2.80", Vendor::AosCx);
        d.auth_profile_id = Some(profile.id);
        app.inventory.upsert_device(d).unwrap()
    }

    fn ssh_session(device_id: &str) -> SessionInfo {
        SessionInfo {
            id: "ssh-sess-1".into(),
            device_id: Some(device_id.into()),
            name: "aruba ssh".into(),
            kind: SessionKind::Ssh,
            vendor: Vendor::AosCx,
            connected: true,
            created_at: Utc::now(),
            accent: None,
        }
    }

    fn serial_session(device_id: &str) -> SessionInfo {
        SessionInfo {
            id: "serial-sess-1".into(),
            device_id: Some(device_id.into()),
            name: "console".into(),
            kind: SessionKind::Serial,
            vendor: Vendor::Linux,
            connected: true,
            created_at: Utc::now(),
            accent: None,
        }
    }

    #[test]
    fn path_push_uses_inventory_ssh_even_if_session_is_serial() {
        let (app, dir) = isolated_app();
        let ssh = ssh_lab(&app);
        let serial = serial_console(&app);
        app.insert_test_session(serial_session(&serial.id));

        let art = app
            .stage_save(
                "ansible",
                "ntp servers",
                Some("hosts: all\ntasks: []\n"),
                Some(&ssh.id),
                Some("serial-sess-1"),
                None,
            )
            .unwrap();
        let target = app
            .resolve_push_target(Some(&ssh.id), Some("serial-sess-1"), &art)
            .unwrap()
            .expect("PATH Push must resolve the SSH inventory device");
        assert_eq!(target.host, "192.0.2.10");
        assert_eq!(target.username, "admin");

        let plan = app.stage_plan(
            Some(&art.id),
            "ansible",
            "ntp servers",
            Some("hosts: all\ntasks: []\n"),
            Some(&ssh.id),
            Some("serial-sess-1"),
        );
        if let Err(e) = plan {
            let m = e.to_string();
            assert!(
                !m.contains("PATH Push needs an SSH inventory device"),
                "serial session must not reject PATH Push when an SSH inventory device is selected: {m}"
            );
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn path_push_works_without_session_id() {
        let (app, dir) = isolated_app();
        let ssh = ssh_lab(&app);
        let art = app
            .stage_save(
                "netmiko",
                "ntp",
                Some("from netmiko import ConnectHandler\n"),
                Some(&ssh.id),
                None,
                None,
            )
            .unwrap();
        let target = app
            .resolve_push_target(Some(&ssh.id), None, &art)
            .unwrap()
            .expect("inventory SSH device is enough");
        assert_eq!(target.host, "192.0.2.10");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn path_push_password_only_profile_does_not_write_secret() {
        let (app, dir) = isolated_app();
        let secret = "super-secret-pass-aruba";
        let aruba = ssh_aruba_password(&app, Some(secret));
        app.insert_test_session(ssh_session(&aruba.id));
        let art = app
            .stage_save(
                "ansible",
                "ntp servers",
                Some("hosts: late_targets\ntasks: []\n"),
                Some(&aruba.id),
                None,
                None,
            )
            .unwrap();
        let target = app
            .resolve_push_target(Some(&aruba.id), None, &art)
            .unwrap()
            .expect("password-only vault auth is enough");
        assert_eq!(target.host, "192.0.2.80");
        assert_eq!(target.username, "admin");
        assert!(target.has_vault_password);
        assert!(target.key_path.is_none());
        assert!(!target.use_agent);
        assert_eq!(target.vendor, Vendor::AosCx);

        let bindir = std::env::temp_dir().join(format!("late-fake-bins-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&bindir).unwrap();
        for name in ["ansible-playbook", "sshpass"] {
            let p = bindir.join(name);
            std::fs::write(&p, "#!/bin/sh\nexit 0\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = std::fs::metadata(&p).unwrap().permissions();
                perms.set_mode(0o755);
                std::fs::set_permissions(&p, perms).unwrap();
            }
        }
        let _path = crate::stage::TEST_PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let old = std::env::var("PATH").ok();
        std::env::set_var("PATH", &bindir);
        let plan = app.stage_plan(
            Some(&art.id),
            "ansible",
            "ntp servers",
            Some("hosts: late_targets\ntasks: []\n"),
            Some(&aruba.id),
            None,
        );
        match old {
            Some(v) => std::env::set_var("PATH", v),
            None => std::env::remove_var("PATH"),
        }
        let plan = plan.expect("plan should succeed for password-only vault auth");
        let staging = crate::stage::staging_dir(&app.paths);
        let mut saw_inv = false;
        for ent in std::fs::read_dir(&staging).unwrap() {
            let p = ent.unwrap().path();
            let text = std::fs::read_to_string(&p).unwrap_or_default();
            assert!(
                !text.contains(secret),
                "staging file {} leaked vault password",
                p.display()
            );
            let lower = text.to_ascii_lowercase();
            assert!(!lower.contains("ansible_ssh_pass"), "{}", p.display());
            if p.extension().and_then(|e| e.to_str()) == Some("ini") {
                saw_inv = true;
                assert!(!lower.contains("sshpass"), "{}", p.display());
            }
        }
        assert!(saw_inv, "expected generated inventory.ini");
        assert!(
            !plan.argv.join(" ").contains(secret),
            "password must not be on argv"
        );
        let _ = std::fs::remove_dir_all(bindir);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn path_push_password_flag_without_vault_is_refused() {
        let (app, dir) = isolated_app();
        let aruba = ssh_aruba_password(&app, None);
        let art = app
            .stage_save(
                "ansible",
                "ntp",
                Some("hosts: late_targets\ntasks: []\n"),
                Some(&aruba.id),
                None,
                None,
            )
            .unwrap();
        let err = app
            .resolve_push_target(Some(&aruba.id), None, &art)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("will not put a password in generated files"),
            "{err}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn path_push_auth_from_open_ssh_session_when_device_profile_missing() {
        let (app, dir) = isolated_app();
        let profile = app
            .inventory
            .upsert_auth(AuthProfile {
                id: String::new(),
                name: "session login".into(),
                username: "admin".into(),
                key_path: None,
                use_agent: false,
                has_password: true,
            })
            .unwrap();
        app.secrets
            .set(&profile.id, "session-vault-secret")
            .unwrap();
        let mut d = Device::new_ssh("lab-aruba", "192.0.2.80", Vendor::AosCx);
        d.auth_profile_id = None;
        let d = app.inventory.upsert_device(d).unwrap();
        app.insert_test_session_with_auth(ssh_session(&d.id), Some(profile.id.clone()));
        let art = app
            .stage_save(
                "netmiko",
                "ntp",
                Some("from netmiko import ConnectHandler\n"),
                Some(&d.id),
                None,
                None,
            )
            .unwrap();
        let target = app
            .resolve_push_target(Some(&d.id), None, &art)
            .unwrap()
            .expect("open SSH session auth should fill in");
        assert_eq!(target.username, "admin");
        assert!(target.has_vault_password);
        assert_eq!(target.auth_profile_id.as_deref(), Some(profile.id.as_str()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn path_push_rejects_serial_only_device_clearly() {
        let (app, dir) = isolated_app();
        let serial = serial_console(&app);
        app.insert_test_session(serial_session(&serial.id));
        let art = app
            .stage_save(
                "ansible",
                "ntp",
                Some("hosts: all\ntasks: []\n"),
                Some(&serial.id),
                Some("serial-sess-1"),
                None,
            )
            .unwrap();
        let err = app
            .resolve_push_target(Some(&serial.id), Some("serial-sess-1"), &art)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("serial-only") || err.contains("no SSH host"),
            "{err}"
        );
        assert!(err.contains("your computer"), "{err}");
        assert!(!err.contains("PATH Push needs an SSH inventory device. Use Push CLI"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn save_without_session_keeps_saved_session_and_device() {
        let (app, dir) = isolated_app();
        let aruba = ssh_aruba_password(&app, Some("vault-secret"));
        app.insert_test_session(ssh_session(&aruba.id));
        let art = app
            .stage_save(
                "ansible",
                "configure VLAN 2000",
                None,
                Some(&aruba.id),
                Some("ssh-sess-1"),
                None,
            )
            .unwrap();
        assert_eq!(art.session_id.as_deref(), Some("ssh-sess-1"));
        assert_eq!(art.device_id.as_deref(), Some(aruba.id.as_str()));
        let again = app
            .stage_save(
                "ansible",
                "configure VLAN 2000",
                Some(&art.body),
                None,
                None,
                Some(&art.id),
            )
            .unwrap();
        assert_eq!(again.session_id.as_deref(), Some("ssh-sess-1"));
        assert_eq!(again.device_id.as_deref(), Some(aruba.id.as_str()));
        assert!(again.body.contains("vlan 2000"), "{}", again.body);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn mcp_cli_body_save_keeps_session_for_cli_push() {
        let (app, dir) = isolated_app();
        let aruba = ssh_aruba_password(&app, Some("vault-secret"));
        app.insert_test_session(ssh_session(&aruba.id));
        let art = app
            .stage_save(
                "cli",
                "configure VLAN 2500",
                Some("vlan 2500\nname VLAN2500\n"),
                None,
                Some("ssh-sess-1"),
                None,
            )
            .unwrap();
        assert_eq!(art.session_id.as_deref(), Some("ssh-sess-1"));
        assert!(art.body.contains("vlan 2500"), "{}", art.body);
        assert!(art.body.contains("name VLAN2500"), "{}", art.body);

        let (in_tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(8);
        {
            let mut inner = app.inner.lock();
            if let Some(s) = inner.sessions.get_mut("ssh-sess-1") {
                s.input = Some(in_tx);
                s.info.connected = true;
            }
        }
        let pushed = app
            .stage_push(
                Some(&art.id),
                "cli",
                "configure VLAN 2500",
                None,
                None,
                None,
            )
            .unwrap();
        assert!(pushed.ok, "{}", pushed.display);
        assert!(pushed.display.contains("2 line"), "{}", pushed.display);
        let first = rx.try_recv().expect("CLI Push must type into the saved session");
        assert!(String::from_utf8_lossy(&first).contains("vlan 2500"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn path_push_uses_ssh_session_device_when_device_id_omitted() {
        let (app, dir) = isolated_app();
        let aruba = ssh_aruba_password(&app, Some("vault-secret"));
        app.insert_test_session(ssh_session(&aruba.id));
        let art = app
            .stage_save(
                "ansible",
                "configure VLAN 2000",
                None,
                None,
                Some("ssh-sess-1"),
                None,
            )
            .unwrap();
        assert!(art.body.contains("vlan 2000"), "{}", art.body);
        assert!(art.body.contains("name VLAN2000"), "{}", art.body);
        assert!(!art.body.to_ascii_lowercase().contains("replace with vendor syntax"));
        let target = app
            .resolve_push_target(None, Some("ssh-sess-1"), &art)
            .unwrap()
            .expect("SSH session inventory row is enough for PATH Push");
        assert_eq!(target.host, "192.0.2.80");
        assert_eq!(target.vendor, Vendor::AosCx);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn push_prepare_fills_placeholder_when_aoscx_device_is_known() {
        let (app, dir) = isolated_app();
        let aruba = ssh_aruba_password(&app, Some("vault-secret"));
        let placeholder = "---\n- name: vlan\n  hosts: late_targets\n  tasks:\n    - name: apply\n      ansible.netcommon.cli_config:\n        config: |\n          # Replace with vendor syntax for: configure VLAN 2000\n";
        let art = app
            .stage_save(
                "ansible",
                "configure VLAN 2000",
                Some(placeholder),
                None,
                None,
                None,
            )
            .unwrap();
        assert!(
            art.body.to_ascii_lowercase().contains("replace with vendor syntax")
                || art.vendor == "generic",
            "{}",
            art.body
        );
        let filled = app
            .stage_plan(
                Some(&art.id),
                "ansible",
                "configure VLAN 2000",
                Some(placeholder),
                Some(&aruba.id),
                None,
            );
        match filled {
            Ok(plan) => {
                let play = std::path::PathBuf::from(plan.file.unwrap());
                let body = std::fs::read_to_string(&play).unwrap();
                assert!(body.contains("vlan 2000"), "{body}");
                assert!(body.contains("name VLAN2000"), "{body}");
                assert!(!body.to_ascii_lowercase().contains("replace with vendor syntax"));
            }
            Err(e) => {
                let m = e.to_string();
                assert!(
                    m.contains("ansible-playbook") || m.contains("not installed"),
                    "unexpected plan error: {m}"
                );
                let loaded = app.stage_get(&art.id).unwrap();
                assert!(loaded.body.contains("vlan 2000"), "{}", loaded.body);
                assert!(!loaded.body.to_ascii_lowercase().contains("replace with vendor syntax"));
            }
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cli_push_refuses_placeholder_and_permit_denied() {
        let (app, dir) = isolated_app();
        let aruba = ssh_aruba_password(&app, Some("vault-secret"));
        app.insert_test_session(ssh_session(&aruba.id));
        let err = app
            .stage_push(
                None,
                "cli",
                "configure VLAN 2000",
                Some("# Replace with vendor syntax for: vlan\n"),
                Some(&aruba.id),
                Some("ssh-sess-1"),
            )
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("Open the SSH") || err.contains("Nothing to push") || err.contains("placeholder"),
            "{err}"
        );

        let (in_tx, _rx) = tokio::sync::mpsc::channel::<Vec<u8>>(8);
        {
            let mut inner = app.inner.lock();
            if let Some(s) = inner.sessions.get_mut("ssh-sess-1") {
                s.input = Some(in_tx);
                s.info.connected = true;
            }
        }
        let err = app
            .stage_push(
                None,
                "cli",
                "reload",
                Some("reload\n"),
                Some(&aruba.id),
                Some("ssh-sess-1"),
            )
            .unwrap_err()
            .to_string();
        assert!(err.contains("Permit list") || err.contains("denied"), "{err}");
        assert!(err.contains("did not send") || err.to_ascii_lowercase().contains("reload"), "{err}");

        let ok = app
            .stage_push(
                None,
                "cli",
                "configure VLAN 2000",
                Some("configure terminal\nvlan 2000\nname VLAN2000\nend\n"),
                Some(&aruba.id),
                Some("ssh-sess-1"),
            )
            .unwrap();
        assert!(ok.ok);
        assert!(ok.display.contains("4 line"));

        let vlan2500 = app
            .stage_push(
                None,
                "cli",
                "configure VLAN 2500",
                Some("configure terminal\nvlan 2500\nname VLAN2500\nend\n"),
                Some(&aruba.id),
                Some("ssh-sess-1"),
            )
            .unwrap();
        assert!(vlan2500.ok, "{}", vlan2500.display);
        assert!(vlan2500.display.contains("4 line"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cli_push_vlan_name_works_when_config_yaml_omits_name() {
        let (app, dir) = boot_with_stale_aos_cx_policy();
        let aruba = ssh_aruba_password(&app, Some("vault-secret"));
        app.insert_test_session(ssh_session(&aruba.id));
        let (in_tx, _rx) = tokio::sync::mpsc::channel::<Vec<u8>>(8);
        {
            let mut inner = app.inner.lock();
            if let Some(s) = inner.sessions.get_mut("ssh-sess-1") {
                s.input = Some(in_tx);
                s.info.connected = true;
            }
        }
        let name = app.check_command("ssh-sess-1", "name VLAN2500").unwrap();
        assert!(name.allowed, "{}", name.reason);
        let ok = app
            .stage_push(
                None,
                "cli",
                "configure VLAN 2500",
                Some("configure terminal\nvlan 2500\nname VLAN2500\nend\n"),
                Some(&aruba.id),
                Some("ssh-sess-1"),
            )
            .unwrap();
        assert!(ok.ok, "{}", ok.display);
        let denied = app.check_command("ssh-sess-1", "reload").unwrap();
        assert!(!denied.allowed, "{}", denied.reason);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn policy_set_allow_saves_yaml_and_reloads() {
        let (app, dir) = isolated_app();
        let listed = app.policy_list();
        assert!(listed.iter().any(|p| p.vendor == "aos_cx"));
        let linux = app.policy_get(Vendor::Linux);
        assert!(linux.unrestricted);
        assert!(!linux.allow_always_allow);
        let err = app.policy_set_allow(Vendor::Linux, vec!["ls".into()]).unwrap_err();
        assert!(err.to_string().contains("Linux has no permit list"));

        let mut allow = app.policy_get(Vendor::AosCx).allow;
        if !allow.iter().any(|a| a.eq_ignore_ascii_case("name")) {
            allow.push("name".into());
        }
        let view = app.policy_set_allow(Vendor::AosCx, allow).unwrap();
        assert!(view.allow.iter().any(|a| a.eq_ignore_ascii_case("name")));
        let yaml = std::fs::read_to_string(dir.join("policies/aos_cx.yaml")).unwrap();
        assert!(yaml.to_ascii_lowercase().contains("name"));
        assert!(app.check_policy(Vendor::AosCx, "name VLAN2500").allowed);
        assert!(!app.check_policy(Vendor::AosCx, "reload").allowed);

        let without_name: Vec<String> = view
            .allow
            .into_iter()
            .filter(|a| !a.eq_ignore_ascii_case("name"))
            .collect();
        let after = app.policy_set_allow(Vendor::AosCx, without_name).unwrap();
        assert!(!after.allow.iter().any(|a| a.eq_ignore_ascii_case("name")));
        assert!(after
            .builtin_allow
            .iter()
            .any(|a| a.eq_ignore_ascii_case("name")));
        assert!(app.check_policy(Vendor::AosCx, "name VLAN2500").allowed);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn stage_delete_removes_saved_draft() {
        let (app, dir) = isolated_app();
        let art = app
            .stage_save("cli", "ntp", Some("show ntp\n"), None, None, None)
            .unwrap();
        assert_eq!(app.stage_list().unwrap().len(), 1);
        app.stage_delete(&art.id).unwrap();
        assert!(app.stage_list().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
