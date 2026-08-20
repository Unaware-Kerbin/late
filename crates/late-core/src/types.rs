use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Vendor {
    Junos,
    CiscoIos,
    CiscoIosXe,
    CiscoNxos,
    AristaEos,
    Panos,
    Linux,
    Fortios,
    Routeros,
    AosCx,
    Generic,
}

impl Vendor {
    pub fn as_str(&self) -> &'static str {
        match self {
            Vendor::Junos => "junos",
            Vendor::CiscoIos => "cisco_ios",
            Vendor::CiscoIosXe => "cisco_ios_xe",
            Vendor::CiscoNxos => "cisco_nxos",
            Vendor::AristaEos => "arista_eos",
            Vendor::Panos => "panos",
            Vendor::Linux => "linux",
            Vendor::Fortios => "fortios",
            Vendor::Routeros => "routeros",
            Vendor::AosCx => "aos_cx",
            Vendor::Generic => "generic",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "junos" | "juniper" => Vendor::Junos,
            "cisco_ios" | "ios" | "cisco" => Vendor::CiscoIos,
            "cisco_ios_xe" | "ios-xe" | "iosxe" | "xe" => Vendor::CiscoIosXe,
            "cisco_nxos" | "nxos" | "nexus" => Vendor::CiscoNxos,
            "arista_eos" | "eos" | "arista" => Vendor::AristaEos,
            "panos" | "paloalto" | "pan-os" => Vendor::Panos,
            "linux" | "unix" => Vendor::Linux,
            "fortios" | "fortigate" => Vendor::Fortios,
            "routeros" | "mikrotik" => Vendor::Routeros,
            "aos_cx" | "aos-cx" | "aoscx" | "aruba" | "arubaos-cx" => Vendor::AosCx,
            _ => Vendor::Generic,
        }
    }

    /// Banner / MOTD / `show version` text. A non-Generic inventory vendor is sticky.
    pub fn infer_from_text(text: &str, hint: Self) -> Self {
        if hint != Self::Generic {
            return hint;
        }
        Self::detect_from_text(text).unwrap_or(hint)
    }

    pub fn detect_from_text(text: &str) -> Option<Self> {
        let t = text.to_ascii_lowercase();
        if t.contains("aos-cx")
            || t.contains("arubaos-cx")
            || t.contains("arubanetworks")
            || t.contains("hewlett packard enterprise")
            || t.contains("aruba")
        {
            return Some(Vendor::AosCx);
        }
        if t.contains("nx-os") || t.contains("nexus") {
            return Some(Vendor::CiscoNxos);
        }
        if t.contains("ios-xe") || t.contains("iosxe") || t.contains("cisco ios xe") {
            return Some(Vendor::CiscoIosXe);
        }
        if t.contains("cisco ios")
            || t.contains("ios software")
            || t.contains("cisco internetwork operating system")
        {
            return Some(Vendor::CiscoIos);
        }
        if t.contains("arista") {
            return Some(Vendor::AristaEos);
        }
        if t.contains("junos") || t.contains("juniper") {
            return Some(Vendor::Junos);
        }
        if t.contains("fortigate") || t.contains("fortios") {
            return Some(Vendor::Fortios);
        }
        if t.contains("routeros") || t.contains("mikrotik") {
            return Some(Vendor::Routeros);
        }
        if t.contains("palo alto") || t.contains("pan-os") || t.contains("panos") {
            return Some(Vendor::Panos);
        }
        None
    }
}

impl<'de> Deserialize<'de> for Vendor {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(Self::parse(&raw))
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    Ssh,
    Serial,
    Local,
    Api,
}

impl DeviceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ssh => "ssh",
            Self::Serial => "serial",
            Self::Local => "local",
            Self::Api => "api",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "ssh" => Some(Self::Ssh),
            "serial" | "tty" | "console" => Some(Self::Serial),
            "local" | "shell" => Some(Self::Local),
            "api" | "http" => Some(Self::Api),
            _ => None,
        }
    }
}

impl<'de> Deserialize<'de> for DeviceKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).ok_or_else(|| {
            serde::de::Error::unknown_variant(&raw, &["ssh", "serial", "local", "api"])
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    Ssh,
    Serial,
    Local,
    Sftp,
    Pcap,
    Api,
}

impl SessionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ssh => "ssh",
            Self::Serial => "serial",
            Self::Local => "local",
            Self::Sftp => "sftp",
            Self::Pcap => "pcap",
            Self::Api => "api",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "ssh" => Some(Self::Ssh),
            "serial" | "tty" | "console" => Some(Self::Serial),
            "local" | "shell" => Some(Self::Local),
            "sftp" => Some(Self::Sftp),
            "pcap" => Some(Self::Pcap),
            "api" | "http" => Some(Self::Api),
            _ => None,
        }
    }
}

impl<'de> Deserialize<'de> for SessionKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).ok_or_else(|| {
            serde::de::Error::unknown_variant(
                &raw,
                &["ssh", "serial", "local", "sftp", "pcap", "api"],
            )
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthProfile {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default, alias = "user", alias = "userName")]
    pub username: String,
    #[serde(default, alias = "keyPath", alias = "identity")]
    pub key_path: Option<String>,
    #[serde(default, alias = "useAgent")]
    pub use_agent: bool,
    #[serde(default, alias = "hasPassword")]
    pub has_password: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    pub vendor: Vendor,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default, alias = "serialPath", alias = "serial_port", alias = "path")]
    pub serial_path: Option<String>,
    #[serde(default)]
    pub baud: Option<u32>,
    #[serde(default)]
    pub api_base_url: Option<String>,
    #[serde(default)]
    pub api_controller: Option<String>,
    #[serde(
        default,
        alias = "authProfileId",
        alias = "auth_profile",
        alias = "profile_id"
    )]
    pub auth_profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub accent: Option<String>,
    #[serde(default)]
    pub syntax_highlight: bool,
    #[serde(default)]
    pub quick_copy: bool,
    #[serde(default)]
    pub legacy_ssh: bool,
    #[serde(default)]
    pub jump_host: Option<String>,
    #[serde(default)]
    pub shell: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

impl Device {
    pub fn new_ssh(name: &str, host: &str, vendor: Vendor) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name: name.to_string(),
            kind: DeviceKind::Ssh,
            vendor,
            host: Some(host.to_string()),
            port: Some(22),
            serial_path: None,
            baud: None,
            api_base_url: None,
            api_controller: None,
            auth_profile_id: None,
            folder: None,
            tags: vec![],
            accent: None,
            syntax_highlight: true,
            quick_copy: true,
            legacy_ssh: false,
            jump_host: None,
            shell: None,
            notes: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Inventory {
    pub devices: Vec<Device>,
    pub folders: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub device_id: Option<String>,
    pub name: String,
    pub kind: SessionKind,
    pub vendor: Vendor,
    pub connected: bool,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub accent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyDecision {
    pub allowed: bool,
    pub reason: String,
    pub expanded: String,
    pub linux_unrestricted: bool,
    /// False on Linux/generic and on any deny. Always-allow in the UI is gated on this.
    #[serde(default)]
    pub allow_always_allow: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandProposal {
    pub id: String,
    pub session_id: String,
    pub command: String,
    pub expanded: String,
    pub vendor: Vendor,
    pub decision: PolicyDecision,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiProposal {
    pub id: String,
    pub session_id: String,
    pub method: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PcapFinding {
    pub id: String,
    pub kind: String,
    pub summary: String,
    pub packet_indexes: Vec<usize>,
    pub evidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PacketSummary {
    pub index: usize,
    pub timestamp: String,
    pub src: String,
    pub dst: String,
    pub protocol: String,
    pub length: usize,
    pub info: String,
    pub fields: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureRecord {
    pub id: String,
    pub name: String,
    pub device_id: Option<String>,
    pub command: String,
    pub output: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandCollection {
    pub id: String,
    pub name: String,
    pub vendor: Option<Vendor>,
    pub commands: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CollectionsFile {
    pub collections: Vec<CommandCollection>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApiController {
    Generic,
    Unifi,
    Meraki,
    Aci,
    Vmanage,
    Apstra,
    Aruba,
    Fortimanager,
}

impl ApiController {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "unifi" | "ubiquiti" => Self::Unifi,
            "meraki" => Self::Meraki,
            "aci" | "apic" => Self::Aci,
            "vmanage" | "sdwan" => Self::Vmanage,
            "apstra" => Self::Apstra,
            "aruba" => Self::Aruba,
            "fortimanager" | "fortigate" => Self::Fortimanager,
            _ => Self::Generic,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_kind_reads_ssh_and_serial_from_toml() {
        #[derive(Deserialize)]
        struct Wrap {
            kind: DeviceKind,
        }
        for (raw, want) in [
            ("kind = \"ssh\"", DeviceKind::Ssh),
            ("kind = \"SSH\"", DeviceKind::Ssh),
            ("kind = \"Ssh\"", DeviceKind::Ssh),
            ("kind = \"serial\"", DeviceKind::Serial),
            ("kind = \"Serial\"", DeviceKind::Serial),
        ] {
            let w: Wrap = toml::from_str(raw).unwrap();
            assert_eq!(w.kind, want, "{raw}");
        }
        let json: Wrap = serde_json::from_str(r#"{"kind":"ssh"}"#).unwrap();
        assert_eq!(json.kind, DeviceKind::Ssh);
        assert_eq!(serde_json::to_value(DeviceKind::Ssh).unwrap(), "ssh");
    }

    #[test]
    fn vendor_reads_parse_aliases_from_toml() {
        #[derive(Deserialize)]
        struct Wrap {
            vendor: Vendor,
        }
        for (raw, want) in [
            ("vendor = \"aos_cx\"", Vendor::AosCx),
            ("vendor = \"aos-cx\"", Vendor::AosCx),
            ("vendor = \"aruba\"", Vendor::AosCx),
            ("vendor = \"juniper\"", Vendor::Junos),
            ("vendor = \"generic\"", Vendor::Generic),
        ] {
            let w: Wrap = toml::from_str(raw).unwrap();
            assert_eq!(w.vendor, want, "{raw}");
        }
        assert_eq!(serde_json::to_value(Vendor::AosCx).unwrap(), "aos_cx");
        assert_eq!(
            Vendor::infer_from_text("Hewlett Packard Enterprise  aruba  AOS-CX", Vendor::Generic),
            Vendor::AosCx
        );
        assert_eq!(
            Vendor::infer_from_text("", Vendor::CiscoIos),
            Vendor::CiscoIos
        );
        assert_eq!(
            Vendor::infer_from_text("aruba aos-cx CDP neighbor", Vendor::Linux),
            Vendor::Linux
        );
        assert_eq!(
            Vendor::infer_from_text("aruba aos-cx CDP neighbor", Vendor::CiscoIos),
            Vendor::CiscoIos
        );
    }

    #[test]
    fn legacy_inventory_without_folders_key_roundtrips() {
        let raw = r#"
[[devices]]
id = "ssh-1"
name = "core"
kind = "ssh"
vendor = "generic"
host = "192.0.2.1"
"#;
        let inv: Inventory = toml::from_str(raw).unwrap();
        assert_eq!(inv.devices.len(), 1);
        assert!(inv.folders.is_empty());
        assert_eq!(inv.devices[0].folder, None);
        let encoded = toml::to_string_pretty(&inv).unwrap();
        assert!(
            !encoded.lines().any(|l| {
                let t = l.trim();
                t.starts_with("folder ") || t.starts_with("folder=")
            }),
            "ungrouped devices must not write a folder key: {encoded}"
        );
        let again: Inventory = toml::from_str(&encoded).unwrap();
        assert_eq!(again.devices[0].name, "core");
        assert_eq!(again.devices[0].folder, None);
    }

    #[test]
    fn inventory_fixture_keeps_ssh_auth_profile_id() {
        let raw = r#"
folders = []

[[devices]]
id = "serial-1"
name = "Aruba AOS-CX"
kind = "serial"
vendor = "generic"
serial_path = "/dev/ttyUSB0"
baud = 115200

[[devices]]
id = "ssh-1"
name = "ssh-aos-cx"
kind = "ssh"
vendor = "generic"
host = "192.168.2.247"
port = 22
auth_profile_id = "471f3b33-a154-4152-8413-4e34290c26d2"
"#;
        let inv: Inventory = toml::from_str(raw).unwrap();
        assert_eq!(inv.devices.len(), 2);
        assert_eq!(inv.devices[0].kind, DeviceKind::Serial);
        assert_eq!(inv.devices[0].serial_path.as_deref(), Some("/dev/ttyUSB0"));
        assert_eq!(inv.devices[1].kind, DeviceKind::Ssh);
        assert_eq!(
            inv.devices[1].auth_profile_id.as_deref(),
            Some("471f3b33-a154-4152-8413-4e34290c26d2")
        );
    }

    #[test]
    fn auth_profile_toml_username_and_flags() {
        let raw = r#"
id = "471f3b33-a154-4152-8413-4e34290c26d2"
name = "ssh-aos-cx login"
username = "admin"
use_agent = false
has_password = true
"#;
        let p: AuthProfile = toml::from_str(raw).unwrap();
        assert_eq!(p.id, "471f3b33-a154-4152-8413-4e34290c26d2");
        assert_eq!(p.username, "admin");
        assert!(p.has_password);
        assert!(!p.use_agent);
    }
}
