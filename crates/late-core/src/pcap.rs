use crate::error::{LateError, Result};
use crate::types::{PacketSummary, PcapFinding};
use pcap_file::pcap::PcapReader;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureInfo {
    pub id: String,
    pub iface: String,
    pub file: PathBuf,
    pub running: bool,
    #[serde(default)]
    pub kind: String,
}

pub struct LiveCapture {
    pub id: String,
    pub iface: String,
    pub file: PathBuf,
    child: Option<Child>,
    pump: Option<std::thread::JoinHandle<()>>,
    stop_tx: Option<std::sync::mpsc::Sender<()>>,
    worker: Option<std::thread::JoinHandle<Result<()>>>,
}

pub const UI_PACKET_CAP: usize = 400;
pub const STORE_PACKET_CAP: usize = 2_500;
pub const SCAN_PACKET_CAP: usize = 25_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseResult {
    pub packets: Vec<PacketSummary>,
    pub findings: Vec<PcapFinding>,
    pub total: usize,
    pub truncated: bool,
}

pub fn validate_iface(iface: &str) -> Result<()> {
    let ok = iface == "any"
        || (iface.len() <= 64
            && iface
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._:+@-".contains(c)));
    if ok {
        Ok(())
    } else {
        Err(LateError::Pcap("invalid interface name".into()))
    }
}

pub fn validate_bpf(bpf: &str) -> Result<()> {
    if bpf.len() > 200 {
        return Err(LateError::Pcap("filter too long".into()));
    }
    if !bpf.chars().all(|c| {
        c.is_ascii_alphanumeric()
            || matches!(
                c,
                ' ' | '\t' | '.' | ':' | '/' | '(' | ')' | '=' | '!' | '-' | '>' | '<' | '\\'
            )
    }) {
        return Err(LateError::Pcap("filter contains unsafe characters".into()));
    }
    Ok(())
}

fn bpf_argv(bpf: &str) -> Result<String> {
    validate_bpf(bpf)?;
    let t = bpf.trim();
    if t.is_empty() {
        return Ok(String::new());
    }
    Ok(format!(" '{}'", t.replace('\'', "")))
}

pub fn looks_like_pcap(bytes: &[u8]) -> bool {
    matches!(
        bytes.get(..4),
        Some(&[0xd4, 0xc3, 0xb2, 0xa1])
            | Some(&[0xa1, 0xb2, 0xc3, 0xd4])
            | Some(&[0x4d, 0x3c, 0xb2, 0xa1])
            | Some(&[0xa1, 0xb2, 0x3c, 0x4d])
            | Some(&[0x0a, 0x0d, 0x0d, 0x0a])
    )
}

/// Strip SSH chatter from a failed unix-tcpdump probe. Platform-specific
/// capture is handled in `remote_pcap`, not by dumping a MOTD here.
pub fn explain_remote_capture_failure(stdout: &[u8], stderr: &str) -> String {
    let out = String::from_utf8_lossy(stdout);
    let blob = format!("{stderr}\n{out}").to_lowercase();
    if blob.contains("restricted rights")
        || blob.contains("aos-cx")
        || blob.contains("arubaos-cx")
        || blob.contains("hewlett packard enterprise")
        || blob.contains("aruba")
        || blob.contains("nx-os")
        || blob.contains("cisco ios")
        || blob.contains("junos")
        || blob.contains("arista")
    {
        return "unix tcpdump is not available on this SSH CLI; trying the device capture command"
            .into();
    }
    let cleaned: String = stderr
        .lines()
        .filter(|line| {
            let l = line.trim();
            !(l.is_empty()
                || l.starts_with("Warning: Permanently added")
                || l.contains("post-quantum key exchange")
                || l.contains("store now, decrypt later")
                || l.contains("openssh.com/pq.html")
                || l.starts_with("**"))
        })
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .chars()
        .take(280)
        .collect();
    if cleaned.is_empty() {
        "remote capture did not return a pcap (need tcpdump or dumpcap on that host)".into()
    } else {
        format!("remote capture did not return a pcap: {cleaned}")
    }
}

fn read_child_err(child: &mut Child) -> String {
    let mut err = String::new();
    if let Some(mut s) = child.stderr.take() {
        use std::io::Read;
        let _ = s.read_to_string(&mut err);
    }
    let _ = child.wait();
    err.trim().to_string()
}

fn pcap_bytes(path: &Path) -> u64 {
    path.metadata().map(|m| m.len()).unwrap_or(0)
}

fn spawn_local_capture(
    iface: &str,
    outfile: &Path,
    bpf: Option<&str>,
    count: Option<u32>,
) -> Result<Child> {
    let count_s = count.map(|c| c.clamp(1, 2_000).to_string());
    let bpf = bpf.map(str::trim).filter(|s| !s.is_empty());
    let mut last = String::from("no packet capturer found");
    let mut tries: Vec<(String, Command)> = Vec::new();

    let mut dumpcap = Command::new("dumpcap");
    dumpcap
        .arg("-q")
        .arg("-F")
        .arg("pcap")
        .arg("-i")
        .arg(iface)
        .arg("-s")
        .arg("128")
        .arg("-w")
        .arg(outfile);
    if let Some(c) = &count_s {
        dumpcap.arg("-c").arg(c);
    }
    if let Some(f) = bpf {
        dumpcap.arg("-f").arg(f);
    }
    tries.push(("dumpcap".into(), dumpcap));

    let mut sudo = Command::new("sudo");
    sudo.arg("-n")
        .arg("tcpdump")
        .arg("-nn")
        .arg("-U")
        .arg("-s")
        .arg("128")
        .arg("-i")
        .arg(iface)
        .arg("-w")
        .arg(outfile);
    if let Some(c) = &count_s {
        sudo.arg("-c").arg(c);
    }
    if let Some(f) = bpf {
        sudo.arg(f);
    }
    tries.push(("sudo tcpdump".into(), sudo));

    for (label, mut cmd) in tries {
        let _ = std::fs::remove_file(outfile);
        cmd.stdout(Stdio::null()).stderr(Stdio::piped());
        match cmd.spawn() {
            Ok(mut child) => {
                let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1200);
                loop {
                    if let Some(status) = child.try_wait().ok().flatten() {
                        last = format!("{label} exited {status}: {}", read_child_err(&mut child));
                        break;
                    }
                    if pcap_bytes(outfile) >= 24 {
                        drain_stderr(&mut child);
                        return Ok(child);
                    }
                    if std::time::Instant::now() >= deadline {
                        let _ = child.kill();
                        last = format!(
                            "{label} ran but wrote no file at {} ({})",
                            outfile.display(),
                            read_child_err(&mut child)
                        );
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            }
            Err(e) => last = format!("{label}: {e}"),
        }
    }
    Err(LateError::Pcap(format!(
        "{last}. Local files go in ~/.local/share/late/pcap. dumpcap (wireshark group) or passwordless sudo -n tcpdump is required."
    )))
}

fn drain_stderr(child: &mut Child) {
    if let Some(mut s) = child.stderr.take() {
        std::thread::spawn(move || {
            use std::io::Read;
            let mut buf = Vec::new();
            let _ = s.read_to_end(&mut buf);
        });
    }
}

impl LiveCapture {
    pub fn start(
        iface: &str,
        outfile: PathBuf,
        bpf: Option<&str>,
        count: Option<u32>,
    ) -> Result<Self> {
        validate_iface(iface)?;
        if let Some(f) = bpf {
            validate_bpf(f)?;
        }
        let child = spawn_local_capture(iface, &outfile, bpf, count)?;
        Ok(Self {
            id: uuid::Uuid::new_v4().to_string(),
            iface: iface.into(),
            file: outfile,
            child: Some(child),
            pump: None,
            stop_tx: None,
            worker: None,
        })
    }

    pub fn remote(
        iface: String,
        file: PathBuf,
        child: Option<Child>,
        pump: Option<std::thread::JoinHandle<()>>,
        stop_tx: Option<std::sync::mpsc::Sender<()>>,
        worker: Option<std::thread::JoinHandle<Result<()>>>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            iface,
            file,
            child,
            pump,
            stop_tx,
            worker,
        }
    }

    pub fn wait_or_timeout(&mut self, timeout: std::time::Duration) {
        let start = std::time::Instant::now();
        while start.elapsed() < timeout {
            match self
                .child
                .as_mut()
                .and_then(|c| c.try_wait().ok().flatten())
            {
                Some(_) => {
                    self.child = None;
                    return;
                }
                None if self.child.is_none() => return,
                None => std::thread::sleep(std::time::Duration::from_millis(50)),
            }
        }
    }

    pub fn stop(&mut self) -> Result<()> {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(());
        }
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        if let Some(p) = self.pump.take() {
            let _ = p.join();
        }
        if let Some(w) = self.worker.take() {
            match w.join() {
                Ok(Ok(())) | Err(_) => {}
                Ok(Err(e)) => {
                    if !self.file.is_file()
                        || self.file.metadata().map(|m| m.len()).unwrap_or(0) < 24
                    {
                        return Err(e);
                    }
                }
            }
        }
        Ok(())
    }
}

fn default_route_iface() -> Option<String> {
    let out = Command::new("sh")
        .arg("-c")
        .arg("ip -4 route show default 2>/dev/null | awk '{print $5; exit}'")
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn iface_rank(name: &str, default: Option<&str>) -> u8 {
    if name == "any" {
        0
    } else if Some(name) == default {
        1
    } else if name == "lo" {
        9
    } else if name.starts_with("br-")
        || name == "docker0"
        || name.starts_with("veth")
        || name.starts_with("virbr")
    {
        8
    } else {
        2
    }
}

pub fn list_interfaces() -> Vec<String> {
    let out = Command::new("sh")
        .arg("-c")
        .arg("ls /sys/class/net 2>/dev/null")
        .output();
    let mut nics: Vec<String> = match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        Err(_) => Vec::new(),
    };
    if !nics.iter().any(|n| n == "any") {
        nics.insert(0, "any".into());
    }
    let def = default_route_iface();
    nics.sort_by(
        |a, b| match iface_rank(a, def.as_deref()).cmp(&iface_rank(b, def.as_deref())) {
            Ordering::Equal => a.cmp(b),
            o => o,
        },
    );
    if nics.is_empty() {
        vec!["any".into()]
    } else {
        nics
    }
}

fn read_u32(b: &[u8], off: usize, le: bool) -> Option<u32> {
    let s = b.get(off..off + 4)?;
    Some(if le {
        u32::from_le_bytes(s.try_into().ok()?)
    } else {
        u32::from_be_bytes(s.try_into().ok()?)
    })
}

fn pcap_frames(bytes: &[u8]) -> Result<(u32, Vec<(u64, Vec<u8>)>)> {
    if bytes.len() < 24 {
        return Err(LateError::Pcap("pcap file too short".into()));
    }
    let magic_le = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
    let (le, _nsec) = match magic_le {
        0xa1b2_c3d4 => (true, false),
        0xa1b2_3c4d => (true, true),
        0xd4c3_b2a1 => (false, false),
        0x4d3c_b2a1 => (false, true),
        _ => {
            return Err(LateError::Pcap(format!(
                "unsupported pcap magic {:08x}",
                magic_le
            )))
        }
    };
    let linktype = read_u32(bytes, 20, le).unwrap_or(1);
    let mut off = 24usize;
    let mut frames = Vec::new();
    while off + 16 <= bytes.len() {
        let Some(incl) = read_u32(bytes, off + 8, le) else {
            break;
        };
        let ts = u64::from(read_u32(bytes, off, le).unwrap_or(0));
        off += 16;
        let take = incl as usize;
        if take > 1_000_000 || off + take > bytes.len() {
            break;
        }
        frames.push((ts, bytes[off..off + take].to_vec()));
        off += take;
    }
    Ok((linktype, frames))
}

fn be16(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes(d.get(o..o + 2)?.try_into().ok()?))
}

fn be32(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(o..o + 4)?.try_into().ok()?))
}

fn known_ethertype(et: u16) -> bool {
    matches!(et, 0x0800 | 0x86dd | 0x0806 | 0x8100 | 0x88a8 | 0x9100)
}

fn ipv4_str(d: &[u8], o: usize) -> Option<String> {
    let s = d.get(o..o + 4)?;
    Some(format!("{}.{}.{}.{}", s[0], s[1], s[2], s[3]))
}

fn ipv6_str(d: &[u8], o: usize) -> Option<String> {
    let s: [u8; 16] = d.get(o..o + 16)?.try_into().ok()?;
    Some(std::net::Ipv6Addr::from(s).to_string())
}

fn l3_offset(data: &[u8], linktype: u32) -> Option<(u16, usize)> {
    match linktype {
        113 => {
            if data.len() < 16 {
                return None;
            }
            Some((be16(data, 14)?, 16))
        }
        276 => {
            if data.len() < 20 {
                return None;
            }
            Some((be16(data, 0)?, 20))
        }
        12 | 14 | 101 | 228 | 229 => {
            let ver = *data.first()? >> 4;
            let et = match ver {
                4 => 0x0800,
                6 => 0x86dd,
                _ => return None,
            };
            Some((et, 0))
        }
        0 | 9 => {
            if data.len() < 5 {
                return None;
            }
            let af = u32::from_le_bytes(data[0..4].try_into().ok()?);
            let et = match af {
                2 | 24 => 0x0800,
                28 | 30 => 0x86dd,
                _ => return None,
            };
            Some((et, 4))
        }
        _ => {
            if data.len() < 14 {
                return None;
            }
            let mut et = be16(data, 12)?;
            let mut off = 14usize;
            if !known_ethertype(et) && data.len() >= 16 {
                if let Some(sll) = be16(data, 14) {
                    if known_ethertype(sll) {
                        et = sll;
                        off = 16;
                    }
                }
            }
            while matches!(et, 0x8100 | 0x88a8 | 0x9100) && data.len() >= off + 4 {
                et = be16(data, off + 2)?;
                off += 4;
            }
            Some((et, off))
        }
    }
}

fn skip_ipv6_ext(data: &[u8], mut next: u8, mut off: usize) -> Option<(u8, usize)> {
    for _ in 0..8 {
        match next {
            0 | 43 | 60 => {
                if data.len() < off + 2 {
                    return None;
                }
                let hdr_len = data[off + 1] as usize;
                next = data[off];
                off += (hdr_len + 1) * 8;
            }
            44 => {
                if data.len() < off + 8 {
                    return None;
                }
                next = data[off];
                off += 8;
            }
            51 => {
                if data.len() < off + 2 {
                    return None;
                }
                let len = data[off + 1] as usize;
                next = data[off];
                off += (len + 2) * 4;
            }
            _ => return Some((next, off)),
        }
    }
    Some((next, off))
}

fn dns_qname(dns: &[u8]) -> Option<String> {
    if dns.len() < 13 {
        return None;
    }
    let mut i = 12usize;
    let mut labels = Vec::new();
    for _ in 0..16 {
        let l = *dns.get(i)? as usize;
        if l == 0 {
            break;
        }
        if l & 0xc0 == 0xc0 {
            break;
        }
        if l > 63 || i + 1 + l > dns.len() {
            return None;
        }
        labels.push(String::from_utf8_lossy(&dns[i + 1..i + 1 + l]).into_owned());
        i += 1 + l;
        if labels.join(".").len() > 253 {
            break;
        }
    }
    if labels.is_empty() {
        None
    } else {
        Some(labels.join("."))
    }
}

fn dns_rcode_name(rcode: u16) -> &'static str {
    match rcode {
        1 => "FORMERR",
        2 => "SERVFAIL",
        3 => "NXDOMAIN",
        4 => "NOTIMP",
        5 => "REFUSED",
        _ => "FAILED",
    }
}

fn icmp_unreach_name(code: u8) -> &'static str {
    match code {
        0 => "net",
        1 => "host",
        2 => "protocol",
        3 => "port",
        4 => "frag-needed",
        5 => "source-route",
        9 | 10 | 13 => "admin-prohibited",
        _ => "unreach",
    }
}

fn tls_alert(payload: &[u8]) -> Option<(u8, u8)> {
    if payload.len() < 7 {
        return None;
    }
    if payload[0] != 0x15 || payload[1] != 0x03 || payload[2] > 4 {
        return None;
    }
    let rec_len = u16::from_be_bytes([payload[3], payload[4]]) as usize;
    if !(2..16_384).contains(&rec_len) {
        return None;
    }
    Some((payload[5], payload[6]))
}

fn tls_alert_name(desc: u8) -> &'static str {
    match desc {
        0 => "close_notify",
        10 => "unexpected_message",
        20 => "bad_record_mac",
        40 => "handshake_failure",
        47 => "illegal_parameter",
        70 => "protocol_version",
        80 => "internal_error",
        86 => "inappropriate_fallback",
        90 => "user_canceled",
        112 => "unrecognized_name",
        _ => "alert",
    }
}

fn tcp_flag_names(flags: u8) -> String {
    let mut n = Vec::new();
    if flags & 0x02 != 0 {
        n.push("SYN");
    }
    if flags & 0x10 != 0 {
        n.push("ACK");
    }
    if flags & 0x01 != 0 {
        n.push("FIN");
    }
    if flags & 0x04 != 0 {
        n.push("RST");
    }
    if flags & 0x08 != 0 {
        n.push("PSH");
    }
    if flags & 0x20 != 0 {
        n.push("URG");
    }
    if n.is_empty() {
        format!("0x{flags:02x}")
    } else {
        n.join("-")
    }
}

#[derive(Default)]
struct Hits {
    retrans: Vec<(usize, String)>,
    zero_win: Vec<(usize, String)>,
    dns_fail: Vec<(usize, String)>,
    tls_alert: Vec<(usize, String)>,
    refused: Vec<(usize, String)>,
    icmp_unreach: Vec<(usize, String)>,
    arp_conflict: Vec<(usize, String)>,
    cleartext: Vec<(usize, String)>,
}

struct Analyzer {
    tcp_seq: HashMap<(String, String, u16, u16), HashSet<u32>>,
    arp_map: HashMap<String, String>,
    hits: Hits,
}

impl Analyzer {
    fn new() -> Self {
        Self {
            tcp_seq: HashMap::new(),
            arp_map: HashMap::new(),
            hits: Hits::default(),
        }
    }

    fn dissect(&mut self, idx: usize, ts: u64, data: &[u8], linktype: u32) -> PacketSummary {
        let mut summary = PacketSummary {
            index: idx,
            timestamp: ts.to_string(),
            src: String::new(),
            dst: String::new(),
            protocol: "other".into(),
            length: data.len(),
            info: String::new(),
            fields: BTreeMap::new(),
        };
        let Some((ethertype, l3)) = l3_offset(data, linktype) else {
            return summary;
        };
        match ethertype {
            0x0800 => self.ipv4(idx, data, l3, &mut summary),
            0x86dd => self.ipv6(idx, data, l3, &mut summary),
            0x0806 => self.arp(idx, data, l3, &mut summary),
            _ => {}
        }
        summary
    }

    fn ipv4(&mut self, idx: usize, data: &[u8], l3: usize, summary: &mut PacketSummary) {
        if data.len() < l3 + 20 {
            return;
        }
        let ihl = (data[l3] & 0x0f) as usize * 4;
        if ihl < 20 {
            return;
        }
        let proto = data[l3 + 9];
        let Some(src) = ipv4_str(data, l3 + 12) else {
            return;
        };
        let Some(dst) = ipv4_str(data, l3 + 16) else {
            return;
        };
        summary.src = src.clone();
        summary.dst = dst.clone();
        summary.fields.insert("ip.src".into(), src.clone());
        summary.fields.insert("ip.dst".into(), dst.clone());
        let frag = be16(data, l3 + 6).unwrap_or(0);
        if frag & 0x1fff != 0 {
            summary.protocol = "ipv4".into();
            summary.info = format!("{src} → {dst} fragment");
            return;
        }
        self.l4(idx, data, l3 + ihl, proto, src, dst, summary);
    }

    fn ipv6(&mut self, idx: usize, data: &[u8], l3: usize, summary: &mut PacketSummary) {
        if data.len() < l3 + 40 {
            return;
        }
        let next = data[l3 + 6];
        let Some(src) = ipv6_str(data, l3 + 8) else {
            return;
        };
        let Some(dst) = ipv6_str(data, l3 + 24) else {
            return;
        };
        summary.src = src.clone();
        summary.dst = dst.clone();
        summary.fields.insert("ip.src".into(), src.clone());
        summary.fields.insert("ip.dst".into(), dst.clone());
        let Some((proto, l4)) = skip_ipv6_ext(data, next, l3 + 40) else {
            return;
        };
        self.l4(idx, data, l4, proto, src, dst, summary);
    }

    fn l4(
        &mut self,
        idx: usize,
        data: &[u8],
        l4: usize,
        proto: u8,
        src: String,
        dst: String,
        summary: &mut PacketSummary,
    ) {
        match proto {
            6 => self.tcp(idx, data, l4, src, dst, summary),
            17 => self.udp(idx, data, l4, src, dst, summary),
            1 => self.icmp4(idx, data, l4, src, dst, summary),
            58 => self.icmp6(idx, data, l4, src, dst, summary),
            _ => {
                summary.protocol = format!("ip-{proto}");
                summary.info = format!("{src} → {dst} proto={proto}");
            }
        }
    }

    fn tcp(
        &mut self,
        idx: usize,
        data: &[u8],
        l4: usize,
        src: String,
        dst: String,
        summary: &mut PacketSummary,
    ) {
        if data.len() < l4 + 20 {
            return;
        }
        let Some(sport) = be16(data, l4) else {
            return;
        };
        let Some(dport) = be16(data, l4 + 2) else {
            return;
        };
        let seq = be32(data, l4 + 4).unwrap_or(0);
        let window = be16(data, l4 + 14).unwrap_or(0);
        let flags = data[l4 + 13];
        let hdr_len = ((data[l4 + 12] >> 4) as usize).saturating_mul(4).max(20);
        let payload_off = l4 + hdr_len;
        let payload_len = data.len().saturating_sub(payload_off);
        let syn = flags & 0x02 != 0;
        let ack = flags & 0x10 != 0;
        let fin = flags & 0x01 != 0;
        let rst = flags & 0x04 != 0;
        let names = tcp_flag_names(flags);
        summary.protocol = "tcp".into();
        summary.info = format!("{src}:{sport} → {dst}:{dport} {names} seq={seq} win={window}");
        summary
            .fields
            .insert("tcp.srcport".into(), sport.to_string());
        summary
            .fields
            .insert("tcp.dstport".into(), dport.to_string());
        summary
            .fields
            .insert("tcp.window".into(), window.to_string());
        summary.fields.insert("tcp.seq".into(), seq.to_string());
        summary.fields.insert("tcp.flags".into(), names.clone());
        let ev = format!("{src}:{sport} → {dst}:{dport} {names} seq={seq}");
        if rst {
            self.hits.refused.push((idx, format!("{ev} (RST)")));
        }
        if window == 0 && !rst && !syn {
            self.hits.zero_win.push((
                idx,
                format!("{src}:{sport} → {dst}:{dport} win=0 {}", names),
            ));
        }
        if payload_len > 0 || syn || fin {
            let key = (src.clone(), dst.clone(), sport, dport);
            if !self.tcp_seq.entry(key).or_default().insert(seq) {
                self.hits.retrans.push((idx, ev.clone()));
            }
        }
        let payload = data.get(payload_off..).unwrap_or(&[]);
        if let Some((level, desc)) = tls_alert(payload) {
            let fatal = if level == 2 { "fatal" } else { "warning" };
            self.hits.tls_alert.push((
                idx,
                format!(
                    "{src}:{sport} → {dst}:{dport} TLS {fatal} {} ({desc})",
                    tls_alert_name(desc)
                ),
            ));
            summary.info = format!(
                "{} TLS alert {} {}",
                summary.info,
                fatal,
                tls_alert_name(desc)
            );
            summary.fields.insert("tls.alert".into(), desc.to_string());
        }
        if (sport == 80 || dport == 80) && !payload.is_empty() {
            let peek =
                String::from_utf8_lossy(&payload[..payload.len().min(240)]).to_ascii_lowercase();
            if peek.contains("authorization:") || peek.contains("password=") {
                self.hits.cleartext.push((
                    idx,
                    format!("{src}:{sport} → {dst}:{dport} HTTP Authorization or password="),
                ));
            }
        }
        let _ = ack;
    }

    fn udp(
        &mut self,
        idx: usize,
        data: &[u8],
        l4: usize,
        src: String,
        dst: String,
        summary: &mut PacketSummary,
    ) {
        if data.len() < l4 + 8 {
            return;
        }
        let Some(sport) = be16(data, l4) else {
            return;
        };
        let Some(dport) = be16(data, l4 + 2) else {
            return;
        };
        summary.protocol = "udp".into();
        summary.info = format!("{src}:{sport} → {dst}:{dport}");
        summary
            .fields
            .insert("udp.srcport".into(), sport.to_string());
        summary
            .fields
            .insert("udp.dstport".into(), dport.to_string());
        if sport != 53 && dport != 53 {
            return;
        }
        summary.protocol = "dns".into();
        let dns = data.get(l4 + 8..).unwrap_or(&[]);
        if dns.len() < 4 {
            return;
        }
        let flags = be16(dns, 2).unwrap_or(0);
        let rcode = flags & 0x000f;
        let response = flags & 0x8000 != 0;
        summary.fields.insert("dns.rcode".into(), rcode.to_string());
        let qname = dns_qname(dns).unwrap_or_default();
        if !qname.is_empty() {
            summary.fields.insert("dns.qname".into(), qname.clone());
        }
        summary.info = if qname.is_empty() {
            format!("{src}:{sport} → {dst}:{dport} dns rcode={rcode}")
        } else {
            format!("{src}:{sport} → {dst}:{dport} {qname} rcode={rcode}")
        };
        if response && rcode != 0 {
            self.hits.dns_fail.push((
                idx,
                format!(
                    "{src}:{sport} → {dst}:{dport} {} rcode={rcode} {}",
                    if qname.is_empty() {
                        "query"
                    } else {
                        qname.as_str()
                    },
                    dns_rcode_name(rcode)
                ),
            ));
        }
    }

    fn icmp4(
        &mut self,
        idx: usize,
        data: &[u8],
        l4: usize,
        src: String,
        dst: String,
        summary: &mut PacketSummary,
    ) {
        if data.len() < l4 + 2 {
            return;
        }
        let typ = data[l4];
        let code = data[l4 + 1];
        summary.protocol = "icmp".into();
        summary.fields.insert("icmp.type".into(), typ.to_string());
        summary.fields.insert("icmp.code".into(), code.to_string());
        let inner = ipv4_str(data, l4 + 8 + 16).unwrap_or_default();
        if typ == 3 {
            let name = icmp_unreach_name(code);
            summary.info = if inner.is_empty() {
                format!("{src} → {dst} dest-unreach {name} code={code}")
            } else {
                format!("{src} → {dst} dest-unreach {name} for {inner}")
            };
            self.hits.icmp_unreach.push((idx, summary.info.clone()));
        } else {
            summary.info = format!("{src} → {dst} icmp type={typ} code={code}");
        }
    }

    fn icmp6(
        &mut self,
        idx: usize,
        data: &[u8],
        l4: usize,
        src: String,
        dst: String,
        summary: &mut PacketSummary,
    ) {
        if data.len() < l4 + 2 {
            return;
        }
        let typ = data[l4];
        let code = data[l4 + 1];
        summary.protocol = "icmpv6".into();
        summary.fields.insert("icmp.type".into(), typ.to_string());
        summary.fields.insert("icmp.code".into(), code.to_string());
        if typ == 1 {
            summary.info = format!("{src} → {dst} dest-unreach code={code}");
            self.hits.icmp_unreach.push((idx, summary.info.clone()));
        } else {
            summary.info = format!("{src} → {dst} icmpv6 type={typ} code={code}");
        }
    }

    fn arp(&mut self, idx: usize, data: &[u8], l3: usize, summary: &mut PacketSummary) {
        // Ethernet ARP: SHA@8, SPA@14, THA@18, TPA@24 relative to ARP payload (l3)
        if data.len() < l3 + 28 {
            return;
        }
        let Some(spa) = ipv4_str(data, l3 + 14) else {
            return;
        };
        let sha = data[l3 + 8..l3 + 14]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(":");
        summary.protocol = "arp".into();
        summary.src = spa.clone();
        summary.info = format!("ARP {spa} is-at {sha}");
        summary.fields.insert("arp.spa".into(), spa.clone());
        summary.fields.insert("arp.sha".into(), sha.clone());
        if let Some(prev) = self.arp_map.get(&spa) {
            if prev != &sha {
                self.hits
                    .arp_conflict
                    .push((idx, format!("{spa} claimed by {prev} and {sha}")));
            }
        } else {
            self.arp_map.insert(spa, sha);
        }
    }

    fn findings(self) -> Vec<PcapFinding> {
        let mut out = Vec::new();
        push_finding(
            &mut out,
            "retransmit",
            "TCP retransmissions",
            &self.hits.retrans,
        );
        push_finding(
            &mut out,
            "zero_window",
            "TCP zero-window stalls",
            &self.hits.zero_win,
        );
        push_finding(
            &mut out,
            "dns_failure",
            "DNS failures (nonzero rcode)",
            &self.hits.dns_fail,
        );
        push_finding(
            &mut out,
            "tls_alert",
            "TLS alert records",
            &self.hits.tls_alert,
        );
        push_finding(
            &mut out,
            "refused",
            "TCP RST / refused connections",
            &self.hits.refused,
        );
        push_finding(
            &mut out,
            "icmp_unreach",
            "ICMP destination unreachable",
            &self.hits.icmp_unreach,
        );
        push_finding(
            &mut out,
            "arp_conflict",
            "ARP IP claimed by multiple MACs",
            &self.hits.arp_conflict,
        );
        push_finding(
            &mut out,
            "cleartext_creds",
            "Cleartext credentials on the wire",
            &self.hits.cleartext,
        );
        out
    }
}

fn analyze_frames(linktype: u32, frames: Vec<(u64, Vec<u8>)>) -> ParseResult {
    let mut analyzer = Analyzer::new();
    let mut packets = Vec::new();
    let mut idx = 0usize;
    for (ts, data_buf) in frames {
        let summary = analyzer.dissect(idx, ts, &data_buf, linktype);
        if packets.len() < STORE_PACKET_CAP {
            packets.push(summary);
        }
        idx += 1;
        if idx >= SCAN_PACKET_CAP {
            break;
        }
    }
    ParseResult {
        truncated: idx >= SCAN_PACKET_CAP || idx > packets.len(),
        total: idx,
        findings: analyzer.findings(),
        packets,
    }
}

pub fn parse_pcap(path: &Path) -> Result<ParseResult> {
    let bytes = std::fs::read(path).map_err(|e| LateError::Pcap(e.to_string()))?;
    let (linktype, frames) = pcap_frames(&bytes).or_else(|_| {
        let file = File::open(path).map_err(|e| LateError::Pcap(e.to_string()))?;
        let mut reader = PcapReader::new(file).map_err(|e| LateError::Pcap(e.to_string()))?;
        let linktype = u32::from(reader.header().datalink);
        let mut frames = Vec::new();
        while let Some(pkt) = reader.next_packet() {
            let Ok(pkt) = pkt else { break };
            frames.push((pkt.timestamp.as_micros() as u64, pkt.data.into_owned()));
        }
        Ok::<_, LateError>((linktype, frames))
    })?;
    Ok(analyze_frames(linktype, frames))
}

fn push_finding(out: &mut Vec<PcapFinding>, kind: &str, summary: &str, hits: &[(usize, String)]) {
    if hits.is_empty() {
        return;
    }
    let packet_indexes: Vec<usize> = hits
        .iter()
        .map(|(i, _)| *i)
        .filter(|i| *i < STORE_PACKET_CAP)
        .take(80)
        .collect();
    let evidence = hits
        .iter()
        .take(8)
        .map(|(i, e)| format!("#{i} {e}"))
        .collect::<Vec<_>>()
        .join("; ");
    out.push(PcapFinding {
        id: uuid::Uuid::new_v4().to_string(),
        kind: kind.into(),
        summary: format!("{summary} ({})", hits.len()),
        packet_indexes,
        evidence,
    });
}

fn findings_for_agent(findings: &[PcapFinding]) -> Vec<PcapFinding> {
    findings
        .iter()
        .map(|f| {
            let mut c = f.clone();
            c.packet_indexes.truncate(24);
            if c.evidence.len() > 800 {
                c.evidence.truncate(800);
                c.evidence.push('…');
            }
            c
        })
        .collect()
}

fn packet_digest(p: &PacketSummary) -> serde_json::Value {
    serde_json::json!({
        "index": p.index,
        "src": p.src,
        "dst": p.dst,
        "protocol": p.protocol,
        "length": p.length,
        "info": p.info,
        "fields": p.fields,
    })
}

pub fn query_analysis(
    packets: &[PacketSummary],
    findings: &[PcapFinding],
    q: &str,
) -> serde_json::Value {
    let findings = findings_for_agent(findings);
    let ql = q.trim().to_ascii_lowercase();
    if ql.is_empty() || ql == "summary" || ql == "findings" || ql == "overview" {
        let mut protocols = BTreeMap::new();
        let mut talkers: BTreeMap<String, u32> = BTreeMap::new();
        for p in packets {
            *protocols.entry(p.protocol.clone()).or_insert(0u32) += 1;
            if !p.src.is_empty() {
                *talkers.entry(p.src.clone()).or_insert(0u32) += 1;
            }
        }
        let mut top: Vec<_> = talkers.into_iter().collect();
        top.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let top_talkers: Vec<_> = top
            .into_iter()
            .take(8)
            .map(|(ip, n)| serde_json::json!({"ip": ip, "packets": n}))
            .collect();
        let sample: Vec<_> = packets.iter().take(30).map(packet_digest).collect();
        return serde_json::json!({
            "packet_count": packets.len(),
            "protocols": protocols,
            "findings": findings,
            "top_talkers": top_talkers,
            "sample": sample,
            "note": "header-derived analysis only; payload bytes are never included"
        });
    }
    let hits: Vec<_> = packets
        .iter()
        .filter(|p| {
            p.info.to_ascii_lowercase().contains(&ql)
                || p.src.contains(q)
                || p.dst.contains(q)
                || p.protocol.to_ascii_lowercase().contains(&ql)
                || p.fields
                    .iter()
                    .any(|(k, v)| k.contains(&ql) || v.to_ascii_lowercase().contains(&ql))
        })
        .take(50)
        .map(packet_digest)
        .collect();
    serde_json::json!({
        "findings": findings,
        "match_count": hits.len(),
        "matches": hits,
        "note": "header-derived analysis only; payload bytes are never included"
    })
}

pub fn filter_packets(packets: &[PacketSummary], expr: &str) -> Vec<PacketSummary> {
    if expr.trim().is_empty() {
        return ui_packets(packets);
    }
    let parts: Vec<&str> = expr.split(" and ").collect();
    packets
        .iter()
        .filter(|p| {
            parts.iter().all(|part| {
                let part = part.trim();
                if let Some((k, v)) = part.split_once("==") {
                    let k = k.trim();
                    let v = v.trim().trim_matches('"');
                    p.fields.get(k).map(|x| x == v).unwrap_or(false)
                        || (k == "ip.src" && p.src == v)
                        || (k == "ip.dst" && p.dst == v)
                } else if let Some((k, v)) = part.split_once("!=") {
                    let k = k.trim();
                    let v = v.trim();
                    p.fields.get(k).map(|x| x != v).unwrap_or(true)
                } else {
                    p.info.contains(part) || p.protocol.contains(part)
                }
            })
        })
        .cloned()
        .take(UI_PACKET_CAP)
        .collect()
}

pub fn ui_packets(packets: &[PacketSummary]) -> Vec<PacketSummary> {
    packets.iter().take(UI_PACKET_CAP).cloned().collect()
}

pub fn remote_tcpdump_sh(iface: &str, count: u32, bpf: &str) -> Result<String> {
    validate_iface(iface)?;
    let extra = bpf_argv(bpf)?;
    let count = count.clamp(1, 2_000);
    Ok(format!(
        "F=/tmp/late-pcap.$$; \
         run() {{ if command -v timeout >/dev/null 2>&1; then timeout 18 \"$@\"; else \"$@\"; fi; }}; \
         dump() {{ run \"$@\" -nn -U -s 128 -c {count} -i {iface} -w \"$F\"{extra}; }}; \
         ok=0; \
         if command -v tcpdump >/dev/null 2>&1 && sudo -n true >/dev/null 2>&1; then \
           dump sudo -n tcpdump && ok=1; \
         fi; \
         if [ \"$ok\" != 1 ] && command -v tcpdump >/dev/null 2>&1; then \
           dump tcpdump && ok=1; \
         fi; \
         if [ \"$ok\" != 1 ] && command -v dumpcap >/dev/null 2>&1; then \
           run dumpcap -q -P -i {iface} -c {count} -w \"$F\" && ok=1; \
         fi; \
         if [ ! -s \"$F\" ]; then echo \"late-pcap: tcpdump/dumpcap missing or not permitted on this host\" >&2; exit 1; fi; \
         cat \"$F\"; rm -f \"$F\""
    ))
}

/// Stream pcap on stdout until SSH is killed (Start/Stop).
pub fn remote_tcpdump_live_sh(iface: &str, bpf: &str) -> Result<String> {
    validate_iface(iface)?;
    let extra = bpf_argv(bpf)?;
    let dumpcap_f = if extra.is_empty() {
        String::new()
    } else {
        format!(" -f{extra}")
    };
    Ok(format!(
        "if command -v tcpdump >/dev/null 2>&1 && sudo -n true >/dev/null 2>&1; then \
           exec sudo -n tcpdump -nn -U -s 128 -i {iface} -w -{extra}; \
         fi; \
         if command -v tcpdump >/dev/null 2>&1; then \
           exec tcpdump -nn -U -s 128 -i {iface} -w -{extra}; \
         fi; \
         if command -v dumpcap >/dev/null 2>&1; then \
           exec dumpcap -q -P -i {iface} -w -{dumpcap_f}; \
         fi; \
         echo 'late-pcap: tcpdump/dumpcap missing or not permitted on this host' >&2; exit 1"
    ))
}

pub fn open_in_wireshark(path: &Path) -> Result<()> {
    let bin = ["wireshark", "wireshark-qt", "wireshark-gtk"]
        .into_iter()
        .find(|b| {
            Command::new("which")
                .arg(b)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        })
        .ok_or_else(|| LateError::Pcap("Wireshark is not installed on this host".into()))?;
    Command::new(bin)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| LateError::Pcap(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eth_ipv4(proto: u8, src: [u8; 4], dst: [u8; 4], l4: &[u8]) -> Vec<u8> {
        let mut pkt = vec![0u8; 14];
        pkt[12] = 0x08;
        pkt[13] = 0x00;
        let total = 20 + l4.len();
        let mut ip = vec![0u8; 20];
        ip[0] = 0x45;
        ip[2] = (total >> 8) as u8;
        ip[3] = total as u8;
        ip[8] = 64;
        ip[9] = proto;
        ip[12..16].copy_from_slice(&src);
        ip[16..20].copy_from_slice(&dst);
        pkt.extend_from_slice(&ip);
        pkt.extend_from_slice(l4);
        pkt
    }

    fn tcp(sport: u16, dport: u16, seq: u32, flags: u8, window: u16, payload: &[u8]) -> Vec<u8> {
        let mut h = vec![0u8; 20];
        h[0..2].copy_from_slice(&sport.to_be_bytes());
        h[2..4].copy_from_slice(&dport.to_be_bytes());
        h[4..8].copy_from_slice(&seq.to_be_bytes());
        h[12] = 0x50;
        h[13] = flags;
        h[14..16].copy_from_slice(&window.to_be_bytes());
        h.extend_from_slice(payload);
        h
    }

    fn udp(sport: u16, dport: u16, payload: &[u8]) -> Vec<u8> {
        let mut h = vec![0u8; 8];
        h[0..2].copy_from_slice(&sport.to_be_bytes());
        h[2..4].copy_from_slice(&dport.to_be_bytes());
        let len = 8 + payload.len();
        h[4..6].copy_from_slice(&(len as u16).to_be_bytes());
        h.extend_from_slice(payload);
        h
    }

    fn parse_eth(frames: Vec<Vec<u8>>) -> ParseResult {
        let tagged = frames
            .into_iter()
            .enumerate()
            .map(|(i, d)| (i as u64, d))
            .collect();
        analyze_frames(1, tagged)
    }

    fn kinds(r: &ParseResult) -> Vec<&str> {
        r.findings.iter().map(|f| f.kind.as_str()).collect()
    }

    #[test]
    fn retransmit_and_not_pure_ack() {
        let a = [10, 0, 0, 1];
        let b = [10, 0, 0, 2];
        let r = parse_eth(vec![
            eth_ipv4(6, a, b, &tcp(1234, 80, 100, 0x18, 64, b"hello")),
            eth_ipv4(6, a, b, &tcp(1234, 80, 100, 0x18, 64, b"hello")),
            eth_ipv4(6, b, a, &tcp(80, 1234, 50, 0x10, 64, b"")),
            eth_ipv4(6, b, a, &tcp(80, 1234, 50, 0x10, 64, b"")),
        ]);
        assert!(kinds(&r).contains(&"retransmit"), "{:?}", r.findings);
        let re = r.findings.iter().find(|f| f.kind == "retransmit").unwrap();
        assert_eq!(re.packet_indexes, vec![1]);
        assert!(re.evidence.contains("seq=100"));
    }

    #[test]
    fn zero_window_rst_dns_tls_icmp() {
        let a = [10, 0, 0, 1];
        let b = [10, 0, 0, 2];
        let mut dns = vec![
            0x12, 0x34, 0x81, 0x83, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        dns.extend_from_slice(&[7]);
        dns.extend_from_slice(b"example");
        dns.extend_from_slice(&[3]);
        dns.extend_from_slice(b"com");
        dns.extend_from_slice(&[0, 0, 1, 0, 1]);
        let tls = [0x15, 0x03, 0x03, 0x00, 0x02, 0x02, 0x28];
        let mut icmp = vec![3, 3, 0, 0, 0, 0, 0, 0];
        let mut inner = vec![0u8; 20];
        inner[0] = 0x45;
        inner[9] = 6;
        inner[16..20].copy_from_slice(&[8, 8, 8, 8]);
        icmp.extend_from_slice(&inner);
        icmp.extend_from_slice(&[0u8; 8]);
        let r = parse_eth(vec![
            eth_ipv4(6, a, b, &tcp(1234, 443, 1, 0x10, 0, b"")),
            eth_ipv4(6, b, a, &tcp(443, 1234, 1, 0x14, 0, b"")),
            eth_ipv4(17, b, a, &udp(53, 5555, &dns)),
            eth_ipv4(6, b, a, &tcp(443, 1234, 9, 0x18, 64, &tls)),
            eth_ipv4(1, [10, 0, 0, 9], a, &icmp),
        ]);
        let k = kinds(&r);
        assert!(k.contains(&"zero_window"), "{k:?}");
        assert!(k.contains(&"refused"), "{k:?}");
        assert!(k.contains(&"dns_failure"), "{k:?}");
        assert!(k.contains(&"tls_alert"), "{k:?}");
        assert!(k.contains(&"icmp_unreach"), "{k:?}");
        let dns_f = r.findings.iter().find(|f| f.kind == "dns_failure").unwrap();
        assert!(dns_f.evidence.contains("NXDOMAIN") || dns_f.evidence.contains("example.com"));
        let tls_f = r.findings.iter().find(|f| f.kind == "tls_alert").unwrap();
        assert!(tls_f.evidence.contains("handshake_failure") || tls_f.evidence.contains("TLS"));
        let icmp_f = r
            .findings
            .iter()
            .find(|f| f.kind == "icmp_unreach")
            .unwrap();
        assert!(icmp_f.evidence.contains("8.8.8.8") || icmp_f.evidence.contains("port"));
    }

    #[test]
    fn query_analysis_includes_findings_not_payload() {
        let a = [10, 0, 0, 1];
        let b = [10, 0, 0, 2];
        let r = parse_eth(vec![eth_ipv4(
            6,
            a,
            b,
            &tcp(1234, 80, 1, 0x18, 64, b"POST /login password=supersecret"),
        )]);
        let v = query_analysis(&r.packets, &r.findings, "summary");
        let s = v.to_string();
        assert!(v.get("findings").is_some());
        assert!(v.get("note").unwrap().as_str().unwrap().contains("payload"));
        assert!(!s.contains("supersecret"));
        assert!(s.contains("cleartext") || r.findings.iter().any(|f| f.kind == "cleartext_creds"));
        let blob = serde_json::to_string(&v).unwrap();
        assert!(!blob.contains("POST /login"));
    }

    #[test]
    fn bpf_quotes_and_rejects_shell_meta() {
        assert!(validate_bpf("host 1.1.1.1").is_ok());
        assert!(validate_bpf("tcp; id").is_err());
        assert!(validate_bpf("port 22 | cat").is_err());
        let sh = remote_tcpdump_sh("eth0", 10, "host 1.1.1.1").unwrap();
        assert!(sh.contains("'host 1.1.1.1'"), "{sh}");
    }
}
