use crate::error::{LateError, Result};
use crate::types::{Device, DeviceKind, Vendor};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, serde::Serialize)]
pub struct ImportResult {
    pub devices: Vec<Device>,
    pub warnings: Vec<String>,
}

pub fn import_file(path: &Path) -> Result<ImportResult> {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let raw = std::fs::read_to_string(path)?;
    if name.ends_with(".csv") || name.contains("csv") {
        import_csv(&raw)
    } else if name.contains("config") || path.file_name() == Some(std::ffi::OsStr::new("config")) {
        import_ssh_config(&raw)
    } else if name.ends_with(".xml") {
        import_securecrt_xml(&raw)
    } else if name.ends_with(".ini") || name.contains("moba") {
        import_mobaxterm(&raw)
    } else {
        import_ssh_config(&raw)
    }
}

pub fn import_csv(raw: &str) -> Result<ImportResult> {
    let mut rdr = csv::Reader::from_reader(raw.as_bytes());
    let mut devices = Vec::new();
    let mut warnings = Vec::new();
    for rec in rdr.deserialize() {
        let row: CsvRow = rec.map_err(|e| LateError::Import(e.to_string()))?;
        if row.host.trim().is_empty() {
            warnings.push("skipped row with empty host".into());
            continue;
        }
        if row.password.is_some() {
            warnings.push(format!(
                "ignored password column for {} (map credentials in the wizard)",
                row.host
            ));
        }
        let mut d = Device::new_ssh(
            if row.name.is_empty() {
                &row.host
            } else {
                &row.name
            },
            &row.host,
            Vendor::parse(&row.vendor),
        );
        if let Some(p) = row.port {
            d.port = Some(p);
        }
        d.folder = row.folder.filter(|s| !s.is_empty());
        d.tags = row
            .tags
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        devices.push(d);
    }
    Ok(ImportResult { devices, warnings })
}

#[derive(Debug, Deserialize)]
struct CsvRow {
    #[serde(default)]
    name: String,
    host: String,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    vendor: String,
    #[serde(default)]
    folder: Option<String>,
    #[serde(default)]
    tags: Option<String>,
    #[serde(default)]
    password: Option<String>,
}

pub fn import_ssh_config(raw: &str) -> Result<ImportResult> {
    let mut devices = Vec::new();
    let mut warnings = Vec::new();
    let mut cur_names: Vec<String> = Vec::new();
    let mut host: Option<String> = None;
    let mut port = 22u16;
    let mut user: Option<String> = None;
    let flush = |names: &Vec<String>,
                 host: &Option<String>,
                 port: u16,
                 _user: &Option<String>,
                 devices: &mut Vec<Device>| {
        for n in names {
            if n.contains('*') || n.contains('?') {
                continue;
            }
            let h = host.clone().unwrap_or_else(|| n.clone());
            let mut d = Device::new_ssh(n, &h, Vendor::Generic);
            d.port = Some(port);
            devices.push(d);
        }
    };
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.splitn(2, char::is_whitespace);
        let key = parts.next().unwrap_or("").to_ascii_lowercase();
        let val = parts.next().unwrap_or("").trim();
        match key.as_str() {
            "host" => {
                flush(&cur_names, &host, port, &user, &mut devices);
                cur_names = val.split_whitespace().map(|s| s.to_string()).collect();
                host = None;
                port = 22;
                user = None;
            }
            "hostname" => host = Some(val.to_string()),
            "port" => port = val.parse().unwrap_or(22),
            "user" => user = Some(val.to_string()),
            "proxyjump" => warnings.push(format!("ProxyJump on {:?} not auto-wired", cur_names)),
            _ => {}
        }
    }
    flush(&cur_names, &host, port, &user, &mut devices);
    Ok(ImportResult { devices, warnings })
}

pub fn import_securecrt_xml(raw: &str) -> Result<ImportResult> {
    let mut devices = Vec::new();
    let mut warnings = Vec::new();
    // VanDyke session XML: look for hostname/protocol tags without touching password vaults.
    if raw.to_ascii_lowercase().contains("password") {
        warnings.push("SecureCRT password vault entries were ignored".into());
    }
    let mut cur_name = String::new();
    let mut cur_host = String::new();
    let mut cur_port = 22u16;
    for line in raw.lines() {
        let l = line.trim();
        if let Some(n) = xml_text(l, "name") {
            cur_name = n;
        }
        if let Some(n) = xml_text(l, "hostname") {
            cur_host = n;
        }
        if let Some(n) = xml_text(l, "port") {
            cur_port = n.parse().unwrap_or(22);
        }
        if l.contains("</session>") || l.contains("</Session>") {
            if !cur_host.is_empty() {
                let name = if cur_name.is_empty() {
                    cur_host.clone()
                } else {
                    cur_name.clone()
                };
                let mut d = Device::new_ssh(&name, &cur_host, Vendor::Generic);
                d.port = Some(cur_port);
                devices.push(d);
            }
            cur_name.clear();
            cur_host.clear();
            cur_port = 22;
        }
    }
    if devices.is_empty() {
        warnings.push("no SSH sessions found in XML".into());
    }
    Ok(ImportResult { devices, warnings })
}

fn xml_text(line: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let a = line.find(&open)? + open.len();
    let b = line.find(&close)?;
    Some(line[a..b].trim().to_string())
}

pub fn import_mobaxterm(raw: &str) -> Result<ImportResult> {
    let mut devices = Vec::new();
    let mut warnings = Vec::new();
    for line in raw.lines() {
        // Bookmarks: sesssion name # ... # 0 # host # port
        if !line.to_ascii_lowercase().contains("#ssh") && !line.contains("# 0 #") {
            continue;
        }
        let parts: Vec<&str> = line.split('#').map(|s| s.trim()).collect();
        if parts.len() < 5 {
            continue;
        }
        let name = parts[0].rsplit('=').next().unwrap_or("session").to_string();
        let host = parts.iter().find(|p| p.contains('.')).copied().unwrap_or("");
        if host.is_empty() {
            continue;
        }
        devices.push(Device::new_ssh(&name, host, Vendor::Generic));
    }
    if devices.is_empty() {
        warnings.push("no MobaXterm SSH bookmarks parsed".into());
    }
    let _ = DeviceKind::Ssh;
    Ok(ImportResult { devices, warnings })
}

pub fn import_mtputty(raw: &str) -> Result<ImportResult> {
    import_securecrt_xml(raw)
}
