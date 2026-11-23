//! Containerlab discovery over SSH. The lab host login is key or agent only.
//! Device passwords are not stored here.

use crate::error::{LateError, Result};
use crate::ssh::validate_jump;
use crate::types::Vendor;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClabNode {
    pub long: String,
    pub short: String,
    pub kind: String,
    pub ip: String,
    pub vendor: Vendor,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClabConfig {
    pub host: String,
    pub ssh_user: String,
    pub device_user: String,
}

pub fn config_path() -> PathBuf {
    if let Some(over) = std::env::var_os("CLAB_SSH_CONFIG") {
        let p = PathBuf::from(over);
        if !p.as_os_str().is_empty() {
            return p;
        }
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME").unwrap_or_else(|| ".".into());
            PathBuf::from(home).join(".config")
        });
    base.join("clab-ssh").join("config")
}

pub fn load_config() -> ClabConfig {
    let text = fs::read_to_string(config_path()).unwrap_or_default();
    let mut cfg = ClabConfig::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let v = v.trim().trim_matches('"').trim_matches('\'').to_string();
        match k.trim() {
            "CLAB_HOST" => cfg.host = v,
            "SSH_USER" => cfg.ssh_user = v,
            "CLAB_USER" => cfg.device_user = v,
            _ => {}
        }
    }
    cfg
}

pub fn save_config(cfg: &ClabConfig) -> Result<PathBuf> {
    if !cfg.host.is_empty() {
        validate_jump(&cfg.host)?;
    }
    if !cfg.ssh_user.is_empty() {
        validate_user(&cfg.ssh_user)?;
    }
    if !cfg.device_user.is_empty() {
        validate_user(&cfg.device_user)?;
    }
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| LateError::Message(e.to_string()))?;
    }
    let body = format!(
        "# clab-ssh user settings — local only, do not commit\n\nCLAB_HOST={}\nSSH_USER={}\nCLAB_USER={}\n",
        cfg.host, cfg.ssh_user, cfg.device_user
    );
    fs::write(&path, body).map_err(|e| LateError::Message(e.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(path)
}

pub fn discover(host: &str, ssh_user: &str) -> Result<Vec<ClabNode>> {
    validate_jump(host)?;
    validate_user(ssh_user)?;
    let target = format!("{ssh_user}@{host}");
    let mut cmd = Command::new("ssh");
    cmd.args([
        "-o",
        "BatchMode=yes",
        "-o",
        "StrictHostKeyChecking=accept-new",
        "-o",
        "UpdateHostKeys=no",
        "-o",
        "ConnectTimeout=15",
        &target,
        "clab inspect --all -f json 2>/dev/null || clab inspect -f json",
    ]);
    let (stdout, stderr) = crate::ssh::run_capturing(cmd, Duration::from_secs(20))
        .map_err(|e| LateError::Ssh(format!("clab inspect on {target}: {e}")))?;
    let text = String::from_utf8_lossy(&stdout);
    let raw = text.trim();
    if raw.is_empty() {
        let err = String::from_utf8_lossy(&stderr);
        let err = err.trim();
        let detail = if err.is_empty() {
            "empty inspect output".to_string()
        } else {
            clip(err)
        };
        return Err(LateError::Ssh(format!("clab inspect on {target}: {detail}")));
    }
    parse_inspect(raw)
}

pub fn parse_inspect(raw: &str) -> Result<Vec<ClabNode>> {
    let data: Value = serde_json::from_str(raw)
        .map_err(|e| LateError::Message(format!("clab inspect was not JSON: {e}")))?;
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for node in iter_nodes(&data) {
        let Some(parsed) = parse_node(node) else {
            continue;
        };
        if !seen.insert(parsed.long.clone()) {
            continue;
        }
        out.push(parsed);
    }
    Ok(out)
}

pub fn vendor_for_kind(kind: &str) -> Vendor {
    let k = kind.to_ascii_lowercase();
    if k.contains("srlinux") || k.contains("srl") {
        return Vendor::Generic;
    }
    if k.contains("junos") || k.contains("juniper") || k.contains("vmx") || k.contains("vqfx") {
        return Vendor::Junos;
    }
    if k.contains("nxos") || k.contains("n9kv") || k.contains("nexus") {
        return Vendor::CiscoNxos;
    }
    if k.contains("aoscx") || k.contains("aruba") {
        return Vendor::AosCx;
    }
    if k.contains("eos") || k.contains("arista") || k.contains("ceos") {
        return Vendor::AristaEos;
    }
    if k.contains("ios") || k.contains("xrd") || k.contains("xrv") || k.contains("csr") {
        return Vendor::CiscoIosXe;
    }
    if k.contains("forti") {
        return Vendor::Fortios;
    }
    if k.contains("routeros") || k.contains("mikrotik") {
        return Vendor::Routeros;
    }
    if k.contains("panos") || k.contains("palo") {
        return Vendor::Panos;
    }
    if k.contains("linux") || k.contains("alpine") {
        return Vendor::Linux;
    }
    Vendor::Generic
}

fn parse_node(node: &Value) -> Option<ClabNode> {
    let state = node.get("State").or_else(|| node.get("state")).and_then(|v| v.as_str()).unwrap_or("");
    if !state.is_empty() && state != "running" {
        return None;
    }
    let labels = node.get("Labels").or_else(|| node.get("labels")).cloned().unwrap_or(Value::Null);
    let label = |k: &str| -> String {
        labels.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string()
    };
    let long = {
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
    if long.is_empty() {
        return None;
    }
    let short_guess = short_from_long(&long);
    let short = if !short_guess.is_empty() && short_guess != long {
        short_guess
    } else {
        let named = label("clab-node-name");
        if !named.is_empty() {
            named
        } else {
            node.get("shortname")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(&long)
                .to_string()
        }
    };
    let kind = {
        let labeled = label("clab-node-kind");
        if !labeled.is_empty() {
            labeled
        } else {
            node.get("kind")
                .or_else(|| node.get("Kind"))
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .to_string()
        }
    };
    let ip = strip_cidr(
        node.get("NetworkSettings")
            .and_then(|n| n.get("IPv4addr"))
            .and_then(|v| v.as_str())
            .or_else(|| node.get("ipv4_address").and_then(|v| v.as_str()))
            .or_else(|| node.get("IPv4Address").and_then(|v| v.as_str()))
            .unwrap_or(""),
    );
    if ip.is_empty() || ip == "N/A" || !crate::ssh::validate_scan_host(&ip) {
        return None;
    }
    Some(ClabNode {
        vendor: vendor_for_kind(&kind),
        long,
        short,
        kind,
        ip,
    })
}

fn iter_nodes(data: &Value) -> Vec<&Value> {
    let mut out = Vec::new();
    match data {
        Value::Array(items) => out.extend(items.iter()),
        Value::Object(map) => {
            for value in map.values() {
                if let Value::Array(items) = value {
                    out.extend(items.iter());
                } else if value.get("name").is_some() || value.get("Name").is_some() || value.get("Labels").is_some()
                {
                    out.push(value);
                }
            }
        }
        _ => {}
    }
    out
}

fn short_from_long(long_name: &str) -> String {
    let Some(rest) = long_name.strip_prefix("clab-") else {
        return long_name.to_string();
    };
    match rest.split_once('-') {
        Some((_, node)) if !node.is_empty() => node.to_string(),
        _ => rest.to_string(),
    }
}

fn strip_cidr(value: &str) -> String {
    value.split('/').next().unwrap_or("").trim().to_string()
}

pub fn validate_user(user: &str) -> Result<()> {
    if user.is_empty()
        || user.len() > 64
        || user.starts_with('-')
        || !user
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    {
        return Err(LateError::Message("SSH user has unsupported characters".into()));
    }
    Ok(())
}

fn clip(s: &str) -> String {
    let mut t = s.replace('\n', " ");
    if t.len() > 240 {
        t.truncate(240);
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_inspect_skips_stopped_and_dedupes() {
        let raw = r#"{
          "lab": [
            {"name":"clab-lab-pe1","state":"running","ipv4_address":"172.20.20.2/24",
             "labels":{"clab-node-longname":"clab-lab-pe1","clab-node-name":"pe1","clab-node-kind":"nokia_srlinux"}},
            {"name":"clab-lab-pe1","state":"running","ipv4_address":"172.20.20.2/24",
             "labels":{"clab-node-longname":"clab-lab-pe1","clab-node-kind":"nokia_srlinux"}},
            {"name":"clab-lab-down","state":"exited","ipv4_address":"172.20.20.3/24",
             "labels":{"clab-node-longname":"clab-lab-down","clab-node-kind":"linux"}}
          ]
        }"#;
        let nodes = parse_inspect(raw).unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].short, "pe1");
        assert_eq!(nodes[0].ip, "172.20.20.2");
        assert_eq!(nodes[0].vendor, Vendor::Generic);
    }

    #[test]
    fn kinds_map_to_vendors() {
        assert_eq!(vendor_for_kind("nokia_srlinux"), Vendor::Generic);
        assert_eq!(vendor_for_kind("juniper_vjunosrouter"), Vendor::Junos);
        assert_eq!(vendor_for_kind("ceos"), Vendor::AristaEos);
        assert_eq!(vendor_for_kind("linux"), Vendor::Linux);
    }
}
