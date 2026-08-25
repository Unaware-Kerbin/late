//! Staging jail for CLI / Ansible / Netmiko / Salt / Chef drafts.
//! The helper may only `save`. Operator Push (`stage.push`) runs on the operator's
//! computer: CLI is typed into an open session; other formats spawn PATH tools.

use crate::config::LatePaths;
use crate::confine::safe_export_stem;
use crate::error::{LateError, Result};
use crate::fsutil;
use crate::types::Vendor;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

const FORBIDDEN: &[&str] = &[
    "password",
    "passwd",
    "ansible_ssh_pass",
    "ansible_become_pass",
    "ansible_sudo_pass",
    "community_string",
    "snmpv3",
    "private_key",
    "begin rsa",
    "begin openssh",
    "aws_secret",
    "client_secret",
    "api_key",
    "sshpass",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageFormat {
    Cli,
    Ansible,
    Netmiko,
    Salt,
    Chef,
}

impl StageFormat {
    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "cli" | "ios" | "set" => Ok(Self::Cli),
            "ansible" | "playbook" | "yaml" => Ok(Self::Ansible),
            "netmiko" | "python" => Ok(Self::Netmiko),
            "salt" | "sls" => Ok(Self::Salt),
            "chef" | "recipe" => Ok(Self::Chef),
            other => Err(LateError::Message(format!("unknown stage format: {other}"))),
        }
    }

    pub fn ext(self) -> &'static str {
        match self {
            Self::Cli => "txt",
            Self::Ansible | Self::Salt => "yml",
            Self::Netmiko => "py",
            Self::Chef => "rb",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Ansible => "ansible",
            Self::Netmiko => "netmiko",
            Self::Salt => "salt",
            Self::Chef => "chef",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageArtifact {
    pub id: String,
    pub format: StageFormat,
    pub vendor: String,
    pub intent: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask_operator: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageMeta {
    pub id: String,
    pub format: StageFormat,
    pub vendor: String,
    pub intent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask_operator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

pub fn staging_dir(paths: &LatePaths) -> PathBuf {
    paths.data.join("staging")
}

pub fn strip_secrets(text: &str) -> Result<String> {
    let mut out = String::new();
    for line in text.lines() {
        let lower = line.to_ascii_lowercase();
        if FORBIDDEN.iter().any(|k| lower.contains(k)) {
            return Err(LateError::Message(
                "draft contains a credential-like field; remove passwords, keys, and community strings"
                    .into(),
            ));
        }
        out.push_str(line);
        out.push('\n');
    }
    Ok(out)
}

pub fn platform_map(vendor: Vendor) -> (&'static str, &'static str, &'static str) {
    match vendor {
        Vendor::CiscoIos => ("cisco.ios.ios", "cisco_ios", "ios"),
        Vendor::CiscoIosXe => ("cisco.ios.ios", "cisco_xe", "ios"),
        Vendor::CiscoNxos => ("cisco.nxos.nxos", "cisco_nxos", "nxos"),
        Vendor::AristaEos => ("arista.eos.eos", "arista_eos", "eos"),
        Vendor::Junos => ("junipernetworks.junos.junos", "juniper_junos", "junos"),
        Vendor::AosCx => ("arubanetworks.aoscx", "aruba_os", "aoscx"),
        Vendor::Panos => ("paloaltonetworks.panos", "paloalto_panos", "panos"),
        Vendor::Fortios => ("fortinet.fortios", "fortinet", "fortios"),
        Vendor::Routeros => ("community.routeros", "mikrotik_routeros", "routeros"),
        Vendor::Linux => ("ansible.builtin", "linux", "linux"),
        Vendor::Generic => ("unknown", "unknown", "unknown"),
    }
}

pub fn render(
    format: StageFormat,
    vendor: Vendor,
    intent: &str,
    body: Option<&str>,
) -> Result<StageArtifact> {
    let intent = intent.trim();
    if intent.is_empty() && body.unwrap_or("").trim().is_empty() {
        return Err(LateError::Message("intent or body is required".into()));
    }
    let generic = vendor == Vendor::Generic;
    let ask_operator = if generic {
        Some(
            "This device is still Generic. Set Vendor/OS on the inventory item, or paste the draft yourself. Late will not assume Cisco IOS."
                .into(),
        )
    } else {
        None
    };
    // CLI on Generic has no safe session syntax — do not invent Cisco IOS.
    // Ansible / Netmiko / Salt / Chef still get a vendor-neutral review scaffold.
    if generic && format == StageFormat::Cli && body.unwrap_or("").trim().is_empty() {
        return Ok(StageArtifact {
            id: String::new(),
            format,
            vendor: vendor.as_str().into(),
            intent: intent.into(),
            body: String::new(),
            device_id: None,
            session_id: None,
            ask_operator,
        });
    }
    let rendered = if let Some(b) = body.map(str::trim).filter(|s| !s.is_empty()) {
        strip_secrets(b)?
    } else {
        strip_secrets(&scaffold(format, vendor, intent))?
    };
    Ok(StageArtifact {
        id: String::new(),
        format,
        vendor: vendor.as_str().into(),
        intent: intent.into(),
        body: rendered,
        device_id: None,
        session_id: None,
        ask_operator,
    })
}

fn yaml_name(intent: &str) -> String {
    intent.replace(['\n', '\r', ':'], " ")
}

fn scaffold(format: StageFormat, vendor: Vendor, intent: &str) -> String {
    let intent_c = yaml_name(intent);
    match format {
        StageFormat::Cli => cli_scaffold(vendor, &intent_c),
        StageFormat::Ansible => ansible_scaffold(vendor, &intent_c),
        StageFormat::Netmiko => netmiko_scaffold(vendor, intent),
        StageFormat::Salt => salt_scaffold(vendor, intent),
        StageFormat::Chef => chef_scaffold(vendor, intent),
    }
}

fn ansible_scaffold(vendor: Vendor, intent: &str) -> String {
    let header = "# Late staging draft. Operator Push from Staging runs ansible-playbook on your computer.\n# The helper cannot Push. Auth is the device login Late already has on your computer — do not add login vars.\n";
    if vendor == Vendor::Generic {
        return format!(
            "---\n{header}# Set Vendor/OS on the inventory item before you fill ansible_network_os.\n# Do not guess an OS.\n- name: {intent}\n  hosts: late_targets\n  gather_facts: false\n  tasks:\n    - name: apply intended change\n      ansible.netcommon.cli_config:\n        config: |\n          # Replace with vendor syntax for: {intent}\n"
        );
    }
    if vendor == Vendor::Linux {
        return format!(
            "---\n{header}- name: {intent}\n  hosts: late_targets\n  gather_facts: true\n  tasks:\n    - name: apply intended change\n      ansible.builtin.debug:\n        msg: replace with the real task for {intent}\n"
        );
    }
    let (network_os, _, _) = platform_map(vendor);
    format!(
        "---\n{header}- name: {intent}\n  hosts: late_targets\n  gather_facts: false\n  vars:\n    ansible_network_os: {network_os}\n    ansible_connection: network_cli\n  tasks:\n    - name: apply intended change\n      ansible.netcommon.cli_config:\n        config: |\n          # Replace with vendor syntax for: {intent}\n"
    )
}

fn netmiko_scaffold(vendor: Vendor, intent: &str) -> String {
    let header = "#!/usr/bin/env python3\n# Late staging draft. Operator Push from Staging runs this with python3 on your computer.\n# The helper cannot Push. Env: LATE_HOST LATE_USER LATE_KEY_FILE LATE_PORT LATE_DEVICE_TYPE.\n# Runtime auth is injected by Late on your computer. Do not hard-code credentials.\n";
    if vendor == Vendor::Generic {
        return format!(
            "{header}INTENT = {intent:?}\n# DEVICE_TYPE: set Vendor/OS on the inventory item first. Do not guess an OS.\nDEVICE_TYPE = \"\"  # operator fills\n# from netmiko import ConnectHandler\n# conn = ConnectHandler(device_type=DEVICE_TYPE, host=host, username=user, use_keys=True)\n# conn.send_config_set([\n#     # vendor syntax for INTENT\n# ])\nprint(\"review-only draft; Late did not run this\")\n"
        );
    }
    let (_, device_type, _) = platform_map(vendor);
    format!(
        "{header}INTENT = {intent:?}\nDEVICE_TYPE = {device_type:?}\n# from netmiko import ConnectHandler\n# conn = ConnectHandler(device_type=DEVICE_TYPE, host=host, username=user, use_keys=True)\n# conn.send_config_set([\n#     # vendor syntax for INTENT\n# ])\nprint(\"review-only draft; Late did not run this\")\n"
    )
}

fn salt_scaffold(vendor: Vendor, intent: &str) -> String {
    let header = "# Late staging draft. Operator Push from Staging runs salt-call --local on your computer.\n# The helper cannot Push. Do not put credentials in this file.\n";
    if vendor == Vendor::Generic {
        return format!(
            "{header}# Set Vendor/OS on the inventory item before you pick a napalm driver.\n# Do not guess an OS.\nlate_review:\n  cmd.run:\n    - name: echo {intent:?}\n    # Replace with the real state for this intent after you set the OS.\n"
        );
    }
    let (_, _, napalm) = platform_map(vendor);
    format!(
        "{header}# napalm driver hint: {napalm}\nlate_review:\n  cmd.run:\n    - name: echo {intent:?}\n    # Replace with the real state for this intent.\n"
    )
}

fn chef_scaffold(vendor: Vendor, intent: &str) -> String {
    let header = "# Late staging draft. Operator Push from Staging runs chef-apply on your computer.\n# The helper cannot Push. Do not add login vars.\n";
    if vendor == Vendor::Generic {
        return format!(
            "{header}# Set Vendor/OS on the inventory item first. Do not guess an OS.\nlog 'late-review' do\n  message {intent:?}\n  level :info\nend\n# Replace with real resources for this intent after the OS is known.\n"
        );
    }
    let (_, _, napalm) = platform_map(vendor);
    format!(
        "{header}# platform hint: {napalm}\nlog 'late-review' do\n  message {intent:?}\n  level :info\nend\n# Replace with real resources for this intent.\n"
    )
}

fn cli_scaffold(vendor: Vendor, intent: &str) -> String {
    let note = format!(
        "! intent: {intent}\n! review before Push — Late will not write memory or reboot for you\n"
    );
    match vendor {
        Vendor::CiscoIos
        | Vendor::CiscoIosXe
        | Vendor::CiscoNxos
        | Vendor::AristaEos
        | Vendor::AosCx => {
            format!("{note}configure terminal\n! paste vendor syntax here\nend\n")
        }
        Vendor::Junos => {
            format!(
                "{note}configure\n# paste set/delete lines here\n# commit only after you review\n"
            )
        }
        Vendor::Panos => format!("{note}configure\n# paste set lines; commit is operator-only\n"),
        Vendor::Fortios => {
            format!("{note}config system console\nend\n# replace with the real config block\n")
        }
        Vendor::Routeros => format!("{note}# /interface ...\n"),
        Vendor::Linux => format!("# {intent}\n# sudo ...\n"),
        Vendor::Generic => String::new(),
    }
}

fn artifact_path(paths: &LatePaths, id: &str, format: StageFormat) -> Result<PathBuf> {
    let stem = safe_export_stem(id)?;
    let dir = staging_dir(paths);
    fsutil::mkdir_private(&dir)?;
    let path = dir.join(format!("{stem}.{}", format.ext()));
    let root = dir.canonicalize().unwrap_or(dir.clone());
    if let Ok(canon) = path.canonicalize() {
        if !canon.starts_with(&root) {
            return Err(LateError::Message("staging path escaped jail".into()));
        }
    } else {
        match path.parent().and_then(|p| p.canonicalize().ok()) {
            Some(p) if p.starts_with(&root) => {}
            _ => return Err(LateError::Message("staging path escaped jail".into())),
        }
    }
    Ok(path)
}

pub fn save(paths: &LatePaths, mut art: StageArtifact) -> Result<StageArtifact> {
    if art.id.is_empty() {
        art.id = Uuid::new_v4().to_string();
    }
    if !art.body.trim().is_empty() {
        art.body = strip_secrets(&art.body)?;
    }
    let path = artifact_path(paths, &art.id, art.format)?;
    let header = format!(
        "# late-stage format={} vendor={} intent={}\n",
        art.format.as_str(),
        art.vendor,
        art.intent.replace('\n', " ")
    );
    fsutil::write_private(&path, format!("{header}{}", art.body))?;
    let meta = StageMeta {
        id: art.id.clone(),
        format: art.format,
        vendor: art.vendor.clone(),
        intent: art.intent.clone(),
        ask_operator: art.ask_operator.clone(),
        device_id: art.device_id.clone(),
        session_id: art.session_id.clone(),
    };
    fsutil::write_private(
        &path.with_extension("meta.json"),
        serde_json::to_vec_pretty(&meta).map_err(|e| LateError::Message(e.to_string()))?,
    )?;
    Ok(art)
}

pub fn get(paths: &LatePaths, id: &str) -> Result<StageArtifact> {
    let dir = staging_dir(paths);
    let stem = safe_export_stem(id)?;
    let meta_path = dir.join(format!("{stem}.meta.json"));
    let meta: StageMeta = serde_json::from_slice(&fs::read(&meta_path)?)?;
    let body_path = artifact_path(paths, &meta.id, meta.format)?;
    let raw = fs::read_to_string(&body_path)?;
    let body = raw
        .lines()
        .skip_while(|l| l.starts_with("# late-stage"))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(StageArtifact {
        id: meta.id,
        format: meta.format,
        vendor: meta.vendor,
        intent: meta.intent,
        body: if body.ends_with('\n') || body.is_empty() {
            body
        } else {
            format!("{body}\n")
        },
        device_id: meta.device_id,
        session_id: meta.session_id,
        ask_operator: meta.ask_operator,
    })
}

pub fn list(paths: &LatePaths) -> Result<Vec<StageMeta>> {
    let dir = staging_dir(paths);
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for ent in fs::read_dir(&dir)? {
        let path = ent?.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !name.ends_with(".meta.json") {
            continue;
        }
        if let Ok(meta) = serde_json::from_slice::<StageMeta>(&fs::read(&path)?) {
            out.push(meta);
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

const GENERATED_FORBIDDEN: &[&str] = &[
    "ansible_ssh_pass",
    "ansible_become_pass",
    "ansible_sudo_pass",
    "ansible_password",
    "sshpass",
    "password=",
    "passwd=",
];

const RUN_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone)]
pub struct PushTarget {
    pub name: String,
    pub vendor: Vendor,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub key_path: Option<PathBuf>,
    pub use_agent: bool,
    /// True when the daemon vault has a password for this profile. Never written to files.
    pub has_vault_password: bool,
    pub auth_profile_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagePlan {
    pub format: StageFormat,
    pub argv: Vec<String>,
    pub display: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagePushResult {
    pub ok: bool,
    pub format: StageFormat,
    pub display: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub stdout_tail: String,
    pub stderr_tail: String,
}

pub fn reject_generated_secrets(text: &str) -> Result<()> {
    for line in text.lines() {
        let lower = line.to_ascii_lowercase();
        if GENERATED_FORBIDDEN.iter().any(|k| lower.contains(k)) {
            return Err(LateError::Message(
                "generated file would contain a password-like field; Late will not write it. Add an SSH key or agent on your computer."
                    .into(),
            ));
        }
    }
    Ok(())
}

fn missing_tool(name: &str) -> LateError {
    LateError::Message(format!(
        "{name} is not installed on your computer. Install {name}; Late does not bundle it."
    ))
}

fn find_bin_in(names: &[&str], path_var: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let path_var = path_var?;
    for dir in std::env::split_paths(path_var) {
        for name in names {
            let p = dir.join(name);
            if p.is_file() {
                return Some(p);
            }
            #[cfg(windows)]
            {
                let p = dir.join(format!("{name}.exe"));
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    None
}

fn find_bin(names: &[&str]) -> Option<PathBuf> {
    find_bin_in(names, std::env::var_os("PATH").as_deref())
}

fn tail(s: &str) -> String {
    const MAX: usize = 4000;
    if s.len() <= MAX {
        s.to_string()
    } else {
        s[s.len() - MAX..].to_string()
    }
}

/// Ansible extra-var: read SSHPASS from the child env. The secret itself is never on argv.
const ANSIBLE_PASSWORD_FROM_ENV: &str = "ansible_password={{ lookup('env','SSHPASS') }}";

fn sshpass_prefix() -> Result<Vec<String>> {
    let sshpass = find_bin(&["sshpass"]).ok_or_else(|| missing_tool("sshpass"))?;
    Ok(vec![sshpass.to_string_lossy().into_owned(), "-e".into()])
}

fn run_timed(
    mut cmd: Command,
    timeout: Duration,
    runtime_password: Option<&str>,
) -> Result<(i32, String, String)> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("ANSIBLE_SSH_PASS")
        .env_remove("ANSIBLE_BECOME_PASS");
    // Do not inherit an ambient SSHPASS. Inject only when the daemon vault supplied one.
    if let Some(pw) = runtime_password.filter(|s| !s.is_empty()) {
        cmd.env("SSHPASS", pw);
        cmd.env("LATE_SSH_PASSWORD", pw);
    } else {
        cmd.env_remove("SSHPASS");
        cmd.env_remove("LATE_SSH_PASSWORD");
    }
    let mut child = cmd.spawn()?;
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let out_h = thread::spawn(move || {
        let mut b = String::new();
        if let Some(ref mut s) = stdout {
            let _ = s.read_to_string(&mut b);
        }
        b
    });
    let err_h = thread::spawn(move || {
        let mut b = String::new();
        if let Some(ref mut s) = stderr {
            let _ = s.read_to_string(&mut b);
        }
        b
    });
    let start = Instant::now();
    loop {
        match child.try_wait()? {
            Some(st) => {
                let stdout = out_h.join().unwrap_or_default();
                let stderr = err_h.join().unwrap_or_default();
                return Ok((st.code().unwrap_or(-1), tail(&stdout), tail(&stderr)));
            }
            None if start.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(LateError::Message(
                    "command timed out on your computer".into(),
                ));
            }
            None => thread::sleep(Duration::from_millis(40)),
        }
    }
}

pub fn ansible_inventory(target: &PushTarget) -> Result<String> {
    if target.vendor == Vendor::Generic {
        return Err(LateError::Message(
            "This device is still Generic. Set Vendor/OS on the inventory item on your computer. Late will not assume Cisco IOS."
                .into(),
        ));
    }
    if target.host.trim().is_empty() {
        return Err(LateError::Message(
            "Fill inventory on your computer: this device has no SSH host.".into(),
        ));
    }
    if target.username.trim().is_empty() {
        return Err(LateError::Message(
            "Add an auth profile with a username and SSH key or agent on your computer.".into(),
        ));
    }
    if target.key_path.is_none() && !target.use_agent && !target.has_vault_password {
        return Err(LateError::Message(
            "Late will not put a password in generated files. Add an SSH key or enable the agent on the auth profile on your computer."
                .into(),
        ));
    }
    let conn = if target.vendor == Vendor::Linux {
        "ssh"
    } else {
        "network_cli"
    };
    let mut line = format!(
        "{} ansible_host={} ansible_user={} ansible_port={} ansible_connection={}",
        sanitize_inv_name(&target.name),
        target.host.trim(),
        target.username.trim(),
        target.port,
        conn
    );
    if target.vendor != Vendor::Linux {
        let (network_os, _, _) = platform_map(target.vendor);
        line.push_str(&format!(" ansible_network_os={network_os}"));
    }
    if let Some(key) = &target.key_path {
        line.push_str(&format!(
            " ansible_ssh_common_args='-o IdentitiesOnly=yes -o IdentityFile={}'",
            key.display()
        ));
    }
    let body = format!(
        "# Generated by Late for operator Push on your computer. User and key/agent only. No passwords in this file.\n[late_targets]\n{line}\n"
    );
    reject_generated_secrets(&body)?;
    Ok(body)
}

fn sanitize_inv_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() {
        "late_device".into()
    } else {
        s
    }
}

fn require_ssh_target(format: StageFormat, target: Option<&PushTarget>) -> Result<&PushTarget> {
    let t = target.ok_or_else(|| {
        LateError::Message(
            "Pick an SSH inventory device on your computer (hostname/IP). Push session is only for Push CLI into an open terminal.".into(),
        )
    })?;
    if t.vendor == Vendor::Generic {
        return Err(LateError::Message(
            "This device is still Generic. Set Vendor/OS on the inventory item on your computer. Late will not assume Cisco IOS."
                .into(),
        ));
    }
    if t.host.trim().is_empty() {
        return Err(LateError::Message(
            "Fill inventory on your computer: this device has no SSH host.".into(),
        ));
    }
    match format {
        StageFormat::Ansible | StageFormat::Netmiko => {
            ansible_inventory(t)?;
        }
        _ => {}
    }
    Ok(t)
}

pub fn plan_push(
    paths: &LatePaths,
    art: &StageArtifact,
    target: Option<&PushTarget>,
) -> Result<StagePlan> {
    if art.body.trim().is_empty() {
        return Err(LateError::Message("Nothing to push.".into()));
    }
    strip_secrets(&art.body)?;
    match art.format {
        StageFormat::Cli => Ok(StagePlan {
            format: art.format,
            argv: vec!["session.input".into()],
            display: "type draft into the selected SSH/serial session".into(),
            file: None,
            binary: None,
        }),
        StageFormat::Ansible => {
            let t = require_ssh_target(art.format, target)?;
            let play = artifact_path(paths, &art.id, art.format)?;
            let inv = play.with_extension("inventory.ini");
            fsutil::write_private(&inv, ansible_inventory(t)?)?;
            let bin =
                find_bin(&["ansible-playbook"]).ok_or_else(|| missing_tool("ansible-playbook"))?;
            let mut argv = Vec::new();
            if t.has_vault_password {
                argv.extend(sshpass_prefix()?);
            }
            argv.extend([
                bin.to_string_lossy().into_owned(),
                "-i".into(),
                inv.display().to_string(),
                play.display().to_string(),
            ]);
            if t.has_vault_password {
                argv.push("-e".into());
                argv.push(ANSIBLE_PASSWORD_FROM_ENV.into());
            }
            Ok(StagePlan {
                format: art.format,
                display: argv.join(" "),
                file: Some(play.display().to_string()),
                binary: Some(bin.display().to_string()),
                argv,
            })
        }
        StageFormat::Netmiko => {
            let t = require_ssh_target(art.format, target)?;
            let script = artifact_path(paths, &art.id, art.format)?;
            let py = find_bin(&["python3", "python"]).ok_or_else(|| missing_tool("python3"))?;
            ensure_netmiko(&py)?;
            let argv = vec![
                py.to_string_lossy().into_owned(),
                script.display().to_string(),
            ];
            let _ = t;
            Ok(StagePlan {
                format: art.format,
                display: argv.join(" "),
                file: Some(script.display().to_string()),
                binary: Some(py.display().to_string()),
                argv,
            })
        }
        StageFormat::Salt => {
            let src = artifact_path(paths, &art.id, art.format)?;
            let sls = src.with_extension("sls");
            let body = fs::read_to_string(&src).unwrap_or_else(|_| art.body.clone());
            reject_generated_secrets(&body)?;
            fsutil::write_private(&sls, body)?;
            let bin = find_bin(&["salt-call", "salt-ssh", "salt"])
                .ok_or_else(|| missing_tool("salt-call"))?;
            let name = bin
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("salt-call");
            let argv = if name.starts_with("salt-call") {
                vec![
                    bin.to_string_lossy().into_owned(),
                    "--local".into(),
                    "--file-root".into(),
                    staging_dir(paths).display().to_string(),
                    "state.apply".into(),
                    art.id.clone(),
                ]
            } else if name.starts_with("salt-ssh") {
                let t = require_ssh_target(art.format, target)?;
                let mut argv = vec![
                    bin.to_string_lossy().into_owned(),
                    t.host.clone(),
                    "state.apply".into(),
                    art.id.clone(),
                ];
                if t.has_vault_password {
                    let mut wrapped = sshpass_prefix()?;
                    wrapped.append(&mut argv);
                    argv = wrapped;
                }
                argv
            } else {
                return Err(LateError::Message(
                    "salt master CLI found, but a local state file needs salt-call or salt-ssh on your computer."
                        .into(),
                ));
            };
            Ok(StagePlan {
                format: art.format,
                display: argv.join(" "),
                file: Some(sls.display().to_string()),
                binary: Some(bin.display().to_string()),
                argv,
            })
        }
        StageFormat::Chef => {
            let recipe = artifact_path(paths, &art.id, art.format)?;
            let (bin, extra) = if let Some(b) = find_bin(&["chef-apply"]) {
                (b, Vec::<String>::new())
            } else if let Some(b) = find_bin(&["chef"]) {
                (b, vec!["apply".into()])
            } else if let Some(b) = find_bin(&["chef-client"]) {
                (b, vec!["--local-mode".into(), "-z".into()])
            } else {
                return Err(missing_tool("chef-apply"));
            };
            let mut argv = vec![bin.to_string_lossy().into_owned()];
            argv.extend(extra);
            argv.push(recipe.display().to_string());
            Ok(StagePlan {
                format: art.format,
                display: argv.join(" "),
                file: Some(recipe.display().to_string()),
                binary: Some(bin.display().to_string()),
                argv,
            })
        }
    }
}

fn ensure_netmiko(py: &Path) -> Result<()> {
    let mut cmd = Command::new(py);
    cmd.args(["-c", "import netmiko"]);
    let (code, _, err) = run_timed(cmd, Duration::from_secs(8), None)?;
    if code != 0 {
        return Err(LateError::Message(format!(
            "netmiko is not installed for python3 on your computer. Install it with pip; Late does not bundle it. {err}"
        )));
    }
    Ok(())
}

pub fn run_push(
    paths: &LatePaths,
    art: &StageArtifact,
    target: Option<&PushTarget>,
    runtime_password: Option<&str>,
) -> Result<StagePushResult> {
    let plan = plan_push(paths, art, target)?;
    if art.format == StageFormat::Cli {
        return Err(LateError::Message(
            "CLI Push is typed into an open session, not a PATH tool.".into(),
        ));
    }
    if plan.argv.is_empty() {
        return Err(LateError::Message("nothing to run".into()));
    }
    let mut cmd = Command::new(&plan.argv[0]);
    cmd.args(&plan.argv[1..]);
    cmd.current_dir(staging_dir(paths));
    cmd.env("ANSIBLE_RETRY_FILES_ENABLED", "false");
    if art.format == StageFormat::Netmiko {
        if let Some(t) = target {
            cmd.env("LATE_HOST", &t.host);
            cmd.env("LATE_USER", &t.username);
            cmd.env("LATE_PORT", t.port.to_string());
            let (_, device_type, _) = platform_map(t.vendor);
            cmd.env("LATE_DEVICE_TYPE", device_type);
            if let Some(key) = &t.key_path {
                cmd.env("LATE_KEY_FILE", key);
            }
        }
    }
    // Vault password: child env only (sshpass -e / Ansible lookup). Never argv, never staging files.
    let inject = runtime_password
        .filter(|s| !s.is_empty())
        .filter(|_| target.is_some_and(|t| t.has_vault_password));
    let (code, stdout_tail, stderr_tail) = run_timed(cmd, RUN_TIMEOUT, inject)?;
    Ok(StagePushResult {
        ok: code == 0,
        format: art.format,
        display: plan.display,
        exit_code: Some(code),
        stdout_tail,
        stderr_tail,
    })
}

pub fn jail_ok(root: &Path, candidate: &Path) -> bool {
    let Ok(root) = root.canonicalize() else {
        return false;
    };
    candidate
        .canonicalize()
        .map(|p| p.starts_with(&root))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> (tempfile::TempDir, LatePaths) {
        let t = tempfile::tempdir().unwrap();
        let paths = LatePaths {
            config: t.path().join("c"),
            data: t.path().join("d"),
        };
        fsutil::mkdir_private(&paths.data).unwrap();
        (t, paths)
    }

    #[test]
    fn rejects_password_lines() {
        let err =
            strip_secrets("set system root-authentication password-secret foo\n").unwrap_err();
        assert!(err.to_string().contains("credential"));
    }

    #[test]
    fn generic_cli_asks_operator() {
        let a = render(StageFormat::Cli, Vendor::Generic, "vlan 10", None).unwrap();
        assert!(a.ask_operator.is_some());
        assert!(a.body.is_empty());
    }

    #[test]
    fn generic_non_cli_has_scaffold_not_cisco() {
        for format in [
            StageFormat::Ansible,
            StageFormat::Netmiko,
            StageFormat::Salt,
            StageFormat::Chef,
        ] {
            let a = render(format, Vendor::Generic, "ntp servers", None).unwrap();
            assert!(a.ask_operator.is_some(), "{format:?}");
            assert!(!a.body.is_empty(), "{format:?}");
            let l = a.body.to_ascii_lowercase();
            assert!(!l.contains("cisco.ios"), "{format:?}");
            assert!(!l.contains("cisco_ios"), "{format:?}");
            assert!(!l.contains("configure terminal"), "{format:?}");
            assert!(!l.contains("password"), "{format:?}");
            assert!(!l.contains("sshpass"), "{format:?}");
            assert!(!l.contains("ansible_ssh_pass"), "{format:?}");
        }
    }

    #[test]
    fn generic_ansible_looks_like_playbook() {
        let a = render(StageFormat::Ansible, Vendor::Generic, "ntp servers", None).unwrap();
        assert!(a.ask_operator.is_some());
        assert!(a.body.contains("hosts:"));
        assert!(a.body.contains("tasks:"));
        assert!(a.body.contains("ansible.netcommon.cli_config") || a.body.contains("cli_config"));
    }

    #[test]
    fn generic_netmiko_looks_like_python() {
        let a = render(StageFormat::Netmiko, Vendor::Generic, "ntp servers", None).unwrap();
        assert!(a.body.contains("netmiko") || a.body.contains("ConnectHandler"));
        assert!(a.body.contains("INTENT"));
        assert!(
            a.body.to_ascii_lowercase().contains("operator fills")
                || a.body.contains("DEVICE_TYPE = \"\"")
        );
    }

    #[test]
    fn generic_salt_looks_like_state() {
        let a = render(StageFormat::Salt, Vendor::Generic, "ntp servers", None).unwrap();
        assert!(a.body.contains("cmd.run") || a.body.contains("late_review:"));
    }

    #[test]
    fn generic_chef_looks_like_recipe() {
        let a = render(StageFormat::Chef, Vendor::Generic, "ntp servers", None).unwrap();
        assert!(a.body.contains("do"));
        assert!(a.body.contains("end"));
    }

    #[test]
    fn cisco_cli_has_no_reload() {
        let a = render(StageFormat::Cli, Vendor::CiscoIos, "mtu 9000", None).unwrap();
        let l = a.body.to_ascii_lowercase();
        assert!(l.contains("configure"));
        assert!(!l.contains("reload"));
        assert!(!l.contains("write erase"));
    }

    #[test]
    fn ansible_has_no_ssh_pass() {
        let a = render(StageFormat::Ansible, Vendor::Junos, "mtu", None).unwrap();
        assert!(a.body.contains("junipernetworks.junos"));
        assert!(!a.body.to_ascii_lowercase().contains("ansible_ssh_pass"));
    }

    #[test]
    fn scaffolds_have_no_forbidden_tokens() {
        for vendor in [
            Vendor::CiscoIos,
            Vendor::Junos,
            Vendor::Linux,
            Vendor::AristaEos,
        ] {
            for format in [
                StageFormat::Cli,
                StageFormat::Ansible,
                StageFormat::Netmiko,
                StageFormat::Salt,
                StageFormat::Chef,
            ] {
                let a = render(format, vendor, "ntp servers", None).unwrap();
                assert!(a.ask_operator.is_none(), "{format:?} {vendor:?}");
                assert!(!a.body.is_empty());
            }
        }
    }

    #[test]
    fn save_roundtrip_and_jail() {
        let (_t, paths) = tmp();
        let mut a = render(StageFormat::Salt, Vendor::Linux, "ntp", None).unwrap();
        a = save(&paths, a).unwrap();
        let loaded = get(&paths, &a.id).unwrap();
        assert!(loaded.body.contains("ntp"));
        let listed = list(&paths).unwrap();
        assert_eq!(listed.len(), 1);
        let body_path = staging_dir(&paths).join(format!("{}.{}", a.id, a.format.ext()));
        assert!(body_path.exists());
        assert!(jail_ok(&staging_dir(&paths), &body_path));
    }

    fn key_target(vendor: Vendor, host: &str) -> PushTarget {
        PushTarget {
            name: "r1".into(),
            vendor,
            host: host.into(),
            port: 22,
            username: "netops".into(),
            key_path: Some(PathBuf::from("/home/op/.ssh/id_ed25519")),
            use_agent: false,
            has_vault_password: false,
            auth_profile_id: None,
        }
    }

    fn password_target(vendor: Vendor) -> PushTarget {
        PushTarget {
            name: "aruba1".into(),
            vendor,
            host: "192.0.2.80".into(),
            port: 22,
            username: "admin".into(),
            key_path: None,
            use_agent: false,
            has_vault_password: true,
            auth_profile_id: Some("prof-aruba".into()),
        }
    }

    fn with_fake_bins<T>(names: &[&str], f: impl FnOnce() -> T) -> T {
        let dir = tempfile::tempdir().unwrap();
        for name in names {
            let p = dir.path().join(name);
            std::fs::write(&p, "#!/bin/sh\nexit 0\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = std::fs::metadata(&p).unwrap().permissions();
                perms.set_mode(0o755);
                std::fs::set_permissions(&p, perms).unwrap();
            }
        }
        let old = std::env::var("PATH").ok();
        std::env::set_var("PATH", dir.path());
        let out = f();
        match old {
            Some(v) => std::env::set_var("PATH", v),
            None => std::env::remove_var("PATH"),
        }
        out
    }

    #[test]
    fn generated_inventory_rejects_password_keys() {
        let t = key_target(Vendor::Junos, "192.0.2.1");
        let inv = ansible_inventory(&t).unwrap();
        let lower = inv.to_ascii_lowercase();
        assert!(!lower.contains("ansible_ssh_pass"));
        assert!(!lower.contains("sshpass"));
        assert!(!lower.contains("password="));
        assert!(inv.contains("ansible_user=netops"));
        reject_generated_secrets(&inv).unwrap();
        assert!(reject_generated_secrets("ansible_ssh_pass=secret\n").is_err());
    }

    #[test]
    fn password_only_inventory_has_no_secret_fields() {
        let t = password_target(Vendor::AosCx);
        let inv = ansible_inventory(&t).unwrap();
        let lower = inv.to_ascii_lowercase();
        assert!(!lower.contains("ansible_ssh_pass"), "{inv}");
        assert!(!lower.contains("ansible_password"), "{inv}");
        assert!(!lower.contains("sshpass"), "{inv}");
        assert!(!lower.contains("password="), "{inv}");
        assert!(inv.contains("ansible_user=admin"));
        assert!(inv.contains("arubanetworks.aoscx") || inv.contains("aruba"));
        assert!(!inv.to_ascii_lowercase().contains("cisco.ios"));
        reject_generated_secrets(&inv).unwrap();
    }

    #[test]
    fn password_only_without_vault_still_refused() {
        let mut t = password_target(Vendor::AosCx);
        t.has_vault_password = false;
        let err = ansible_inventory(&t).unwrap_err().to_string();
        assert!(
            err.contains("will not put a password in generated files"),
            "{err}"
        );
    }

    #[test]
    fn password_only_plan_does_not_write_secret_to_staging() {
        let (_t, paths) = tmp();
        let mut a = render(StageFormat::Ansible, Vendor::AosCx, "ntp", None).unwrap();
        a = save(&paths, a).unwrap();
        let target = password_target(Vendor::AosCx);
        let plan = with_fake_bins(&["ansible-playbook", "sshpass"], || {
            plan_push(&paths, &a, Some(&target)).unwrap()
        });
        assert!(
            plan.argv.iter().any(|s| s.contains("sshpass")),
            "password PATH Push wraps sshpass -e: {:?}",
            plan.argv
        );
        assert!(
            plan.argv.iter().any(|s| s.contains("lookup('env'")),
            "ansible must read SSHPASS from env, not files: {:?}",
            plan.argv
        );
        assert!(
            !plan.argv.join(" ").contains("hunter2"),
            "password must not appear on argv"
        );
        let play = PathBuf::from(plan.file.as_ref().unwrap());
        let inv = std::fs::read_to_string(play.with_extension("inventory.ini")).unwrap();
        let play_body = std::fs::read_to_string(&play).unwrap();
        for text in [&inv, &play_body, &a.body] {
            let lower = text.to_ascii_lowercase();
            assert!(!lower.contains("ansible_ssh_pass"), "{text}");
            assert!(!lower.contains("ansible_password"), "{text}");
            assert!(!text.contains("hunter2"));
        }
        reject_generated_secrets(&inv).unwrap();
        reject_generated_secrets(&play_body).unwrap();
    }

    #[test]
    fn password_only_ansible_requires_sshpass() {
        let (_t, paths) = tmp();
        let mut a = render(StageFormat::Ansible, Vendor::AosCx, "ntp", None).unwrap();
        a = save(&paths, a).unwrap();
        let target = password_target(Vendor::AosCx);
        let err = with_fake_bins(&["ansible-playbook"], || {
            plan_push(&paths, &a, Some(&target))
                .unwrap_err()
                .to_string()
        });
        assert!(err.contains("sshpass"), "{err}");
        assert!(err.contains("your computer"), "{err}");
        let dir = staging_dir(&paths);
        for ent in std::fs::read_dir(&dir).unwrap() {
            let p = ent.unwrap().path();
            if p.extension().and_then(|e| e.to_str()) == Some("ini") {
                let inv = std::fs::read_to_string(&p).unwrap();
                assert!(!inv.to_ascii_lowercase().contains("ansible_ssh_pass"));
            }
        }
    }

    #[test]
    fn save_rejects_ansible_ssh_pass_in_artifacts() {
        let (_t, paths) = tmp();
        let mut a = render(StageFormat::Ansible, Vendor::AosCx, "ntp", None).unwrap();
        a.body = "hosts: all\n  vars:\n    ansible_ssh_pass: hunter2\n".into();
        let err = save(&paths, a).unwrap_err().to_string();
        assert!(
            err.contains("credential") || err.contains("password"),
            "{err}"
        );
        assert!(reject_generated_secrets("ansible_ssh_pass=secret\n").is_err());
    }

    #[test]
    fn generic_target_does_not_assume_cisco() {
        let t = PushTarget {
            name: "x".into(),
            vendor: Vendor::Generic,
            host: "192.0.2.8".into(),
            port: 22,
            username: "u".into(),
            key_path: Some(PathBuf::from("/k")),
            use_agent: false,
            has_vault_password: false,
            auth_profile_id: None,
        };
        let err = ansible_inventory(&t).unwrap_err().to_string();
        assert!(err.contains("Generic") || err.contains("Vendor"));
        let l = err.to_ascii_lowercase();
        assert!(!l.contains("cisco.ios"), "{err}");
        assert!(!l.contains("cisco_ios"), "{err}");
        let (_t, paths) = tmp();
        let mut a = render(StageFormat::Netmiko, Vendor::Generic, "ntp", None).unwrap();
        a = save(&paths, a).unwrap();
        let err = plan_push(&paths, &a, Some(&t)).unwrap_err().to_string();
        let l = err.to_ascii_lowercase();
        assert!(!l.contains("cisco_ios"), "{err}");
        assert!(l.contains("generic") || l.contains("vendor"));
    }

    #[test]
    fn missing_binary_fails_fast() {
        let (_t, paths) = tmp();
        let mut a = render(StageFormat::Chef, Vendor::Linux, "ntp", None).unwrap();
        a = save(&paths, a).unwrap();
        let old = std::env::var("PATH").ok();
        std::env::set_var("PATH", "/var/empty/late-no-such-bins");
        let start = Instant::now();
        let err = plan_push(&paths, &a, None).unwrap_err();
        match old {
            Some(v) => std::env::set_var("PATH", v),
            None => std::env::remove_var("PATH"),
        }
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "missing binary should fail immediately"
        );
        let m = err.to_string();
        assert!(
            m.contains("not installed") || m.contains("does not bundle"),
            "{m}"
        );
    }
}
