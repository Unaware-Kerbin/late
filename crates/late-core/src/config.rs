use crate::error::{LateError, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub bind: String,
    pub vllm_base_url: String,
    pub vllm_model: String,
    pub cursor_model: String,
    pub ollama_base_url: String,
    pub ollama_model: String,
    pub llama_cpp_base_url: String,
    pub llama_cpp_model: String,
    pub default_backend: String,
    pub scrollback_lines: usize,
    pub turn_timeout_secs: u64,
    pub max_agent_rounds: u32,
    pub pcap_dir: PathBuf,
    pub log_dir: PathBuf,
    /// Lab gear with private PKI. Default false — verify TLS.
    pub api_insecure_tls: bool,
    /// Cursor and non-loopback OpenAI-compatible chat. Default false (local only).
    pub cloud_chat_enabled: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        let dirs = LatePaths::discover();
        Self {
            bind: "127.0.0.1:7420".into(),
            vllm_base_url: "http://127.0.0.1:8000/v1".into(),
            vllm_model: "local".into(),
            cursor_model: "composer-2.5".into(),
            ollama_base_url: "http://127.0.0.1:11434/v1".into(),
            ollama_model: String::new(),
            llama_cpp_base_url: "http://127.0.0.1:8080/v1".into(),
            llama_cpp_model: String::new(),
            default_backend: "local".into(),
            scrollback_lines: 32_000,
            turn_timeout_secs: 90,
            max_agent_rounds: 50,
            pcap_dir: dirs.data.join("pcap"),
            log_dir: dirs.data.join("logs"),
            api_insecure_tls: false,
            cloud_chat_enabled: false,
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
            &self.data.join("models"),
            &self.data.join("models/gguf"),
        ] {
            crate::fsutil::mkdir_private(p)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_ollama_fields_use_defaults() {
        let raw = r#"
bind = "127.0.0.1:7420"
vllm_base_url = "http://127.0.0.1:8000/v1"
vllm_model = "local"
cursor_model = "composer-2.5"
default_backend = "local"
scrollback_lines = 32000
turn_timeout_secs = 90
max_agent_rounds = 50
pcap_dir = "/tmp/pcap"
log_dir = "/tmp/logs"
"#;
        let s: AppSettings = toml::from_str(raw).expect("legacy settings.toml must still load");
        assert_eq!(s.ollama_base_url, "http://127.0.0.1:11434/v1");
        assert_eq!(s.ollama_model, "");
        assert_eq!(s.llama_cpp_base_url, "http://127.0.0.1:8080/v1");
        assert_eq!(s.llama_cpp_model, "");
        assert_eq!(s.default_backend, "local");
        assert_eq!(s.vllm_base_url, "http://127.0.0.1:8000/v1");
        assert!(!s.cloud_chat_enabled);
        assert!(!s.api_insecure_tls);
    }

    #[test]
    fn cloud_chat_round_trips() {
        let mut s = AppSettings::default();
        s.cloud_chat_enabled = true;
        let raw = toml::to_string_pretty(&s).unwrap();
        let back: AppSettings = toml::from_str(&raw).unwrap();
        assert!(back.cloud_chat_enabled);
    }

    #[test]
    fn ollama_backend_round_trips() {
        let mut s = AppSettings::default();
        s.default_backend = "ollama".into();
        s.ollama_base_url = "http://127.0.0.1:11434/v1".into();
        s.ollama_model = "llama3.2".into();
        let raw = toml::to_string_pretty(&s).unwrap();
        let back: AppSettings = toml::from_str(&raw).unwrap();
        assert_eq!(back.default_backend, "ollama");
        assert_eq!(back.ollama_model, "llama3.2");
        assert_eq!(back.ollama_base_url, "http://127.0.0.1:11434/v1");
    }

    #[test]
    fn llamacpp_backend_round_trips() {
        let mut s = AppSettings::default();
        s.default_backend = "llamacpp".into();
        s.llama_cpp_base_url = "http://127.0.0.1:8080/v1".into();
        s.llama_cpp_model = "qwen2.5-7b".into();
        let raw = toml::to_string_pretty(&s).unwrap();
        let back: AppSettings = toml::from_str(&raw).unwrap();
        assert_eq!(back.default_backend, "llamacpp");
        assert_eq!(back.llama_cpp_model, "qwen2.5-7b");
        assert_eq!(back.llama_cpp_base_url, "http://127.0.0.1:8080/v1");
    }
}
