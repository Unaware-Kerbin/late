use std::net::SocketAddr;

/// Start / chat stay on your computer. Refuse LAN wildcards (same idea as late-daemon).
pub fn parse_loopback_bind(s: &str) -> anyhow::Result<SocketAddr> {
    let addr: SocketAddr = s.parse().map_err(|e| {
        anyhow::anyhow!("invalid --bind {s:?}: {e}")
    })?;
    if !addr.ip().is_loopback() {
        anyhow::bail!(
            "late-infer only binds loopback (got {addr}). Run it on your computer; Late does not start inference on another host."
        );
    }
    Ok(addr)
}

#[cfg(test)]
mod tests {
    use super::parse_loopback_bind;

    #[test]
    fn accepts_loopback() {
        let v4 = parse_loopback_bind("127.0.0.1:8010").expect("127.0.0.1 is loopback");
        assert_eq!(v4.ip().to_string(), "127.0.0.1");
        assert_eq!(v4.port(), 8010);
        let v6 = parse_loopback_bind("[::1]:8010").expect("::1 is loopback");
        assert!(v6.ip().is_loopback());
    }

    #[test]
    fn refuses_wildcard_and_lan() {
        for s in [
            "0.0.0.0:8010",
            "[::]:8010",
            "[::ffff:0.0.0.0]:8010",
            "192.168.2.139:8010",
            "10.0.0.1:8010",
            "172.16.0.1:8010",
            "[::ffff:192.168.1.1]:8010",
        ] {
            let err = parse_loopback_bind(s)
                .expect_err(s)
                .to_string();
            assert!(
                err.contains("loopback"),
                "expected loopback refusal for {s}, got {err}"
            );
            assert!(
                err.contains("your computer"),
                "expected your computer in {s} refusal, got {err}"
            );
        }
    }
}
