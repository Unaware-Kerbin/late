use crate::error::Result;
use crate::types::{PolicyDecision, Vendor};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VendorPolicy {
    pub vendor: String,
    #[serde(default)]
    pub aliases: HashMap<String, String>,
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
    #[serde(default)]
    pub deny_substrings: Vec<String>,
    #[serde(default)]
    pub allow_pipes: Vec<String>,
    #[serde(default)]
    pub deny_pipes: Vec<String>,
    #[serde(default)]
    pub unrestricted: bool,
    #[serde(default)]
    pub allow_always_allow: bool,
}

/// Settings JSON for the device-CLI permit list (not MCP grant-folder).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyView {
    pub vendor: String,
    pub unrestricted: bool,
    pub allow_always_allow: bool,
    pub allow: Vec<String>,
    pub builtin_allow: Vec<String>,
    pub deny: Vec<String>,
    pub deny_substrings: Vec<String>,
    pub path: String,
}

/// Verbs denied by default; operator must add them on purpose (Settings warns).
pub const DANGEROUS_VERBS: &[&str] = &[
    "reload",
    "reboot",
    "erase",
    "write erase",
    "boot",
    "start-shell",
    "shell",
    "factory",
    "copy",
    "bash",
    "python",
    "tclsh",
    "format",
    "guestshell",
    "shutdown",
];

#[derive(Debug, Clone, Default)]
pub struct PolicyEngine {
    policies: HashMap<String, VendorPolicy>,
    /// Builtin vendor policies, kept after YAML overlays replace `policies`.
    /// Overlay deny still wins. Overlay cannot drop builtin allow (stale first-boot YAML).
    builtin: HashMap<String, VendorPolicy>,
}

impl PolicyEngine {
    pub fn load_dir(dir: &Path) -> Result<Self> {
        let mut engine = Self::builtin();
        if dir.is_dir() {
            for entry in std::fs::read_dir(dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("yaml")
                    || path.extension().and_then(|s| s.to_str()) == Some("yml")
                {
                    let raw = std::fs::read_to_string(&path)?;
                    let p: VendorPolicy = serde_yaml::from_str(&raw)?;
                    engine.policies.insert(p.vendor.clone(), p);
                }
            }
        }
        Ok(engine)
    }

    pub fn merge_dir(&mut self, dir: &Path) -> Result<()> {
        if !dir.is_dir() {
            return Ok(());
        }
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let ext = path.extension().and_then(|s| s.to_str());
            if ext == Some("yaml") || ext == Some("yml") {
                let raw = std::fs::read_to_string(&path)?;
                let p: VendorPolicy = serde_yaml::from_str(&raw)?;
                self.policies.insert(p.vendor.clone(), p);
            }
        }
        Ok(())
    }

    pub fn reload(&mut self, bundled: &Path, overlay: &Path) -> Result<()> {
        *self = Self::load_dir(bundled)?;
        self.merge_dir(overlay)?;
        Ok(())
    }

    pub fn view(&self, vendor: Vendor, overlay_dir: &Path) -> PolicyView {
        let overlay = self.get(vendor);
        let builtin_allow = self
            .builtin
            .get(vendor.as_str())
            .map(|b| {
                b.allow
                    .iter()
                    .filter(|a| {
                        !overlay
                            .allow
                            .iter()
                            .any(|o| o.eq_ignore_ascii_case(a))
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        PolicyView {
            vendor: vendor.as_str().into(),
            unrestricted: overlay.unrestricted,
            allow_always_allow: overlay.allow_always_allow && vendor != Vendor::Linux,
            allow: overlay.allow,
            builtin_allow,
            deny: overlay.deny,
            deny_substrings: overlay.deny_substrings,
            path: overlay_path(overlay_dir, vendor)
                .to_string_lossy()
                .into_owned(),
        }
    }

    pub fn builtin() -> Self {
        let mut policies = HashMap::new();
        for p in builtin_policies() {
            policies.insert(p.vendor.clone(), p);
        }
        Self {
            builtin: policies.clone(),
            policies,
        }
    }

    pub fn get(&self, vendor: Vendor) -> VendorPolicy {
        self.policies
            .get(vendor.as_str())
            .cloned()
            .unwrap_or_else(|| VendorPolicy {
                vendor: vendor.as_str().into(),
                unrestricted: vendor == Vendor::Linux,
                allow_always_allow: false,
                ..Default::default()
            })
    }

    pub fn check(&self, vendor: Vendor, command: &str) -> PolicyDecision {
        if command
            .chars()
            .any(|c| c.is_control() || c == '\n' || c == '\r')
        {
            return deny(command, "command must be a single line".into());
        }
        let policy = self.get(vendor);
        let expanded = expand_aliases(&policy, command);
        if policy.unrestricted {
            return PolicyDecision {
                allowed: true,
                reason: "Linux has no allowlist; every command still needs an explicit click and always-allow is disabled.".into(),
                expanded,
                linux_unrestricted: true,
                allow_always_allow: false,
            };
        }

        let stages = split_pipes(&expanded);
        for (i, stage) in stages.iter().enumerate() {
            let match_stage = peel_exec_prefix(strip_acl_sequence(stage));
            let token = first_token(match_stage);
            let token_l = token.to_ascii_lowercase();
            let stage_l = match_stage.to_ascii_lowercase();

            if policy.deny.iter().any(|d| {
                let dl = d.to_ascii_lowercase();
                token_l == dl || stage_l == dl || stage_l.starts_with(&(dl.clone() + " "))
            }) {
                return deny(&expanded, format!("denied verb '{token}' (stage {i})"));
            }
            for sub in &policy.deny_substrings {
                if match_stage
                    .to_ascii_lowercase()
                    .contains(&sub.to_ascii_lowercase())
                    || stage
                        .to_ascii_lowercase()
                        .contains(&sub.to_ascii_lowercase())
                {
                    return deny(&expanded, format!("denied substring '{sub}' in stage {i}"));
                }
            }
            if i > 0 {
                let pipe_verb = first_token(match_stage);
                let pl = pipe_verb.to_ascii_lowercase();
                if policy
                    .deny_pipes
                    .iter()
                    .any(|d| pl == d.to_ascii_lowercase())
                {
                    return deny(&expanded, format!("denied pipe '{pipe_verb}'"));
                }
                if !policy.allow_pipes.is_empty()
                    && !policy
                        .allow_pipes
                        .iter()
                        .any(|a| pl == a.to_ascii_lowercase())
                {
                    return deny(
                        &expanded,
                        format!("pipe '{pipe_verb}' is not on the allow list"),
                    );
                }
            } else {
                let overlay_ok = verb_on_allow(&policy.allow, &stage_l);
                let builtin_ok = self
                    .builtin
                    .get(vendor.as_str())
                    .map(|b| verb_on_allow(&b.allow, &stage_l))
                    .unwrap_or(false);
                if !overlay_ok && !builtin_ok {
                    return deny(
                        &expanded,
                        format!("'{token}' is not on the {vendor:?} allow list"),
                    );
                }
            }
        }

        let builtin_denied = self
            .builtin
            .get(vendor.as_str())
            .map(|b| command_matches_deny(b, &expanded))
            .unwrap_or(false);
        PolicyDecision {
            allowed: true,
            reason: "permit list matched".into(),
            expanded: expanded.clone(),
            linux_unrestricted: false,
            allow_always_allow: policy.allow_always_allow
                && !is_mutating(&expanded)
                && !builtin_denied,
        }
    }
}

fn deny(expanded: &str, reason: String) -> PolicyDecision {
    PolicyDecision {
        allowed: false,
        reason,
        expanded: expanded.to_string(),
        linux_unrestricted: false,
        allow_always_allow: false,
    }
}

fn command_matches_deny(policy: &VendorPolicy, expanded: &str) -> bool {
    let stages = split_pipes(expanded);
    for (i, stage) in stages.iter().enumerate() {
        let match_stage = peel_exec_prefix(strip_acl_sequence(stage));
        let token = first_token(match_stage);
        let token_l = token.to_ascii_lowercase();
        let stage_l = match_stage.to_ascii_lowercase();
        if policy.deny.iter().any(|d| {
            let dl = d.to_ascii_lowercase();
            token_l == dl || stage_l == dl || stage_l.starts_with(&(dl.clone() + " "))
        }) {
            return true;
        }
        for sub in &policy.deny_substrings {
            if match_stage
                .to_ascii_lowercase()
                .contains(&sub.to_ascii_lowercase())
                || stage
                    .to_ascii_lowercase()
                    .contains(&sub.to_ascii_lowercase())
            {
                return true;
            }
        }
        if i > 0 {
            let pl = first_token(match_stage).to_ascii_lowercase();
            if policy
                .deny_pipes
                .iter()
                .any(|d| pl == d.to_ascii_lowercase())
            {
                return true;
            }
        }
    }
    false
}

fn is_mutating(expanded: &str) -> bool {
    matches!(
        first_token(peel_exec_prefix(strip_acl_sequence(expanded)))
            .to_ascii_lowercase()
            .as_str(),
        "configure"
            | "config"
            | "conf"
            | "acl"
            | "access-list"
            | "interface"
            | "vlan"
            | "apply"
            | "router"
            | "routing"
            | "qos"
            | "write"
            | "no"
            | "set"
            | "delete"
            | "edit"
            | "ip"
            | "ipv6"
            | "spanning-tree"
            | "route-map"
            | "line"
            | "hostname"
            | "name"
            | "commit"
            | "load"
            | "rollback"
            | "deny"
            | "permit"
            | "remark"
            | "resequence"
    )
}

/// IOS/NX-OS/AOS-CX `do <exec>` — the first token is a wrapper, not the verb.
/// `do reload` must fail closed like `reload`.
fn peel_exec_prefix(stage: &str) -> &str {
    let mut rest = stage.trim();
    loop {
        let mut parts = rest.splitn(2, char::is_whitespace);
        let first = parts.next().unwrap_or("");
        let after = parts.next().unwrap_or("").trim_start();
        if after.is_empty() || !first.eq_ignore_ascii_case("do") {
            return rest;
        }
        rest = after;
    }
}

/// `10 deny tcp any any eq 80` — sequence number is not the verb.
fn strip_acl_sequence(stage: &str) -> &str {
    let stage = stage.trim();
    let mut parts = stage.splitn(2, char::is_whitespace);
    let first = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("").trim_start();
    if rest.is_empty() {
        return stage;
    }
    if !first.is_empty() && first.len() <= 10 && first.chars().all(|c| c.is_ascii_digit()) {
        return rest;
    }
    stage
}

fn first_token(s: &str) -> String {
    s.split_whitespace().next().unwrap_or("").to_string()
}

fn verb_on_allow(allow: &[String], stage_l: &str) -> bool {
    allow.iter().any(|a| {
        let al = a.to_ascii_lowercase();
        stage_l == al || stage_l.starts_with(&(al.clone() + " "))
    })
}

/// Copy new bundled allow verbs into a first-boot YAML that never got `name` / `exit`.
/// Does not remove operator deny lines or extra allows.
pub fn absorb_bundled_allow(dest: &Path, bundled: &Path) -> Result<bool> {
    let dest_raw = std::fs::read_to_string(dest)?;
    let bundled_raw = std::fs::read_to_string(bundled)?;
    let mut dest_p: VendorPolicy = serde_yaml::from_str(&dest_raw)?;
    let bundled_p: VendorPolicy = serde_yaml::from_str(&bundled_raw)?;
    if !dest_p.vendor.is_empty()
        && !bundled_p.vendor.is_empty()
        && !dest_p.vendor.eq_ignore_ascii_case(&bundled_p.vendor)
    {
        return Ok(false);
    }
    let mut added = false;
    for a in &bundled_p.allow {
        if !dest_p
            .allow
            .iter()
            .any(|d| d.eq_ignore_ascii_case(a))
        {
            dest_p.allow.push(a.clone());
            added = true;
        }
    }
    if added {
        std::fs::write(dest, serde_yaml::to_string(&dest_p)?)?;
    }
    Ok(added)
}

pub fn overlay_path(dir: &Path, vendor: Vendor) -> std::path::PathBuf {
    dir.join(format!("{}.yaml", vendor.as_str()))
}

pub fn is_dangerous_verb(token: &str) -> bool {
    let t = token.trim().to_ascii_lowercase();
    let first = t.split_whitespace().next().unwrap_or("");
    DANGEROUS_VERBS.iter().any(|d| {
        t == *d || first == *d || t.starts_with(&format!("{d} "))
    })
}

fn lifts_overlay_deny(token_l: &str) -> bool {
    matches!(
        token_l,
        "reload"
            | "reboot"
            | "erase"
            | "write erase"
            | "boot"
            | "start-shell"
            | "shell"
            | "factory"
            | "copy"
            | "tclsh"
            | "format"
            | "guestshell"
            | "shutdown"
    )
}

pub fn normalize_allow_token(raw: &str) -> Result<String> {
    let t = raw.trim();
    if t.is_empty() {
        return Err(crate::error::LateError::Message("empty permit-list token".into()));
    }
    if t.chars().any(|c| c.is_control() || c == '\n' || c == '\r' || c == '|') {
        return Err(crate::error::LateError::Message(
            "permit-list token must be a single verb (no newlines or pipes)".into(),
        ));
    }
    if t.len() > 64 {
        return Err(crate::error::LateError::Message(
            "permit-list token is too long".into(),
        ));
    }
    Ok(t.to_string())
}

/// Replace overlay `allow`. Overlay deny still wins unless the operator explicitly
/// put that verb on allow (then matching overlay deny / substring lines are dropped).
/// Linux has no permit list. Builtin allow (e.g. `name`) still applies at check time.
pub fn apply_allow_list(mut policy: VendorPolicy, allow: Vec<String>) -> Result<VendorPolicy> {
    if policy.unrestricted || policy.vendor.eq_ignore_ascii_case("linux") {
        return Err(crate::error::LateError::Message(
            "Linux has no permit list. Every command still needs Approve. Always-allow stays off.".into(),
        ));
    }
    let mut next: Vec<String> = Vec::new();
    for raw in allow {
        let token = normalize_allow_token(&raw)?;
        if next.iter().any(|e| e.eq_ignore_ascii_case(&token)) {
            continue;
        }
        next.push(token);
    }
    for token in &next {
        let tl = token.to_ascii_lowercase();
        if !lifts_overlay_deny(&tl) {
            continue;
        }
        policy.deny.retain(|d| {
            let dl = d.to_ascii_lowercase();
            dl != tl && !tl.starts_with(&(dl.clone() + " ")) && !dl.starts_with(&(tl.clone() + " "))
        });
        policy.deny_substrings.retain(|s| {
            let sl = s.to_ascii_lowercase();
            !tl.contains(&sl) && sl != tl
        });
    }
    policy.allow = next;
    policy.allow_always_allow = policy.allow_always_allow && !policy.vendor.eq_ignore_ascii_case("linux");
    Ok(policy)
}

pub fn write_vendor_policy(path: &Path, policy: &VendorPolicy) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_yaml::to_string(policy)?)?;
    Ok(())
}

fn split_pipes(cmd: &str) -> Vec<String> {
    cmd.split('|')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn expand_aliases(policy: &VendorPolicy, command: &str) -> String {
    let mut parts: Vec<String> = command.split_whitespace().map(|s| s.to_string()).collect();
    if parts.is_empty() {
        return String::new();
    }
    let key = parts[0].to_ascii_lowercase();
    if let Some(canon) = policy.aliases.get(&key) {
        let extra: Vec<String> = canon.split_whitespace().map(|s| s.to_string()).collect();
        parts.remove(0);
        let mut out = extra;
        out.append(&mut parts);
        return out.join(" ");
    }
    // two-token aliases like "wr mem"
    if parts.len() >= 2 {
        let two = format!(
            "{} {}",
            parts[0].to_ascii_lowercase(),
            parts[1].to_ascii_lowercase()
        );
        if let Some(canon) = policy.aliases.get(&two) {
            let extra: Vec<String> = canon.split_whitespace().map(|s| s.to_string()).collect();
            parts.drain(0..2);
            let mut out = extra;
            out.append(&mut parts);
            return out.join(" ");
        }
    }
    command.trim().to_string()
}

fn verbs(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn builtin_policies() -> Vec<VendorPolicy> {
    vec![
        cisco_family("cisco_ios", false),
        cisco_family("cisco_ios_xe", true),
        nxos(),
        junos(),
        eos(),
        panos(),
        linux(),
        generic(),
        fortios(),
        routeros(),
        aos_cx(),
    ]
}

fn cisco_family(vendor: &str, xe: bool) -> VendorPolicy {
    let mut deny = verbs(&[
        "enable",
        "disable",
        "copy",
        "delete",
        "erase",
        "write erase",
        "format",
        "archive",
        "boot",
        "reload",
        "reset",
        "clear",
        "logout",
        "quit",
        "tclsh",
        "tclquit",
        "debug",
        "telnet",
    ]);
    deny.extend(verbs(&["event"]));
    if xe {
        deny.extend(verbs(&["guestshell", "app-hosting", "install", "request"]));
    }
    VendorPolicy {
        vendor: vendor.into(),
        aliases: HashMap::from([
            ("sh".into(), "show".into()),
            ("sho".into(), "show".into()),
            ("shw".into(), "show".into()),
            ("wr".into(), "write".into()),
            ("wri".into(), "write".into()),
            ("p".into(), "ping".into()),
            ("tr".into(), "traceroute".into()),
            ("conf".into(), "configure".into()),
            ("t".into(), "telnet".into()),
        ]),
        allow: verbs(&[
            "show",
            "ping",
            "traceroute",
            "dir",
            "more",
            "terminal",
            "where",
            "undebug",
            "configure",
            "interface",
            "vlan",
            "access-list",
            "ip",
            "ipv6",
            "no",
            "end",
            "do",
            "write",
            "router",
            "route-map",
            "line",
            "access-class",
            "deny",
            "permit",
            "remark",
            "name",
            "exit",
        ]),
        deny,
        deny_substrings: verbs(&["tclsh", "guestshell", "app-hosting", "event manager"]),
        allow_pipes: verbs(&["include", "exclude", "begin", "section", "count", "format"]),
        deny_pipes: verbs(&["redirect", "tee", "append"]),
        unrestricted: false,
        allow_always_allow: true,
    }
}

fn nxos() -> VendorPolicy {
    let mut p = cisco_family("cisco_nxos", false);
    p.deny
        .extend(verbs(&["run", "python", "source", "guestshell", "bash"]));
    p.deny_substrings.extend(verbs(&["run bash", "python"]));
    p
}

fn junos() -> VendorPolicy {
    VendorPolicy {
        vendor: "junos".into(),
        aliases: HashMap::from([("sh".into(), "show".into()), ("sho".into(), "show".into())]),
        allow: verbs(&[
            "show",
            "ping",
            "traceroute",
            "monitor",
            "op",
            "test",
            "file list",
            "file show",
        ]),
        deny: verbs(&[
            "configure",
            "edit",
            "commit",
            "rollback",
            "load",
            "save",
            "request",
            "restart",
            "start",
            "clear",
            "set",
            "delete",
            "rename",
            "copy",
            "activate",
            "deactivate",
            "exit",
            "quit",
        ]),
        deny_substrings: verbs(&["start shell", "start network-service"]),
        allow_pipes: verbs(&[
            "match", "except", "find", "count", "display", "trim", "compare",
        ]),
        deny_pipes: verbs(&["save"]),
        unrestricted: false,
        allow_always_allow: true,
    }
}

fn eos() -> VendorPolicy {
    VendorPolicy {
        vendor: "arista_eos".into(),
        aliases: HashMap::from([("sh".into(), "show".into())]),
        allow: verbs(&[
            "show",
            "ping",
            "traceroute",
            "bash timeout",
            "dir",
            "configure",
            "interface",
            "vlan",
            "ip",
            "ipv6",
            "no",
            "end",
            "access-list",
            "deny",
            "permit",
            "remark",
            "name",
            "exit",
        ]),
        deny: verbs(&[
            "copy",
            "delete",
            "reload",
            "bash",
            "python",
            "event-handler",
            "agent",
            "daemon",
            "enable",
        ]),
        deny_substrings: verbs(&["bash", "python", "event-handler"]),
        allow_pipes: verbs(&["include", "exclude", "begin", "section", "nz"]),
        deny_pipes: verbs(&["redirect", "tee", "append"]),
        unrestricted: false,
        allow_always_allow: true,
    }
}

fn panos() -> VendorPolicy {
    VendorPolicy {
        vendor: "panos".into(),
        aliases: HashMap::from([("sh".into(), "show".into())]),
        allow: verbs(&["show", "ping", "traceroute", "test", "request system"]),
        deny: verbs(&[
            "configure",
            "set",
            "delete",
            "commit",
            "load",
            "save",
            "debug",
            "scp",
            "ftp",
            "tftp",
            "less",
            "tail",
            "run",
        ]),
        deny_substrings: verbs(&["debug software shell", "debug system", "request restart"]),
        allow_pipes: verbs(&["match", "except"]),
        deny_pipes: vec![],
        unrestricted: false,
        allow_always_allow: true,
    }
}

fn aos_cx() -> VendorPolicy {
    VendorPolicy {
        vendor: "aos_cx".into(),
        aliases: HashMap::from([("sh".into(), "show".into())]),
        allow: verbs(&[
            "show",
            "ping",
            "traceroute",
            "diag",
            "capture",
            "configure",
            "config",
            "acl",
            "access-list",
            "apply",
            "interface",
            "vlan",
            "ip",
            "ipv6",
            "no",
            "end",
            "do",
            "write",
            "hostname",
            "routing",
            "router",
            "qos",
            "spanning-tree",
            "deny",
            "permit",
            "remark",
            "name",
            "exit",
            "resequence",
        ]),
        deny: verbs(&[
            "erase",
            "reload",
            "boot",
            "write erase",
            "checkpoint",
            "start-shell",
            "copy",
        ]),
        deny_substrings: verbs(&["erase", "reload", "factory", "shell"]),
        allow_pipes: verbs(&["include", "exclude", "begin"]),
        deny_pipes: vec![],
        unrestricted: false,
        allow_always_allow: true,
    }
}

fn linux() -> VendorPolicy {
    VendorPolicy {
        vendor: "linux".into(),
        unrestricted: true,
        allow_always_allow: false,
        ..Default::default()
    }
}

fn generic() -> VendorPolicy {
    VendorPolicy {
        vendor: "generic".into(),
        unrestricted: false,
        allow_always_allow: true,
        allow: verbs(&[
            "show",
            "ping",
            "traceroute",
            "display",
            "get",
            "dir",
            "more",
            "terminal",
        ]),
        deny: verbs(&[
            "configure",
            "config",
            "reload",
            "reboot",
            "write",
            "commit",
            "copy",
            "erase",
            "start-shell",
        ]),
        deny_substrings: verbs(&[
            "configure",
            "reload",
            "reboot",
            "write erase",
            "factory",
            "shell",
        ]),
        ..Default::default()
    }
}

fn fortios() -> VendorPolicy {
    VendorPolicy {
        vendor: "fortios".into(),
        aliases: HashMap::from([("sh".into(), "show".into()), ("get".into(), "get".into())]),
        allow: verbs(&[
            "show",
            "get",
            "diagnose",
            "execute ping",
            "execute traceroute",
        ]),
        deny: verbs(&[
            "config",
            "edit",
            "set",
            "unset",
            "delete",
            "end",
            "execute restore",
            "execute reboot",
            "execute shutdown",
        ]),
        deny_substrings: verbs(&["execute restore", "execute reboot"]),
        allow_pipes: verbs(&["grep"]),
        deny_pipes: vec![],
        unrestricted: false,
        allow_always_allow: true,
    }
}

fn routeros() -> VendorPolicy {
    VendorPolicy {
        vendor: "routeros".into(),
        aliases: Default::default(),
        allow: verbs(&[
            "/ip",
            "/interface",
            "/system resource",
            "/system identity",
            "/routing",
            "/ping",
            "/tool traceroute",
        ]),
        deny: verbs(&[
            "/system reboot",
            "/system shutdown",
            "/user",
            "/file remove",
            "/export",
        ]),
        deny_substrings: verbs(&["reboot", "shutdown", "password", " add", " set", " remove"]),
        allow_pipes: verbs(&["where", "print"]),
        deny_pipes: vec![],
        unrestricted: false,
        allow_always_allow: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wr_erase_is_denied() {
        let e = PolicyEngine::builtin();
        let d = e.check(Vendor::CiscoIos, "wr erase");
        assert!(!d.allowed, "{}", d.reason);
        assert!(d.expanded.starts_with("write"));
    }

    #[test]
    fn cisco_do_prefix_does_not_skip_deny() {
        let e = PolicyEngine::builtin();
        assert!(!e.check(Vendor::CiscoIos, "do reload").allowed);
        assert!(
            !e.check(Vendor::CiscoIos, "do copy running-config tftp://10.0.0.1/x")
                .allowed
        );
        assert!(!e.check(Vendor::CiscoIos, "do write erase").allowed);
        assert!(!e.check(Vendor::CiscoIos, "do do reload").allowed);
        let show = e.check(Vendor::CiscoIos, "do show version");
        assert!(show.allowed, "{}", show.reason);
        assert!(e.check(Vendor::CiscoIos, "do").allowed);
        assert!(!e.check(Vendor::AosCx, "do start-shell").allowed);
        let cfg = e.check(Vendor::CiscoIos, "do configure terminal");
        assert!(cfg.allowed, "{}", cfg.reason);
        assert!(!cfg.allow_always_allow);
        let show = e.check(Vendor::CiscoIos, "do show version");
        assert!(show.allow_always_allow);
    }

    #[test]
    fn overlay_cannot_always_allow_builtin_denied_verb() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("cisco_ios.yaml"),
            r#"
vendor: cisco_ios
allow: [show, reload, do]
deny: []
allow_always_allow: true
"#,
        )
        .unwrap();
        let mut e = PolicyEngine::builtin();
        e.merge_dir(dir.path()).unwrap();
        let reload = e.check(Vendor::CiscoIos, "reload");
        assert!(reload.allowed, "{}", reload.reason);
        assert!(
            !reload.allow_always_allow,
            "overlay must not always-allow a builtin-denied verb"
        );
        let peeled = e.check(Vendor::CiscoIos, "do reload");
        assert!(peeled.allowed, "{}", peeled.reason);
        assert!(
            !peeled.allow_always_allow,
            "do-peel must still treat builtin deny as not always-allow"
        );
        let show = e.check(Vendor::CiscoIos, "show version");
        assert!(show.allowed, "{}", show.reason);
        assert!(show.allow_always_allow);
    }

    #[test]
    fn show_is_allowed() {
        let e = PolicyEngine::builtin();
        let d = e.check(Vendor::CiscoIos, "sh ip bgp sum");
        assert!(d.allowed, "{}", d.reason);
        assert_eq!(d.expanded, "show ip bgp sum");
        assert!(d.allow_always_allow);
        assert!(!d.linux_unrestricted);
    }

    #[test]
    fn pipe_redirect_denied() {
        let e = PolicyEngine::builtin();
        let d = e.check(Vendor::CiscoIos, "show run | redirect tftp://10.0.0.1/x");
        assert!(!d.allowed);
    }

    #[test]
    fn junos_start_shell_denied() {
        let e = PolicyEngine::builtin();
        let d = e.check(Vendor::Junos, "start shell");
        assert!(!d.allowed);
    }

    #[test]
    fn linux_unrestricted_but_flagged() {
        let e = PolicyEngine::builtin();
        let d = e.check(Vendor::Linux, "rm -rf /");
        assert!(d.allowed);
        assert!(d.linux_unrestricted);
        assert!(!d.allow_always_allow, "Linux never always-allow");
    }

    #[test]
    fn tclsh_denied() {
        let e = PolicyEngine::builtin();
        assert!(!e.check(Vendor::CiscoIosXe, "tclsh").allowed);
    }

    #[test]
    fn repo_yaml_overlays_builtin() {
        let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../policies");
        if !p.is_dir() {
            return;
        }
        let e = PolicyEngine::load_dir(&p).unwrap();
        let d = e.check(Vendor::CiscoIos, "wr erase");
        assert!(!d.allowed);
        assert!(d.expanded.starts_with("write"));
        let linux = e.check(Vendor::Linux, "rm -rf /");
        assert!(linux.allowed && linux.linux_unrestricted);
        assert!(!linux.allow_always_allow);
    }

    #[test]
    fn multiline_command_denied() {
        let e = PolicyEngine::builtin();
        let d = e.check(Vendor::CiscoIos, "show version\nconfigure terminal");
        assert!(!d.allowed, "{}", d.reason);
    }

    #[test]
    fn generic_vendor_allows_show_not_configure() {
        let e = PolicyEngine::builtin();
        let show = e.check(Vendor::Generic, "show version");
        assert!(show.allowed, "{}", show.reason);
        let cfg = e.check(Vendor::Generic, "configure terminal");
        assert!(!cfg.allowed, "{}", cfg.reason);
        assert!(!show.linux_unrestricted);
    }

    #[test]
    fn aos_cx_shell_and_copy_denied() {
        let e = PolicyEngine::builtin();
        assert!(!e.check(Vendor::AosCx, "start-shell").allowed);
        assert!(
            !e.check(Vendor::AosCx, "copy running-config tftp://1.1.1.1/x")
                .allowed
        );
        assert!(e.check(Vendor::AosCx, "show vlan").allowed);
        assert!(e.check(Vendor::AosCx, "vlan 2000").allowed);
        assert!(e.check(Vendor::AosCx, "vlan 2500").allowed);
        assert!(e.check(Vendor::AosCx, "name VLAN2000").allowed);
        assert!(e.check(Vendor::AosCx, "name VLAN2500").allowed);
        let acl = e.check(Vendor::AosCx, "configure terminal");
        assert!(acl.allowed, "{}", acl.reason);
        assert!(!acl.allow_always_allow);
        assert!(e.check(Vendor::AosCx, "access-list ip HTTP_BLOCK").allowed);
        let ace = e.check(Vendor::AosCx, "10 deny tcp any any eq 80");
        assert!(ace.allowed, "{}", ace.reason);
        assert!(!ace.allow_always_allow);
        assert!(e.check(Vendor::AosCx, "20 permit any any").allowed);
        assert!(!e.check(Vendor::AosCx, "10 start-shell").allowed);
        assert!(!e.check(Vendor::AosCx, "10 reload").allowed);
    }

    #[test]
    fn name_vlan_2500_allowed_on_aoscx() {
        let e = PolicyEngine::builtin();
        for cmd in ["name VLAN2500", "name VLAN2000", "vlan 2500", "configure terminal", "end"] {
            let d = e.check(Vendor::AosCx, cmd);
            assert!(d.allowed, "{cmd}: {}", d.reason);
        }
        for cmd in ["reload", "erase", "start-shell", "sudo reboot"] {
            let d = e.check(Vendor::AosCx, cmd);
            assert!(!d.allowed, "{cmd} must stay denied");
        }
    }

    #[test]
    fn stale_aos_cx_overlay_without_name_still_allows_vlan_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("aos_cx.yaml"),
            r#"
vendor: aos_cx
aliases:
  sh: show
allow: [show, ping, traceroute, diag, capture, configure, config, acl, access-list, apply, interface, vlan, ip, ipv6, no, end, do, write, hostname, routing, router, qos, spanning-tree]
deny: [erase, reload, boot, write erase, checkpoint, start-shell, copy]
deny_substrings: [erase, reload, factory, shell]
allow_pipes: [include, exclude, begin]
deny_pipes: []
unrestricted: false
allow_always_allow: true
"#,
        )
        .unwrap();
        let mut e = PolicyEngine::builtin();
        e.merge_dir(dir.path()).unwrap();
        let name = e.check(Vendor::AosCx, "name VLAN2500");
        assert!(name.allowed, "stale first-boot YAML must not drop builtin name: {}", name.reason);
        assert!(e.check(Vendor::AosCx, "vlan 2500").allowed);
        assert!(!e.check(Vendor::AosCx, "reload").allowed);
        assert!(!e.check(Vendor::AosCx, "erase").allowed);
        assert!(!e.check(Vendor::AosCx, "start-shell").allowed);
        assert!(!e.check(Vendor::AosCx, "sudo reboot").allowed);

        let dest = dir.path().join("aos_cx.yaml");
        let bundled = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../policies/aos_cx.yaml");
        if bundled.is_file() {
            assert!(absorb_bundled_allow(&dest, &bundled).unwrap());
            let refreshed = std::fs::read_to_string(&dest).unwrap();
            assert!(
                refreshed.to_ascii_lowercase().contains("name"),
                "{refreshed}"
            );
        }
    }

    #[test]
    fn routeros_reboot_denied_on_full_stage() {
        let e = PolicyEngine::builtin();
        assert!(!e.check(Vendor::Routeros, "/system reboot").allowed);
        assert!(!e.check(Vendor::Routeros, "/ip firewall filter add").allowed);
    }

    #[test]
    fn save_load_allow_tokens_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("aos_cx.yaml");
        std::fs::write(
            &dest,
            r#"
vendor: aos_cx
allow: [show, configure, vlan, end]
deny: [erase, reload, start-shell]
deny_substrings: [erase, reload, factory, shell]
unrestricted: false
allow_always_allow: true
"#,
        )
        .unwrap();
        let raw = std::fs::read_to_string(&dest).unwrap();
        let p: VendorPolicy = serde_yaml::from_str(&raw).unwrap();
        let saved = apply_allow_list(
            p,
            vec![
                "show".into(),
                "configure".into(),
                "vlan".into(),
                "end".into(),
                "name".into(),
            ],
        )
        .unwrap();
        write_vendor_policy(&dest, &saved).unwrap();
        let loaded: VendorPolicy =
            serde_yaml::from_str(&std::fs::read_to_string(&dest).unwrap()).unwrap();
        assert!(
            loaded.allow.iter().any(|a| a.eq_ignore_ascii_case("name")),
            "{:?}",
            loaded.allow
        );
        assert!(loaded.deny.iter().any(|d| d.eq_ignore_ascii_case("reload")));
        let mut e = PolicyEngine::builtin();
        e.merge_dir(dir.path()).unwrap();
        assert!(e.check(Vendor::AosCx, "name VLAN2500").allowed);
        assert!(!e.check(Vendor::AosCx, "reload").allowed);
        assert!(!e.check(Vendor::AosCx, "erase").allowed);
        assert!(!e.check(Vendor::AosCx, "start-shell").allowed);
    }

    #[test]
    fn explicit_reload_drops_overlay_deny_and_warns_as_dangerous() {
        let p: VendorPolicy = serde_yaml::from_str(
            r#"
vendor: aos_cx
allow: [show]
deny: [erase, reload, start-shell]
deny_substrings: [erase, reload, factory, shell]
unrestricted: false
allow_always_allow: true
"#,
        )
        .unwrap();
        assert!(is_dangerous_verb("reload"));
        assert!(!is_dangerous_verb("name"));
        let saved = apply_allow_list(p, vec!["show".into(), "reload".into()]).unwrap();
        assert!(saved.allow.iter().any(|a| a.eq_ignore_ascii_case("reload")));
        assert!(!saved.deny.iter().any(|d| d.eq_ignore_ascii_case("reload")));
        assert!(!saved
            .deny_substrings
            .iter()
            .any(|d| d.eq_ignore_ascii_case("reload")));
        let dir = tempfile::tempdir().unwrap();
        write_vendor_policy(&dir.path().join("aos_cx.yaml"), &saved).unwrap();
        let mut e = PolicyEngine::builtin();
        e.merge_dir(dir.path()).unwrap();
        let reload = e.check(Vendor::AosCx, "reload");
        assert!(reload.allowed, "{}", reload.reason);
        assert!(!reload.allow_always_allow, "builtin deny still blocks always-allow");
        assert!(!e.check(Vendor::AosCx, "erase").allowed);
        assert!(!e.check(Vendor::AosCx, "start-shell").allowed);
    }

    #[test]
    fn stock_eos_save_does_not_lift_bash_deny() {
        let p = eos();
        let deny_before = p.deny.clone();
        let saved = apply_allow_list(p.clone(), p.allow.clone()).unwrap();
        assert!(
            saved.deny.iter().any(|d| d.eq_ignore_ascii_case("bash")),
            "bash timeout on allow must not drop bash deny: {:?}",
            saved.deny
        );
        assert_eq!(saved.deny, deny_before);
    }

    #[test]
    fn linux_has_no_permit_list_edits() {
        let p = linux();
        let err = apply_allow_list(p, vec!["ls".into()]).unwrap_err();
        assert!(err.to_string().contains("Linux has no permit list"));
    }

    #[test]
    fn removing_name_from_overlay_keeps_builtin_floor() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("aos_cx.yaml");
        let p = aos_cx();
        let without_name: Vec<String> = p
            .allow
            .iter()
            .filter(|a| !a.eq_ignore_ascii_case("name"))
            .cloned()
            .collect();
        let saved = apply_allow_list(p, without_name).unwrap();
        assert!(!saved.allow.iter().any(|a| a.eq_ignore_ascii_case("name")));
        write_vendor_policy(&dest, &saved).unwrap();
        let mut e = PolicyEngine::builtin();
        e.merge_dir(dir.path()).unwrap();
        let view = e.view(Vendor::AosCx, dir.path());
        assert!(
            view.builtin_allow
                .iter()
                .any(|a| a.eq_ignore_ascii_case("name")),
            "{:?}",
            view.builtin_allow
        );
        assert!(e.check(Vendor::AosCx, "name VLAN2500").allowed);
        assert!(!e.check(Vendor::AosCx, "reload").allowed);
    }
}
