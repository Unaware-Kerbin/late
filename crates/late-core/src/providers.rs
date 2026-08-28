//! Cloud provider API keys — isolated from SSH password vault.
//!
//! UI may only set/clear/status. Materializing key bytes is for the sidecar
//! over a localhost HTTP route that requires a 0600 token and rejects browsers.

use crate::config::LatePaths;
use crate::error::{LateError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use uuid::Uuid;
use zeroize::Zeroize;

pub const PROVIDER_NAMES: &[&str] = &[
    "cursor",
    "openai",
    "anthropic",
    "gemini",
    "azure",
    "groq",
    "openrouter",
    "custom",
    "mcp",
];

const MAX_KEY_LEN: usize = 8192;

#[derive(Debug, Default, Serialize, Deserialize)]
struct KeyVault {
    keys: HashMap<String, String>,
}

#[derive(Clone)]
pub struct ProviderVault {
    paths: LatePaths,
}

impl ProviderVault {
    pub fn open(paths: LatePaths) -> Result<Self> {
        let v = Self { paths };
        v.ensure_sidecar_token()?;
        Ok(v)
    }

    pub fn set(&self, name: &str, key: &str) -> Result<()> {
        let name = validate_name(name)?;
        let key = validate_key(key)?;
        let mut vault = self.load()?;
        if key.is_empty() {
            if let Some(mut old) = vault.keys.remove(name) {
                old.zeroize();
            }
        } else {
            vault.keys.insert(name.to_string(), key);
        }
        self.save(&vault)
    }

    pub fn delete(&self, name: &str) -> Result<()> {
        self.set(name, "")
    }

    pub fn status(&self) -> Result<HashMap<String, bool>> {
        let vault = self.load()?;
        Ok(PROVIDER_NAMES
            .iter()
            .map(|n| {
                (
                    (*n).to_string(),
                    vault.keys.get(*n).map(|s| !s.is_empty()).unwrap_or(false),
                )
            })
            .collect())
    }

    /// Sidecar-only. Caller must have already authenticated the sidecar token.
    pub fn materialize(&self) -> Result<HashMap<String, String>> {
        let vault = self.load()?;
        Ok(vault
            .keys
            .into_iter()
            .filter(|(n, v)| PROVIDER_NAMES.contains(&n.as_str()) && !v.is_empty())
            .collect())
    }

    pub fn sidecar_token(&self) -> Result<String> {
        let path = self.paths.sidecar_token();
        let tok = fs::read_to_string(&path)?;
        Ok(tok.trim().to_string())
    }

    pub fn token_matches(&self, presented: &str) -> bool {
        let Ok(stored) = self.sidecar_token() else {
            return false;
        };
        if stored.is_empty() || presented.len() != stored.len() {
            return false;
        }
        // constant-time-ish compare
        stored
            .bytes()
            .zip(presented.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
    }

    fn ensure_sidecar_token(&self) -> Result<()> {
        let path = self.paths.sidecar_token();
        if path.exists() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let meta = fs::metadata(&path)?;
                let mode = meta.permissions().mode() & 0o777;
                if mode != 0o600 {
                    let mut p = meta.permissions();
                    p.set_mode(0o600);
                    fs::set_permissions(&path, p)?;
                }
            }
            return Ok(());
        }
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        crate::fsutil::write_private(&path, &token)?;
        Ok(())
    }

    fn load(&self) -> Result<KeyVault> {
        let path = self.paths.provider_keys();
        if !path.exists() {
            return Ok(KeyVault::default());
        }
        let raw = fs::read_to_string(path)?;
        Ok(serde_json::from_str(&raw)?)
    }

    fn save(&self, vault: &KeyVault) -> Result<()> {
        crate::fsutil::write_private(
            &self.paths.provider_keys(),
            serde_json::to_string_pretty(vault)?,
        )?;
        Ok(())
    }
}

fn validate_name(name: &str) -> Result<&str> {
    let n = name.trim().to_ascii_lowercase();
    PROVIDER_NAMES
        .iter()
        .copied()
        .find(|p| *p == n)
        .ok_or_else(|| LateError::Message(format!("unknown provider '{name}'")))
}

fn validate_key(key: &str) -> Result<String> {
    let trimmed = key.trim().to_string();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    if trimmed.len() > MAX_KEY_LEN {
        return Err(LateError::Message("API key is too long".into()));
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err(LateError::Message(
            "API key contains control characters".into(),
        ));
    }
    if trimmed.len() < 8 {
        return Err(LateError::Message("API key looks too short".into()));
    }
    Ok(trimmed)
}
