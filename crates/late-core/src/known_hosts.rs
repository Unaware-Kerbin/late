use crate::config::LatePaths;
use crate::error::{LateError, Result};
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
        fs::write(
            paths.known_hosts(),
            toml::to_string_pretty(self).map_err(|e| LateError::Config(e.to_string()))?,
        )?;
        Ok(())
    }

    pub fn fingerprint(bytes: &[u8]) -> String {
        let digest = Sha256::digest(bytes);
        format!("SHA256:{}", base64_nopad(&digest))
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

