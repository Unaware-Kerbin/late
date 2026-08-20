//! Browser Origin allowlist for loopback HTTP.
//!
//! Prefix matching (`starts_with("http://127.0.0.1")`) is unsafe: it accepts
//! `http://127.0.0.1.evil.com`. Parse host and port instead.
//! Keep this list in sync with `apps/agent-sidecar/src/local-auth.ts`.

const UI_PORTS: &[u16] = &[5173, 4173, 1420];

pub fn is_allowed_origin(origin: &str) -> bool {
    let o = origin.trim();
    if o.is_empty() || o.eq_ignore_ascii_case("null") {
        return false;
    }
    let lower = o.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "tauri://localhost" | "https://tauri.localhost" | "http://tauri.localhost"
    ) {
        return true;
    }
    let Some((scheme, after)) = lower.split_once("://") else {
        return false;
    };
    if scheme != "http" && scheme != "https" {
        return false;
    }
    let hostport = after.split('/').next().unwrap_or("");
    if hostport.is_empty() || hostport.contains('@') {
        return false;
    }
    let (host, port) = split_host_port(hostport);
    let loopback = host == "127.0.0.1" || host == "localhost" || host == "::1";
    if !loopback {
        return false;
    }
    matches!(port, Some(p) if UI_PORTS.contains(&p))
}

fn split_host_port(hostport: &str) -> (&str, Option<u16>) {
    if let Some(rest) = hostport.strip_prefix('[') {
        let Some(end) = rest.find(']') else {
            return ("", None);
        };
        let host = &rest[..end];
        let port = rest[end + 1..]
            .strip_prefix(':')
            .and_then(|p| p.parse::<u16>().ok());
        return (host, port);
    }
    match hostport.rsplit_once(':') {
        Some((h, p)) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => {
            (h, p.parse().ok())
        }
        _ => (hostport, None),
    }
}

/// DNS-rebinding guard: Host must be loopback, not an attacker-controlled name.
pub fn is_loopback_host_header(host: &str) -> bool {
    let host = host.trim().to_ascii_lowercase();
    if host.is_empty() {
        return false;
    }
    let (name, _) = split_host_port(&host);
    name == "127.0.0.1" || name == "localhost" || name == "::1"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_vite_and_preview_loopback() {
        assert!(is_allowed_origin("http://127.0.0.1:5173"));
        assert!(is_allowed_origin("http://localhost:5173"));
        assert!(is_allowed_origin("http://127.0.0.1:4173"));
        assert!(is_allowed_origin("http://127.0.0.1:1420"));
        assert!(is_allowed_origin("https://tauri.localhost"));
        assert!(is_allowed_origin("tauri://localhost"));
        assert!(is_allowed_origin("http://[::1]:5173"));
    }

    #[test]
    fn rejects_prefix_spoof_and_open_ports() {
        assert!(!is_allowed_origin("http://127.0.0.1.evil.com"));
        assert!(!is_allowed_origin("http://127.0.0.1.evil.com:5173"));
        assert!(!is_allowed_origin("http://127.0.0.1:5173.evil.com"));
        assert!(!is_allowed_origin("http://evil.com"));
        assert!(!is_allowed_origin("http://127.0.0.1"));
        assert!(!is_allowed_origin("http://127.0.0.1:80"));
        assert!(!is_allowed_origin("http://127.0.0.1:7420"));
        assert!(!is_allowed_origin("http://127.0.0.1:7430"));
        assert!(!is_allowed_origin("null"));
        assert!(!is_allowed_origin("file://"));
        assert!(!is_allowed_origin(""));
        assert!(!is_allowed_origin("http://user@127.0.0.1:5173"));
    }

    #[test]
    fn host_header_rejects_dns_rebind_names() {
        assert!(is_loopback_host_header("127.0.0.1:7420"));
        assert!(is_loopback_host_header("localhost"));
        assert!(is_loopback_host_header("[::1]:7420"));
        assert!(!is_loopback_host_header("evil.example:7420"));
        assert!(!is_loopback_host_header("127.0.0.1.evil.com"));
        assert!(!is_loopback_host_header(""));
    }
}
