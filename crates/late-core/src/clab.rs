//! Containerlab nodes as Late inventory rows and SSH sessions.
//!
//! `clab-ssh` used to write an Ásbrú config and start Ásbrú. Late is the
//! connection manager now: one folder per lab host, one SSH device per node,
//! sessions opened in this app. The lab host is the jump. A SOCKS port or
//! OpenSSH control socket from the launcher keeps the jump password to a
//! single prompt.

use crate::error::{LateError, Result};
use crate::inventory::InventoryStore;
use crate::session::{App, AppEvent, OpenSession};
use crate::ssh_proxy::validate_jump_spec;
use crate::types::{AuthProfile, Device, DeviceKind, SessionKind, Vendor};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::net::{Ipv4Addr, Ipv6Addr};
use zeroize::Zeroize;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClabNode {
    pub long: String,
    pub short: String,
    pub kind: String,
    pub ip: String,
}

pub struct ClabLaunch {
    pub host: String,
    pub ssh_user: String,
    pub device_user: String,
    pub device_password: Option<String>,
    pub no_jump: bool,
    pub socks_port: Option<u16>,
    pub control_path: Option<String>,
    pub nodes: Vec<ClabNode>,
}

impl Drop for ClabLaunch {
    fn drop(&mut self) {
        if let Some(ref mut pw) = self.device_password {
            pw.zeroize();
        }
    }
}

impl std::fmt::Debug for ClabLaunch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClabLaunch")
            .field("host", &self.host)
            .field("ssh_user", &self.ssh_user)
            .field("device_user", &self.device_user)
            .field(
                "device_password",
                &self.device_password.as_ref().map(|_| "***"),
            )
            .field("no_jump", &self.no_jump)
            .field("nodes", &self.nodes.len())
            .finish_non_exhaustive()
    }
}

pub struct StagedNode {
    pub node: ClabNode,
    pub device: Device,
}

fn clean_label(raw: &str, fallback: &str) -> String {
    let s: String = raw
        .chars()
        .filter(|c| !c.is_control())
        .take(120)
        .collect::<String>()
        .trim()
        .to_string();
    if s.is_empty() {
        fallback.to_string()
    } else {
        s
    }
}

fn validate_name(raw: &str, what: &str) -> Result<String> {
    let s = raw.trim();
    if s.is_empty() || s.len() > 64 || s.starts_with('-') {
        return Err(LateError::Message(format!(
            "{what} must be a short hostname label"
        )));
    }
    if !s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(LateError::Message(format!(
            "{what} must be a hostname label (no spaces)"
        )));
    }
    Ok(s.to_string())
}

fn validate_host(raw: &str) -> Result<String> {
    let s = raw.trim();
    if s.is_empty() || s.len() > 253 || s.starts_with('-') {
        return Err(LateError::Message(
            "containerlab host is required".into(),
        ));
    }
    if !s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':'))
    {
        return Err(LateError::Message(
            "containerlab host must be a hostname or IP".into(),
        ));
    }
    Ok(s.to_string())
}

pub fn folder_for_host(host: &str) -> String {
    format!("clab-{host}")
}

/// Stable across launches so rediscovery updates the same inventory row.
pub fn device_id_for(host: &str, long_name: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"late.clab.v1\0");
    hasher.update(host.as_bytes());
    hasher.update(b"\0");
    hasher.update(long_name.as_bytes());
    let dig = hasher.finalize();
    let b = &dig[..16];
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13],
        b[14], b[15]
    )
}

pub fn vendor_for_kind(kind: &str) -> Vendor {
    let k = kind.trim().to_ascii_lowercase().replace('-', "_");
    // SR Linux is not a Linux server. Leave it Generic so the permit list is not
    // the Linux "approve every command" path until the operator sets the OS.
    if k.contains("srlinux") || k == "srl" {
        return Vendor::Generic;
    }
    if k.contains("vjunos")
        || k.contains("vmx")
        || k.contains("vqfx")
        || k.contains("vsrx")
        || k == "junos"
        || k.contains("juniper")
    {
        return Vendor::Junos;
    }
    if k.contains("ceos") || k.contains("arista") || k == "eos" {
        return Vendor::AristaEos;
    }
    if k.contains("nxos") || k.contains("n9kv") || k.contains("nexus") {
        return Vendor::CiscoNxos;
    }
    if k.contains("iosxe")
        || k.contains("ios_xe")
        || k.contains("csr")
        || k.contains("xrd")
        || k.contains("c8000")
        || k.contains("cat9kv")
    {
        return Vendor::CiscoIosXe;
    }
    if k.contains("ios") || k.contains("iol") {
        return Vendor::CiscoIos;
    }
    if k.contains("panos") || k.contains("palo") {
        return Vendor::Panos;
    }
    if k.contains("forti") {
        return Vendor::Fortios;
    }
    if k.contains("routeros") || k.contains("mikrotik") {
        return Vendor::Routeros;
    }
    if k.contains("aos") || k.contains("aruba") {
        return Vendor::AosCx;
    }
    if k.contains("linux") || k.contains("sonic") || k == "cvx" || k.contains("bridge") {
        return Vendor::Linux;
    }
    Vendor::Generic
}

fn strip_cidr(value: &str) -> String {
    value.split('/').next().unwrap_or("").trim().to_string()
}

/// Node name from a containerlab container.
///
/// Official `clab inspect -f json` has no `clab-node-name` label. The lab name
/// is the object key, and the container is `clab-<lab>-<node>` (both sides may
/// contain hyphens). A label still wins when a caller included one.
fn node_short(long_name: &str, lab: Option<&str>, label_name: &str) -> String {
    let labeled = label_name.trim();
    if !labeled.is_empty() {
        return labeled.to_string();
    }
    if let Some(lab) = lab.map(str::trim).filter(|s| !s.is_empty()) {
        let prefix = format!("clab-{lab}-");
        if let Some(rest) = long_name.strip_prefix(&prefix) {
            if !rest.is_empty() {
                return rest.to_string();
            }
        }
    }
    long_name.to_string()
}

fn parse_node(node: &Value, lab: Option<&str>) -> Option<ClabNode> {
    let state = node
        .get("State")
        .or_else(|| node.get("state"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !state.is_empty() && state != "running" {
        return None;
    }
    let labels = node.get("Labels").cloned().unwrap_or(Value::Null);
    let label = |key: &str| -> String {
        labels
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let long_name = {
        let from_label = label("clab-node-longname");
        if !from_label.is_empty() {
            from_label
        } else {
            node.get("name")
                .or_else(|| node.get("Name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        }
    };
    if long_name.is_empty() {
        return None;
    }
    let labeled = label("clab-node-name");
    let short = if !labeled.is_empty() {
        labeled
    } else {
        node.get("shortname")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| node_short(&long_name, lab, ""))
    };
    let kind = {
        let from_label = label("clab-node-kind");
        if !from_label.is_empty() {
            from_label
        } else {
            node.get("kind")
                .or_else(|| node.get("Kind"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("?")
                .to_string()
        }
    };
    let net = node.get("NetworkSettings");
    let ip_raw = net
        .and_then(|n| n.get("IPv4addr"))
        .and_then(|v| v.as_str())
        .or_else(|| node.get("ipv4_address").and_then(|v| v.as_str()))
        .or_else(|| node.get("IPv4Address").and_then(|v| v.as_str()))
        .unwrap_or("");
    let ip = strip_cidr(ip_raw);
    if ip.is_empty() || ip == "N/A" {
        return None;
    }
    if ip.parse::<Ipv4Addr>().is_err() && ip.parse::<Ipv6Addr>().is_err() {
        return None;
    }
    Some(ClabNode {
        long: clean_label(&long_name, &long_name),
        short: clean_label(&short, &short),
        kind: clean_label(&kind, "?"),
        ip,
    })
}

fn looks_like_node(value: &Value) -> bool {
    value.is_object()
        && (value.get("name").is_some()
            || value.get("Name").is_some()
            || value.get("Labels").is_some())
}

fn iter_node_values(data: &Value, lab: Option<&str>, out: &mut Vec<ClabNode>) {
    match data {
        Value::Array(items) => {
            for item in items {
                if let Some(parsed) = item.as_object().and_then(|_| parse_node(item, lab)) {
                    out.push(parsed);
                }
            }
        }
        Value::Object(map) => {
            for (key, value) in map {
                if value.is_array() {
                    // Official inspect: `{ "<lab>": [ containers ] }`.
                    iter_node_values(value, Some(key.as_str()), out);
                } else if looks_like_node(value) {
                    if let Some(parsed) = parse_node(value, lab) {
                        out.push(parsed);
                    }
                }
            }
        }
        _ => {}
    }
}

/// `clab inspect -f json` → running nodes that have a management IP.
pub fn parse_inspect(data: &Value) -> Vec<ClabNode> {
    let mut devices = Vec::new();
    iter_node_values(data, None, &mut devices);
    let mut seen = std::collections::BTreeSet::new();
    devices.retain(|d| seen.insert(d.long.clone()));
    devices
}

fn node_from_value(v: &Value) -> Result<ClabNode> {
    let long = v
        .get("long")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let short = v
        .get("short")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let kind = v
        .get("kind")
        .and_then(|x| x.as_str())
        .unwrap_or("?")
        .trim()
        .to_string();
    let ip = v.get("ip").and_then(|x| x.as_str()).unwrap_or("").trim();
    let ip = strip_cidr(ip);
    if long.is_empty() || short.is_empty() {
        return Err(LateError::Message(
            "each node needs long and short names".into(),
        ));
    }
    if ip.parse::<Ipv4Addr>().is_err() && ip.parse::<Ipv6Addr>().is_err() {
        return Err(LateError::Message(format!(
            "node {short} has no management IP"
        )));
    }
    Ok(ClabNode {
        long: clean_label(&long, &long),
        short: clean_label(&short, &short),
        kind: clean_label(&kind, "?"),
        ip,
    })
}

pub fn launch_from_value(params: &Value) -> Result<ClabLaunch> {
    let host = validate_host(
        params
            .get("host")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
    )?;
    let ssh_user = validate_name(
        params
            .get("ssh_user")
            .or_else(|| params.get("sshUser"))
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        "lab SSH user",
    )?;
    let device_user = validate_name(
        params
            .get("device_user")
            .or_else(|| params.get("deviceUser"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("admin"),
        "device user",
    )?;
    let no_jump = params
        .get("no_jump")
        .or_else(|| params.get("noJump"))
        .and_then(|v| v.as_bool())
        == Some(true);
    let socks_port = params
        .get("socks_port")
        .or_else(|| params.get("socksPort"))
        .and_then(|v| v.as_u64())
        .and_then(|n| u16::try_from(n).ok())
        .filter(|p| *p > 0);
    let control_path = params
        .get("control_path")
        .or_else(|| params.get("controlPath"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let password = params
        .get("device_password")
        .or_else(|| params.get("devicePassword"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    if !no_jump {
        validate_jump_spec(&format!("{ssh_user}@{host}"))?;
    }
    let arr = params
        .get("devices")
        .or_else(|| params.get("nodes"))
        .and_then(|v| v.as_array())
        .ok_or_else(|| LateError::Message("devices list is required".into()))?;
    if arr.is_empty() {
        return Err(LateError::Message("no containerlab nodes to open".into()));
    }
    if arr.len() > 64 {
        return Err(LateError::Message(
            "refusing to open more than 64 nodes at once".into(),
        ));
    }
    let mut nodes = Vec::with_capacity(arr.len());
    for item in arr {
        nodes.push(node_from_value(item)?);
    }
    Ok(ClabLaunch {
        host,
        ssh_user,
        device_user,
        device_password: password,
        no_jump,
        socks_port,
        control_path,
        nodes,
    })
}

/// Structured host-key failure, when `err` is one the desktop dialog understands.
pub(crate) fn host_key_failure(err: &LateError) -> Option<Value> {
    let data = err.rpc_data()?;
    let code = data.get("code").and_then(|v| v.as_str()).unwrap_or("");
    if code == "host_key_mismatch" || code == "host_key_untrusted" {
        Some(data)
    } else {
        None
    }
}

/// Write or refresh inventory rows. Does not open SSH.
pub fn stage_clab_devices(inventory: &InventoryStore, launch: &ClabLaunch) -> Result<Vec<StagedNode>> {
    let folder = folder_for_host(&launch.host);
    let jump = if launch.no_jump {
        None
    } else {
        Some(format!("{}@{}", launch.ssh_user, launch.host))
    };
    let _ = inventory.upsert_folder(&folder)?;
    let mut staged = Vec::with_capacity(launch.nodes.len());
    for node in &launch.nodes {
        let id = device_id_for(&launch.host, &node.long);
        let mut device = inventory.get(&id).unwrap_or_else(|_| {
            Device::new_ssh(&node.short, &node.ip, vendor_for_kind(&node.kind))
        });
        let previous_vendor = device.vendor;
        device.id = id;
        device.name = node.short.clone();
        device.host = Some(node.ip.clone());
        device.port = Some(22);
        device.kind = DeviceKind::Ssh;
        if previous_vendor == Vendor::Generic || device.tags.is_empty() {
            device.vendor = vendor_for_kind(&node.kind);
        }
        device.folder = Some(folder.clone());
        device.jump_host = jump.clone();
        if device.notes.as_deref().unwrap_or("").is_empty() {
            device.notes = Some(format!("{} ({})", node.long, node.kind));
        }
        if !device.tags.iter().any(|t| t == "containerlab") {
            device.tags.push("containerlab".into());
        }
        if node.kind != "?" && !device.tags.iter().any(|t| t == &node.kind) {
            device.tags.push(node.kind.clone());
        }
        let needs_prompt_login = launch.device_password.is_none()
            && device
                .auth_profile_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .is_none();
        if needs_prompt_login {
            let profile = inventory.upsert_auth(AuthProfile {
                id: String::new(),
                name: format!("{} login", node.short),
                username: launch.device_user.clone(),
                key_path: None,
                use_agent: true,
                has_password: false,
            })?;
            device.auth_profile_id = Some(profile.id);
        }
        let device = inventory.upsert_device(device)?;
        staged.push(StagedNode {
            node: node.clone(),
            device,
        });
    }
    Ok(staged)
}

impl App {
    /// Stage every node, then open an SSH session in this Late process.
    /// One failure does not cancel the rest. Password is never part of the result.
    pub fn open_clab(&self, launch: ClabLaunch) -> Result<Value> {
        let staged = stage_clab_devices(&self.inventory, &launch)?;
        let folder = folder_for_host(&launch.host);
        let socks = if launch.no_jump {
            None
        } else {
            launch.socks_port.filter(|p| *p > 0)
        };
        let control = if launch.no_jump {
            None
        } else {
            launch
                .control_path
                .clone()
                .filter(|s| !s.is_empty())
        };
        if let Some(jump) = staged.first().and_then(|item| item.device.jump_host.clone()) {
            self.remember_jump_tunnel(jump, socks, control.clone());
        }
        let mut results = Vec::with_capacity(staged.len());
        let mut opened = 0usize;
        let mut failed = 0usize;
        for item in staged {
            let mut req = OpenSession::empty();
            req.device_id = Some(item.device.id.clone());
            req.kind = SessionKind::Ssh;
            req.host = item.device.host.clone();
            req.port = item.device.port;
            req.username = Some(launch.device_user.clone());
            req.password = launch.device_password.clone();
            req.name = Some(item.node.short.clone());
            req.vendor = Some(item.device.vendor);
            req.save_session = true;
            req.save_password = launch.device_password.is_some();
            req.accept_unknown_host = true;
            req.cols = 120;
            req.rows = 36;
            req.socks_port = socks;
            req.control_path = control.clone();
            match self.open_ssh(req) {
                Ok(info) => {
                    let _ = self.events.send(AppEvent {
                        event: "session.opened".into(),
                        session_id: info.id.clone(),
                        data: serde_json::to_string(&info).ok(),
                        bytes: None,
                        reason: None,
                    });
                    opened += 1;
                    results.push(json!({
                        "name": item.node.short,
                        "device_id": item.device.id,
                        "session_id": info.id,
                        "ok": true,
                    }));
                }
                Err(err) => {
                    failed += 1;
                    let mut row = json!({
                        "name": item.node.short,
                        "device_id": item.device.id,
                        "ok": false,
                        "error": err.to_string(),
                    });
                    if let Some(mut payload) = host_key_failure(&err) {
                        payload["device_id"] = json!(item.device.id);
                        payload["name"] = json!(item.node.short);
                        let _ = self.events.send(AppEvent {
                            event: "host_key".into(),
                            session_id: String::new(),
                            data: serde_json::to_string(&payload).ok(),
                            bytes: None,
                            reason: None,
                        });
                        row["host_key"] = payload;
                    }
                    results.push(row);
                }
            }
        }
        Ok(json!({
            "folder": folder,
            "opened": opened,
            "failed": failed,
            "results": results,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LatePaths;
    use crate::secrets::SecretStore;

    fn isolated() -> (InventoryStore, LatePaths, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("late-clab-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let paths = LatePaths {
            config: dir.clone(),
            data: dir.clone(),
        };
        paths.ensure().unwrap();
        (InventoryStore::new(paths.clone()), paths, dir)
    }

    fn sample_inspect() -> Value {
        json!({
            "lab1": [
                {
                    "Name": "clab-lab1-pe-1",
                    "State": "running",
                    "Kind": "nokia_srlinux",
                    "IPv4Address": "172.20.20.2/24",
                    "Labels": {
                        "clab-node-longname": "clab-lab1-pe-1",
                        "clab-node-name": "pe-1",
                        "clab-node-kind": "nokia_srlinux"
                    }
                },
                {
                    "Name": "clab-lab1-stopped",
                    "State": "exited",
                    "IPv4Address": "172.20.20.3/24"
                },
                {
                    "name": "clab-lab1-ce-1",
                    "state": "running",
                    "NetworkSettings": {"IPv4addr": "172.20.20.4/24"},
                    "Labels": {"clab-node-kind": "ceos"}
                }
            ]
        })
    }

    #[test]
    fn parse_inspect_keeps_running_nodes_and_strips_cidr() {
        let nodes = parse_inspect(&sample_inspect());
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].short, "pe-1");
        assert_eq!(nodes[0].ip, "172.20.20.2");
        assert_eq!(nodes[0].kind, "nokia_srlinux");
        assert_eq!(nodes[1].short, "ce-1");
        assert_eq!(nodes[1].ip, "172.20.20.4");
        assert_eq!(vendor_for_kind("nokia_srlinux"), Vendor::Generic);
        assert_eq!(vendor_for_kind("ceos"), Vendor::AristaEos);
        assert_eq!(vendor_for_kind("cisco_xrd"), Vendor::CiscoIosXe);
    }

    #[test]
    fn parse_inspect_uses_lab_key_for_hyphenated_names() {
        // Official `clab inspect -f json`: lab name is the object key, and
        // there is no clab-node-name label. Splitting on the first hyphen
        // would turn clab-srl-lab-spine1 into lab-spine1.
        let nodes = parse_inspect(&json!({
            "srl-lab": [
                {
                    "name": "clab-srl-lab-spine1",
                    "state": "running",
                    "kind": "nokia_srlinux",
                    "ipv4_address": "172.20.20.11/24"
                },
                {
                    "name": "clab-srl-lab-leaf-1",
                    "state": "running",
                    "kind": "nokia_srlinux",
                    "ipv4_address": "172.20.20.12/24"
                },
                {
                    "name": "clab-srl-lab-border-1",
                    "state": "running",
                    "kind": "nokia_srlinux",
                    "ipv4_address": "172.20.20.13/24",
                    "Labels": {"clab-node-name": "edge"}
                }
            ]
        }));
        assert_eq!(nodes.len(), 3);
        assert_eq!(nodes[0].short, "spine1");
        assert_eq!(nodes[1].short, "leaf-1");
        assert_eq!(nodes[2].short, "edge");
    }

    #[test]
    fn device_id_is_stable() {
        assert_eq!(
            device_id_for("192.0.2.10", "clab-lab1-pe-1"),
            device_id_for("192.0.2.10", "clab-lab1-pe-1")
        );
        assert_ne!(
            device_id_for("192.0.2.10", "clab-lab1-pe-1"),
            device_id_for("192.0.2.11", "clab-lab1-pe-1")
        );
    }

    #[test]
    fn stage_groups_nodes_and_sets_jump_without_storing_password() {
        let (inv, paths, dir) = isolated();
        let launch = launch_from_value(&json!({
            "host": "192.0.2.10",
            "ssh_user": "labuser",
            "device_user": "admin",
            "device_password": "device-secret-should-not-land-in-inventory",
            "socks_port": 1080,
            "devices": [
                {"long": "clab-lab1-pe-1", "short": "pe-1", "kind": "nokia_srlinux", "ip": "172.20.20.2"}
            ]
        }))
        .unwrap();
        let debug = format!("{launch:?}");
        assert!(!debug.contains("device-secret"));
        assert!(debug.contains("***"));
        let staged = stage_clab_devices(&inv, &launch).unwrap();
        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].device.folder.as_deref(), Some("clab-192.0.2.10"));
        assert_eq!(
            staged[0].device.jump_host.as_deref(),
            Some("labuser@192.0.2.10")
        );
        assert_eq!(staged[0].device.host.as_deref(), Some("172.20.20.2"));
        let raw = std::fs::read_to_string(paths.inventory()).unwrap();
        assert!(!raw.contains("device-secret"));
        let secrets = SecretStore::new(paths);
        assert!(secrets.get("nope").unwrap().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn restage_keeps_user_vendor_and_notes() {
        let (inv, _paths, dir) = isolated();
        let launch = launch_from_value(&json!({
            "host": "lab.example",
            "ssh_user": "lab",
            "device_user": "admin",
            "devices": [
                {"long": "clab-lab-pe-1", "short": "pe-1", "kind": "linux", "ip": "172.20.20.2"}
            ]
        }))
        .unwrap();
        let first = stage_clab_devices(&inv, &launch).unwrap();
        let mut edited = first[0].device.clone();
        edited.vendor = Vendor::Junos;
        edited.notes = Some("operator note".into());
        edited.accent = Some("#abc".into());
        inv.upsert_device(edited).unwrap();
        let again = stage_clab_devices(&inv, &launch).unwrap();
        assert_eq!(again[0].device.vendor, Vendor::Junos);
        assert_eq!(again[0].device.notes.as_deref(), Some("operator note"));
        assert_eq!(again[0].device.accent.as_deref(), Some("#abc"));
        assert_eq!(again[0].device.id, first[0].device.id);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn no_jump_clears_jump_host() {
        let (inv, _paths, dir) = isolated();
        let launch = launch_from_value(&json!({
            "host": "192.0.2.10",
            "ssh_user": "lab",
            "device_user": "admin",
            "no_jump": true,
            "devices": [
                {"long": "clab-lab-pe-1", "short": "pe-1", "kind": "linux", "ip": "172.20.20.2"}
            ]
        }))
        .unwrap();
        let staged = stage_clab_devices(&inv, &launch).unwrap();
        assert!(staged[0].device.jump_host.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_shell_in_host_and_huge_batches() {
        let err = launch_from_value(&json!({
            "host": "192.0.2.10;rm",
            "ssh_user": "lab",
            "devices": [{"long": "n", "short": "n", "kind": "linux", "ip": "172.20.20.2"}]
        }));
        assert!(err.is_err());
        let mut devices = Vec::new();
        for i in 0..65 {
            devices.push(json!({"long": format!("n{i}"), "short": format!("n{i}"), "kind": "linux", "ip": "172.20.20.2"}));
        }
        let err = launch_from_value(&json!({
            "host": "192.0.2.10",
            "ssh_user": "lab",
            "devices": devices
        }));
        assert!(err.unwrap_err().to_string().contains("64"));
    }

    #[test]
    fn host_key_failure_is_the_dialog_payload() {
        let mismatch = LateError::HostKeyMismatch {
            host: "172.20.20.2".into(),
            pinned: "SHA256:old".into(),
            presented: "SHA256:new".into(),
        };
        let payload = host_key_failure(&mismatch).unwrap();
        assert_eq!(payload["code"], "host_key_mismatch");
        assert_eq!(payload["host"], "172.20.20.2");
        assert_eq!(payload["presented"], "SHA256:new");
        assert!(host_key_failure(&LateError::Ssh("no SSH banner".into())).is_none());
    }
}
