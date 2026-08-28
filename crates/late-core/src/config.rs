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
    /// Extra OpenAI-compatible URL for vLLM (LAN / another machine). Empty = none.
    /// `vllm_base_url` is whichever URL chat uses right now (this computer or this field).
    pub vllm_remote_url: String,
    pub cursor_model: String,
    pub ollama_base_url: String,
    pub ollama_model: String,
    pub ollama_remote_url: String,
    pub llama_cpp_base_url: String,
    pub llama_cpp_model: String,
    pub llama_cpp_remote_url: String,
    pub anthropic_base_url: String,
    pub anthropic_model: String,
    pub gemini_base_url: String,
    pub gemini_model: String,
    pub azure_base_url: String,
    pub azure_deployment: String,
    pub azure_api_version: String,
    pub default_backend: String,
    pub scrollback_lines: usize,
    pub turn_timeout_secs: u64,
    pub max_agent_rounds: u32,
    /// Serialized in Settings. Live capture always writes under Late `data/pcap`.
    /// The value is confined on save and is not an extra jail root.
    pub pcap_dir: PathBuf,
    /// Serialized in Settings. Not used as a session-log root.
    pub log_dir: PathBuf,
    /// Lab gear with private PKI. Default false — verify TLS.
    pub api_insecure_tls: bool,
    /// Cursor and public-internet OpenAI-compatible chat. Default false.
    /// Private-network (RFC1918 / .internal) OpenAI-compatible URLs do not need this.
    pub cloud_chat_enabled: bool,
    /// Extra hostnames treated as private inference (air-gapped names that are not RFC1918 literals).
    pub private_inference_hosts: String,
    /// Optional MCP client. Default false. Folder stdio on this computer and/or HTTP URL.
    pub mcp_enabled: bool,
    /// Project folder on this computer (must stay under home or Late data).
    pub mcp_cwd: String,
    /// Optional command. Empty = tsx src/index.ts or node dist/index.js in mcp_cwd.
    pub mcp_command: String,
    /// Optional extra argv (no shell). Empty unless mcp_command is set.
    pub mcp_args: String,
    /// Streamable HTTP MCP URL (another box). Empty = spawn mcp_cwd.
    pub mcp_url: String,
    /// When more than one GPU is on this computer, Local Start uses all of them. Default true.
    pub use_all_gpus: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        let dirs = LatePaths::discover();
        Self {
            bind: "127.0.0.1:7420".into(),
            vllm_base_url: "http://127.0.0.1:8000/v1".into(),
            vllm_model: "local".into(),
            vllm_remote_url: String::new(),
            cursor_model: "composer-2.5".into(),
            ollama_base_url: "http://127.0.0.1:11434/v1".into(),
            ollama_model: String::new(),
            ollama_remote_url: String::new(),
            llama_cpp_base_url: "http://127.0.0.1:8080/v1".into(),
            llama_cpp_model: String::new(),
            llama_cpp_remote_url: String::new(),
            anthropic_base_url: "https://api.anthropic.com".into(),
            anthropic_model: "claude-sonnet-4-5".into(),
            gemini_base_url: "https://generativelanguage.googleapis.com".into(),
            gemini_model: "gemini-2.5-flash".into(),
            azure_base_url: String::new(),
            azure_deployment: String::new(),
            azure_api_version: "2024-10-21".into(),
            default_backend: "local".into(),
            scrollback_lines: 32_000,
            turn_timeout_secs: 90,
            max_agent_rounds: 50,
            pcap_dir: dirs.data.join("pcap"),
            log_dir: dirs.data.join("logs"),
            api_insecure_tls: false,
            cloud_chat_enabled: false,
            private_inference_hosts: String::new(),
            mcp_enabled: false,
            mcp_cwd: String::new(),
            mcp_command: String::new(),
            mcp_args: String::new(),
            mcp_url: String::new(),
            use_all_gpus: true,
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
            &self.data.join("staging"),
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
    let mut s: AppSettings = toml::from_str(&raw).map_err(|e| LateError::Config(e.to_string()))?;
    remember_remote_inference_urls(&mut s);
    Ok(s)
}

/// If the active helper URL is not loopback and no LAN URL is stored yet, keep it as the extra server.
/// In-memory only on load; the next Settings save writes it.
pub(crate) fn remember_remote_inference_urls(s: &mut AppSettings) -> bool {
    let mut changed = false;
    changed |= fill_remote_if_active_is_lan(&s.vllm_base_url, &mut s.vllm_remote_url);
    changed |= fill_remote_if_active_is_lan(&s.ollama_base_url, &mut s.ollama_remote_url);
    changed |= fill_remote_if_active_is_lan(&s.llama_cpp_base_url, &mut s.llama_cpp_remote_url);
    changed
}

fn fill_remote_if_active_is_lan(active: &str, remote: &mut String) -> bool {
    if !remote.trim().is_empty() {
        return false;
    }
    let a = active.trim();
    if a.is_empty() || url_host_is_loopback(a) {
        return false;
    }
    *remote = a.to_string();
    true
}

fn url_host_is_loopback(raw: &str) -> bool {
    let Ok(u) = reqwest::Url::parse(raw.trim()) else {
        return false;
    };
    let host = u.host_str().unwrap_or("").trim_matches(|c| c == '[' || c == ']');
    let h = host.to_ascii_lowercase();
    h == "localhost"
        || h == "localhost.localdomain"
        || h == "::1"
        || h == "0.0.0.0"
        || h.starts_with("127.")
}

/// http(s) only. Empty is ok (stdio folder). No file: / shell / userinfo.
pub fn validate_mcp_http_url(raw: &str) -> Result<()> {
    let t = raw.trim();
    if t.is_empty() {
        return Ok(());
    }
    if t.bytes().any(|b| b < 0x20 || b == 0x7f)
        || t.contains(';')
        || t.contains('|')
        || t.contains('&')
        || t.contains('`')
        || t.contains('$')
        || t.contains('<')
        || t.contains('>')
        || t.contains('\\')
    {
        return Err(LateError::Config(
            "MCP address cannot contain shell or control characters".into(),
        ));
    }
    let u = reqwest::Url::parse(t).map_err(|_| {
        LateError::Config("MCP address is not a valid URL".into())
    })?;
    if u.scheme() != "http" && u.scheme() != "https" {
        return Err(LateError::Config(
            "MCP address must be http:// or https:// (not a file path)".into(),
        ));
    }
    if !u.username().is_empty() || u.password().is_some() {
        return Err(LateError::Config(
            "MCP address cannot include a username or password".into(),
        ));
    }
    if u.host_str().unwrap_or("").is_empty() {
        return Err(LateError::Config("MCP address needs a host (IP or name)".into()));
    }
    if url_host_is_loopback(t) && u.port() == Some(8787) {
        return Err(LateError::Config(
            "That address is the GUI on this computer, not MCP. Use http://127.0.0.1:8790/mcp after npm run mcp:http, or the folder".into(),
        ));
    }
    Ok(())
}

pub fn save_settings(path: &Path, settings: &AppSettings) -> Result<()> {
    crate::fsutil::write_private(
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
        assert_eq!(s.anthropic_base_url, "https://api.anthropic.com");
        assert_eq!(s.gemini_model, "gemini-2.5-flash");
        assert_eq!(s.azure_api_version, "2024-10-21");
        assert_eq!(s.default_backend, "local");
        assert_eq!(s.vllm_base_url, "http://127.0.0.1:8000/v1");
        assert_eq!(s.vllm_remote_url, "");
        assert_eq!(s.ollama_remote_url, "");
        assert_eq!(s.llama_cpp_remote_url, "");
        assert!(!s.cloud_chat_enabled);
        assert_eq!(s.private_inference_hosts, "");
        assert!(!s.api_insecure_tls);
        assert!(!s.mcp_enabled);
        assert_eq!(s.mcp_cwd, "");
        assert_eq!(s.mcp_command, "");
        assert_eq!(s.mcp_args, "");
        assert_eq!(s.mcp_url, "");
        assert!(s.use_all_gpus);
    }

    #[test]
    fn remember_remote_copies_lan_url() {
        let mut s = AppSettings::default();
        s.vllm_base_url = "http://10.0.0.12:8000/v1".into();
        s.ollama_base_url = "http://gpu.lab.internal:11434/v1".into();
        s.llama_cpp_base_url = "http://127.0.0.1:8080/v1".into();
        assert!(remember_remote_inference_urls(&mut s));
        assert_eq!(s.vllm_remote_url, "http://10.0.0.12:8000/v1");
        assert_eq!(s.ollama_remote_url, "http://gpu.lab.internal:11434/v1");
        assert_eq!(s.llama_cpp_remote_url, "");
        assert!(!remember_remote_inference_urls(&mut s));
    }

    #[test]
    fn remember_remote_skips_loopback() {
        let mut s = AppSettings::default();
        assert!(!remember_remote_inference_urls(&mut s));
        assert_eq!(s.vllm_remote_url, "");
        s.vllm_base_url = "http://localhost:8000/v1".into();
        assert!(!remember_remote_inference_urls(&mut s));
        assert_eq!(s.vllm_remote_url, "");
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
    fn mcp_settings_round_trips() {
        let mut s = AppSettings::default();
        assert!(!s.mcp_enabled);
        s.mcp_enabled = true;
        s.mcp_cwd = "/home/me/MCP".into();
        s.mcp_command = String::new();
        s.mcp_args = String::new();
        s.mcp_url = "http://10.0.0.12:8790/mcp".into();
        let raw = toml::to_string_pretty(&s).unwrap();
        let back: AppSettings = toml::from_str(&raw).unwrap();
        assert!(back.mcp_enabled);
        assert_eq!(back.mcp_cwd, "/home/me/MCP");
        assert_eq!(back.mcp_command, "");
        assert_eq!(back.mcp_args, "");
        assert_eq!(back.mcp_url, "http://10.0.0.12:8790/mcp");
    }

    #[test]
    fn use_all_gpus_round_trips_and_defaults_on() {
        let mut s = AppSettings::default();
        assert!(s.use_all_gpus);
        s.use_all_gpus = false;
        let raw = toml::to_string_pretty(&s).unwrap();
        let back: AppSettings = toml::from_str(&raw).unwrap();
        assert!(!back.use_all_gpus);
    }

    #[test]
    fn private_inference_hosts_round_trips() {
        let mut s = AppSettings::default();
        s.private_inference_hosts = "llm.airgap.mil, gpu.lab".into();
        let raw = toml::to_string_pretty(&s).unwrap();
        let back: AppSettings = toml::from_str(&raw).unwrap();
        assert_eq!(back.private_inference_hosts, "llm.airgap.mil, gpu.lab");
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

    #[test]
    fn mcp_http_url_accepts_http_and_rejects_file_and_shell() {
        assert!(validate_mcp_http_url("").is_ok());
        assert!(validate_mcp_http_url("http://10.0.0.12:8790/mcp").is_ok());
        assert!(validate_mcp_http_url("http://127.0.0.1:8790/mcp").is_ok());
        assert!(validate_mcp_http_url("http://127.0.0.1:8790/MCP").is_ok());
        assert!(validate_mcp_http_url("http://localhost:8787/MCP").is_err());
        assert!(validate_mcp_http_url("http://127.0.0.1:8787/").is_err());
        assert!(validate_mcp_http_url("https://mcp.lab.internal/mcp").is_ok());
        assert!(validate_mcp_http_url("file:///tmp/mcp").is_err());
        assert!(validate_mcp_http_url("http://10.0.0.12:8790/mcp; rm -rf /").is_err());
        assert!(validate_mcp_http_url("http://user:pass@10.0.0.12/mcp").is_err());
    }
}
