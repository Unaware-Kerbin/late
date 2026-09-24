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
    /// One-shot connect failure (timeout, refused, unreachable, auth). Never include secrets.
    #[error("Unable to Connect to {host}: {reason}")]
    UnableToConnect {
        host: String,
        reason: String,
        cause: String,
    },
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
            LateError::UnableToConnect { .. } => -32030,
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
            LateError::UnableToConnect { host, reason, cause } => Some(serde_json::json!({
                "code": "unable_to_connect",
                "kind": "unable_to_connect",
                "host": host,
                "reason": reason,
                "cause": cause
            })),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unable_to_connect_rpc_shape_no_secrets() {
        let e = LateError::UnableToConnect {
            host: "10.1.0.10:9".into(),
            reason: "connection refused".into(),
            cause: "refused".into(),
        };
        assert_eq!(e.rpc_code(), -32030);
        assert_eq!(e.to_string(), "Unable to Connect to 10.1.0.10:9: connection refused");
        let d = e.rpc_data().unwrap();
        assert_eq!(d["code"], "unable_to_connect");
        assert_eq!(d["host"], "10.1.0.10:9");
        assert_eq!(d["cause"], "refused");
        assert!(!e.to_string().to_lowercase().contains("password"));
    }
}

pub type Result<T> = std::result::Result<T, LateError>;
