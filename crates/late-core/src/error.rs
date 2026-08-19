use thiserror::Error;

#[derive(Debug, Error)]
pub enum LateError {
    #[error("{0}")]
    Message(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("policy denied: {0}")]
    PolicyDenied(String),
    #[error("host key mismatch for {host}: pinned {pinned}, presented {presented}")]
    HostKeyMismatch {
        host: String,
        pinned: String,
        presented: String,
    },
    #[error("host key untrusted for {host}")]
    HostKeyUntrusted { host: String, presented: String },
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("ssh: {0}")]
    Ssh(String),
    #[error("serial: {0}")]
    Serial(String),
    #[error("sftp: {0}")]
    Sftp(String),
    #[error("pcap: {0}")]
    Pcap(String),
    #[error("http: {0}")]
    Http(String),
    #[error("secret: {0}")]
    Secret(String),
    #[error("config: {0}")]
    Config(String),
    #[error("import: {0}")]
    Import(String),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("yaml: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("toml: {0}")]
    TomlDe(#[from] toml::de::Error),
    #[error("toml: {0}")]
    TomlSer(#[from] toml::ser::Error),
}

impl LateError {
    pub fn rpc_code(&self) -> i32 {
        match self {
            LateError::NotFound(_) => -32004,
            LateError::PolicyDenied(_) => -32010,
            LateError::HostKeyUntrusted { .. } => -32021,
            LateError::HostKeyMismatch { .. } => -32022,
            _ => -32000,
        }
    }

    pub fn rpc_data(&self) -> Option<serde_json::Value> {
        match self {
            LateError::HostKeyUntrusted { host, presented } => Some(serde_json::json!({
                "code": "host_key_untrusted",
                "kind": "host_key_untrusted",
                "host": host,
                "presented": presented
            })),
            LateError::HostKeyMismatch {
                host,
                pinned,
                presented,
            } => Some(serde_json::json!({
                "code": "host_key_mismatch",
                "kind": "host_key_mismatch",
                "host": host,
                "pinned": pinned,
                "presented": presented
            })),
            LateError::PolicyDenied(reason) => Some(serde_json::json!({
                "code": "policy_denied",
                "reason": reason
            })),
            _ => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, LateError>;
