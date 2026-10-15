use crate::config::LatePaths;
use crate::error::{LateError, Result};
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KnownHosts {
    pub pins: BTreeMap<String, String>,
}

impl KnownHosts {
    pub fn load(paths: &LatePaths) -> Result<Self> {
        let path = paths.known_hosts();
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(path)?;
        Ok(toml::from_str(&raw)?)
    }

    pub fn save(&self, paths: &LatePaths) -> Result<()> {
        crate::fsutil::write_private(
            &paths.known_hosts(),
            toml::to_string_pretty(self).map_err(|e| LateError::Config(e.to_string()))?,
        )?;
        Ok(())
    }

    pub fn fingerprint(bytes: &[u8]) -> String {
        let digest = Sha256::digest(bytes);
        format!("SHA256:{}", base64_nopad(&digest))
    }

    /// OpenSSH-style SHA256 of the decoded key blob, plus the algorithm.
    pub fn fingerprint_keyscan_line(line: &str) -> Result<(String, String)> {
        let (algo, raw) = parse_keyscan_line(line)?;
        Ok((algo, Self::fingerprint(&raw)))
    }

    pub fn check(&self, host: &str, presented: &str) -> HostKeyCheck {
        match self.pins.get(host) {
            None => HostKeyCheck::Unknown,
            Some(pinned) if pinned == presented => HostKeyCheck::Match,
            Some(pinned) => HostKeyCheck::Mismatch {
                pinned: pinned.clone(),
                presented: presented.to_string(),
            },
        }
    }

    /// TOFU against every key `ssh-keyscan` returned. A host often advertises
    /// ed25519 + ecdsa + rsa; flaky scans may omit the preferred type, so matching
    /// only the preferred line falsely reports mismatch for an unchanged host.
    pub fn check_any<'a>(
        &self,
        host: &str,
        presented: impl IntoIterator<Item = &'a str>,
    ) -> HostKeyCheck {
        let Some(pinned) = self.pins.get(host) else {
            return HostKeyCheck::Unknown;
        };
        let mut first: Option<String> = None;
        for fp in presented {
            if first.is_none() {
                first = Some(fp.to_string());
            }
            if fp == pinned {
                return HostKeyCheck::Match;
            }
        }
        HostKeyCheck::Mismatch {
            pinned: pinned.clone(),
            presented: first.unwrap_or_else(|| "unknown fingerprint".into()),
        }
    }

    pub fn pin(&mut self, host: &str, fingerprint: &str) {
        self.pins.insert(host.to_string(), fingerprint.to_string());
    }
}

#[derive(Debug, Clone)]
pub enum HostKeyCheck {
    Unknown,
    Match,
    Mismatch { pinned: String, presented: String },
}

fn base64_nopad(bytes: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        let b0 = bytes[i];
        let b1 = if i + 1 < bytes.len() { bytes[i + 1] } else { 0 };
        let b2 = if i + 2 < bytes.len() { bytes[i + 2] } else { 0 };
        out.push(T[(b0 >> 2) as usize] as char);
        out.push(T[(((b0 & 3) << 4) | (b1 >> 4)) as usize] as char);
        if i + 1 < bytes.len() {
            out.push(T[(((b1 & 15) << 2) | (b2 >> 6)) as usize] as char);
        }
        if i + 2 < bytes.len() {
            out.push(T[(b2 & 63) as usize] as char);
        }
        i += 3;
    }
    out
}

pub fn host_port_key(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_string()
    } else {
        format!("[{host}]:{port}")
    }
}

pub fn parse_host_port_key(key: &str) -> Result<(String, u16)> {
    let key = key.trim();
    if key.is_empty() {
        return Err(LateError::Ssh("empty host pin key".into()));
    }
    if let Some(rest) = key.strip_prefix('[') {
        let close = rest
            .find(']')
            .ok_or_else(|| LateError::Ssh("invalid host pin key".into()))?;
        let host = rest[..close].to_string();
        if host.is_empty() {
            return Err(LateError::Ssh("invalid host pin key".into()));
        }
        let port = match rest[close + 1..].strip_prefix(':') {
            Some(p) => p
                .parse()
                .map_err(|_| LateError::Ssh("invalid port in host pin key".into()))?,
            None => 22,
        };
        return Ok((host, port));
    }
    Ok((key.to_string(), 22))
}

const KEY_ALGO_PREF: &[&str] = &[
    "ssh-ed25519",
    "sk-ssh-ed25519@openssh.com",
    "ecdsa-sha2-nistp256",
    "ecdsa-sha2-nistp384",
    "ecdsa-sha2-nistp521",
    "rsa-sha2-512",
    "rsa-sha2-256",
    "ssh-rsa",
];

/// One host key from `ssh-keyscan` stdout: raw line, algorithm, SHA256 fingerprint.
#[derive(Debug, Clone)]
pub struct KeyscanEntry {
    pub line: String,
    pub algo: String,
    pub fingerprint: String,
}

/// Parse every usable host-key line from keyscan output (comments skipped).
pub fn parse_keyscan_entries(text: &str) -> Vec<KeyscanEntry> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Ok((algo, raw)) = parse_keyscan_line(line) else {
            continue;
        };
        out.push(KeyscanEntry {
            line: line.to_string(),
            algo,
            fingerprint: KnownHosts::fingerprint(&raw),
        });
    }
    out
}

fn key_algo_rank(algo: &str) -> usize {
    KEY_ALGO_PREF
        .iter()
        .position(|a| *a == algo)
        .unwrap_or(KEY_ALGO_PREF.len())
}

/// Prefer Ed25519, then ECDSA, then RSA. First matching line wins within a type.
pub fn pick_keyscan_line(text: &str) -> Result<String> {
    Ok(pick_keyscan_entry(text)?.line)
}

/// Prefer the entry whose fingerprint is already pinned; otherwise algo preference.
pub fn pick_keyscan_entry_for_pin(text: &str, pinned: Option<&str>) -> Result<KeyscanEntry> {
    let entries = parse_keyscan_entries(text);
    if entries.is_empty() {
        return Err(LateError::Ssh("ssh-keyscan returned no host key".into()));
    }
    if let Some(pinned) = pinned {
        if let Some(e) = entries.iter().find(|e| e.fingerprint == pinned) {
            return Ok(e.clone());
        }
    }
    entries
        .into_iter()
        .min_by_key(|e| key_algo_rank(&e.algo))
        .ok_or_else(|| LateError::Ssh("ssh-keyscan returned no host key".into()))
}

pub fn pick_keyscan_entry(text: &str) -> Result<KeyscanEntry> {
    pick_keyscan_entry_for_pin(text, None)
}

pub fn parse_keyscan_line(line: &str) -> Result<(String, Vec<u8>)> {
    let line = line.trim();
    let mut parts = line.split_whitespace();
    let _host = parts
        .next()
        .ok_or_else(|| LateError::Ssh("empty keyscan line".into()))?;
    let algo = parts
        .next()
        .ok_or_else(|| LateError::Ssh("keyscan line missing type".into()))?;
    let blob = parts
        .next()
        .ok_or_else(|| LateError::Ssh("keyscan line missing key".into()))?;
    if sanitized_host_key_algo(algo).is_none() {
        return Err(LateError::Ssh(format!("unsupported host key type {algo}")));
    }
    let raw = STANDARD
        .decode(blob)
        .or_else(|_| STANDARD_NO_PAD.decode(blob))
        .map_err(|e| LateError::Ssh(format!("keyscan key: {e}")))?;
    if raw.is_empty() {
        return Err(LateError::Ssh("empty host key".into()));
    }
    Ok((algo.to_string(), raw))
}

pub fn sanitized_host_key_algo(algo: &str) -> Option<&str> {
    if algo.is_empty() || algo.len() > 80 {
        return None;
    }
    if !algo
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '@' | '.'))
    {
        return None;
    }
    if algo.starts_with("ssh-")
        || algo.starts_with("ecdsa-")
        || algo.starts_with("sk-")
        || algo.starts_with("rsa-sha2-")
    {
        Some(algo)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_key_blob_not_hostname() {
        let blob = STANDARD.encode(b"fake-key-bytes");
        let a = format!("evil.example ssh-ed25519 {blob}");
        let b = format!("good.example ssh-ed25519 {blob}");
        let fa = KnownHosts::fingerprint_keyscan_line(&a).unwrap();
        let fb = KnownHosts::fingerprint_keyscan_line(&b).unwrap();
        assert_eq!(fa.0, "ssh-ed25519");
        assert_eq!(fa.1, fb.1);
        assert_ne!(fa.1, KnownHosts::fingerprint(a.as_bytes()));
    }

    #[test]
    fn pick_prefers_ed25519() {
        let rsa_blob = STANDARD.encode(b"rsa-bytes");
        let ed_blob = STANDARD.encode(b"ed-bytes");
        let text = format!(
            "# comment\nhost.example ssh-rsa {rsa_blob}\nhost.example ssh-ed25519 {ed_blob}\n"
        );
        let line = pick_keyscan_line(&text).unwrap();
        assert!(line.contains("ssh-ed25519"));
    }

    #[test]
    fn check_any_matches_non_preferred_algo() {
        let ed_blob = STANDARD.encode(b"ed-bytes");
        let ec_blob = STANDARD.encode(b"ec-bytes");
        let text = format!(
            "host.example ecdsa-sha2-nistp256 {ec_blob}\nhost.example ssh-ed25519 {ed_blob}\n"
        );
        let entries = parse_keyscan_entries(&text);
        assert_eq!(entries.len(), 2);
        let ed_fp = entries
            .iter()
            .find(|e| e.algo == "ssh-ed25519")
            .unwrap()
            .fingerprint
            .clone();
        let mut kh = KnownHosts::default();
        kh.pin("10.1.0.12", &ed_fp);
        // Preferred pick is ed25519; simulate a flaky scan that only returned ECDSA.
        let only_ecdsa = format!("host.example ecdsa-sha2-nistp256 {ec_blob}\n");
        let ecdsa_only = parse_keyscan_entries(&only_ecdsa);
        assert!(matches!(
            kh.check("10.1.0.12", &ecdsa_only[0].fingerprint),
            HostKeyCheck::Mismatch { .. }
        ));
        // Full multi-algo probe still matches the pinned ed25519.
        let fps: Vec<_> = entries.iter().map(|e| e.fingerprint.as_str()).collect();
        assert!(matches!(
            kh.check_any("10.1.0.12", fps),
            HostKeyCheck::Match
        ));
        let picked = pick_keyscan_entry_for_pin(&text, Some(&ed_fp)).unwrap();
        assert_eq!(picked.algo, "ssh-ed25519");
        assert_eq!(picked.fingerprint, ed_fp);
    }

    #[test]
    fn parse_host_port_key_roundtrip() {
        assert_eq!(
            parse_host_port_key("example.com").unwrap(),
            ("example.com".into(), 22)
        );
        assert_eq!(
            parse_host_port_key("[10.0.0.1]:2222").unwrap(),
            ("10.0.0.1".into(), 2222)
        );
    }
}
