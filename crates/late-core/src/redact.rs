use regex::Regex;
use std::collections::HashMap;
use std::sync::OnceLock;

/// Best-effort redaction. Secrets become `[REDACTED:kind#N]` with stable per-session ordinals.
pub struct Redactor {
    seen: HashMap<(String, String), usize>,
    next: HashMap<String, usize>,
}

impl Default for Redactor {
    fn default() -> Self {
        Self {
            seen: HashMap::new(),
            next: HashMap::new(),
        }
    }
}

impl Redactor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn redact(&mut self, input: &str) -> String {
        let mut out = input.to_string();
        for (kind, re) in patterns() {
            out = re
                .replace_all(&out, |caps: &regex::Captures| {
                    let secret = caps
                        .iter()
                        .flatten()
                        .last()
                        .map(|m| m.as_str())
                        .unwrap_or("");
                    self.token(kind, secret)
                })
                .into_owned();
        }
        out
    }

    fn token(&mut self, kind: &str, secret: &str) -> String {
        let key = (kind.to_string(), secret.to_string());
        if let Some(n) = self.seen.get(&key) {
            return format!("[REDACTED:{kind}#{n}]");
        }
        let n = self.next.entry(kind.to_string()).or_insert(1);
        let assigned = *n;
        *n += 1;
        self.seen.insert(key, assigned);
        format!("[REDACTED:{kind}#{assigned}]")
    }
}

fn patterns() -> &'static [(&'static str, Regex)] {
    static PATS: OnceLock<Vec<(&'static str, Regex)>> = OnceLock::new();
    PATS.get_or_init(|| {
        let mk = |kind, pat| (kind, Regex::new(pat).expect("redact regex"));
        vec![
            mk("pem", r"(-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----)"),
            mk("jwt", r"\b(eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,})\b"),
            mk("aws", r"\b(AKIA[0-9A-Z]{16})\b"),
            mk("cisco7", r"(?i)((?:enable\s+)?password\s+7\s+[0-9A-Fa-f]+)"),
            mk("cisco_secret", r"(?i)(enable\s+secret\s+\d+\s+\S+)"),
            mk("cisco_secret5", r"(?i)(secret\s+5\s+\$1\$\S+)"),
            mk("junos9", r"(\$9\$\S+)"),
            mk("junos8", r"(\$8\$\S+)"),
            mk("md5", r"(?i)((?:authentication-key|password|md5)\s+[^\s;]{8,})"),
            mk("snmp", r"(?i)(snmp-server\s+community\s+\S+)"),
            mk("community", r"(?i)(community\s+(?:ro|rw)?\s*\S+)"),
            mk("password_json", r#"(?i)("(password|passwd|passphrase|secret|token|api[_-]?key|psk|community)"\s*:\s*")([^"]+)"#),
            mk("bearer", r"(?i)(authorization:\s*bearer\s+)\S+"),
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pem_and_jwt_and_stable_ordinals() {
        let mut r = Redactor::new();
        let pem = "-----BEGIN PRIVATE KEY-----\nABC\n-----END PRIVATE KEY-----";
        let a = r.redact(pem);
        let b = r.redact(pem);
        assert!(a.contains("[REDACTED:pem#1]"));
        assert_eq!(a, b);
        let jwt = r.redact(
            "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0In0.abcabcabcabcabcabcab",
        );
        assert!(jwt.contains("[REDACTED:jwt#1]"));
    }

    #[test]
    fn json_password_field() {
        let mut r = Redactor::new();
        let out = r.redact(r#"{"password":"hunter2"}"#);
        assert!(!out.contains("hunter2"));
        assert!(out.contains("REDACTED"));
    }
}
