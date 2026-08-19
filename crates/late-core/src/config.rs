use crate::error::{LateError, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub bind: String,
    pub vllm_base_url: String,
    pub vllm_model: String,
    pub cursor_model: String,
    pub default_backend: String,
    pub scrollback_lines: usize,
    pub turn_timeout_secs: u64,
    pub max_agent_rounds: u32,
    pub pcap_dir: PathBuf,
    pub log_dir: PathBuf,
}

impl Default for AppSettings {
    fn default() -> Self {
        let dirs = LatePaths::discover();
        Self {
            bind: "127.0.0.1:7420".into(),
            vllm_base_url: "http://127.0.0.1:8000/v1".into(),
            vllm_model: "local".into(),
            cursor_model: "composer-2.5".into(),
            default_backend: "local".into(),
            scrollback_lines: 32_000,
            turn_timeout_secs: 90,
            max_agent_rounds: 50,
            pcap_dir: dirs.data.join("pcap"),
            log_dir: dirs.data.join("logs"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LatePaths {
    pub config: PathBuf,
    pub data: PathBuf,
}

impl LatePaths {
    pub fn discover() -> Self {
        let config = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("late");
        let data = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("late");
        Self { config, data }
    }

    pub fn ensure(&self) -> Result<()> {
        for p in [
            &self.config,
            &self.data,
            &self.config.join("policies"),
            &self.data.join("pcap"),
            &self.data.join("logs"),
            &self.data.join("captures"),
            &self.data.join("exports"),
        ] {
            fs::create_dir_all(p)?;
        }
        Ok(())
    }

    pub fn inventory(&self) -> PathBuf {
        self.config.join("inventory.toml")
    }
    pub fn known_hosts(&self) -> PathBuf {
        self.config.join("known_hosts.toml")
    }
    pub fn settings(&self) -> PathBuf {
        self.config.join("settings.toml")
    }
    pub fn secrets_file(&self) -> PathBuf {
        self.config.join("secrets.json")
    }
    pub fn provider_keys(&self) -> PathBuf {
        self.config.join("provider-keys.json")
    }
    pub fn sidecar_token(&self) -> PathBuf {
        self.config.join("sidecar.token")
    }
    pub fn collections(&self) -> PathBuf {
        self.config.join("collections.toml")
    }
    pub fn auth_profiles(&self) -> PathBuf {
        self.config.join("auth_profiles.toml")
    }
}

pub fn load_settings(path: &Path) -> Result<AppSettings> {
    if !path.exists() {
        let s = AppSettings::default();
        save_settings(path, &s)?;
        return Ok(s);
    }
    let raw = fs::read_to_string(path)?;
    toml::from_str(&raw).map_err(|e| LateError::Config(e.to_string()))
}

pub fn save_settings(path: &Path, settings: &AppSettings) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        path,
        toml::to_string_pretty(settings).map_err(|e| LateError::Config(e.to_string()))?,
    )?;
    Ok(())
}
