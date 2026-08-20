//! Vendor-agnostic SSH packet capture.
//!
//! Probe the far end, then try strategies in order: unix tcpdump/dumpcap,
//! a unix shell on a network OS, then that OS's own capture CLI. The
//! inventory vendor field is only a hint — detection comes from MOTD/`show version`.

use crate::error::{LateError, Result};
use crate::pcap::{self, looks_like_pcap, LiveCapture};
use crate::secrets::SecretStore;
use crate::ssh::{self, CliSession};
use crate::types::{AuthProfile, Device, Vendor};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

const AOS_CX_NO_SHELL: &str = "AOS-CX captured packets but could not export the pcap: start-shell is missing or blocked on this image. Use copy tcpdump-pcap to pull files from /tmp/tcpdump/ off the switch.";

pub fn capture(
    device: &Device,
    profile: &AuthProfile,
    secrets: &SecretStore,
    iface: &str,
    count: u32,
    bpf: &str,
) -> Result<Vec<u8>> {
    pcap::validate_iface(iface)?;
    pcap::validate_bpf(bpf)?;
    let count = count.clamp(1, 500);
    let mut notes = Vec::new();

    let sh = pcap::remote_tcpdump_sh(iface, count, bpf)?;
    match ssh::exec_bytes_result(
        device,
        profile,
        secrets,
        &sh,
        Duration::from_secs(8),
        8 * 1024 * 1024,
    ) {
        Ok((out, _, _)) if looks_like_pcap(&out) => return Ok(out),
        Ok((out, err, _)) => notes.push(pcap::explain_remote_capture_failure(&out, &err)),
        Err(e) => notes.push(e.to_string()),
    }

    match cli_capture(device, profile, secrets, iface, count, bpf) {
        Ok(bytes) => Ok(bytes),
        Err(e) => {
            notes.push(e.to_string());
            Err(LateError::Pcap(format!(
                "SSH capture did not return a pcap (tried unix tcpdump/dumpcap, then a CLI shell, then the device capture command). {}",
                notes.last().cloned().unwrap_or_default()
            )))
        }
    }
}

fn cli_capture(
    device: &Device,
    profile: &AuthProfile,
    secrets: &SecretStore,
    iface: &str,
    count: u32,
    bpf: &str,
) -> Result<Vec<u8>> {
    let mut cli = CliSession::open(device, profile, secrets)?;
    let kind = prepare_cli(&mut cli, device.vendor)?;

    if let Some(bytes) = try_unix_shell(&mut cli, iface, count, bpf) {
        return Ok(bytes);
    }

    match kind {
        Platform::AosCx => aos_cx(&mut cli, count, bpf),
        Platform::Nxos => nxos(&mut cli, iface, count),
        Platform::Eos => eos(&mut cli, iface, count, bpf),
        Platform::Junos => junos(&mut cli, iface, count),
        Platform::Unix => Err(LateError::Pcap(
            "unix shell is present but tcpdump/dumpcap did not produce a pcap".into(),
        )),
        Platform::Unknown => Err(LateError::Pcap(
            "this SSH CLI is not a unix tcpdump host, and no network-OS capture command succeeded"
                .into(),
        )),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Platform {
    Unix,
    AosCx,
    Nxos,
    Eos,
    Junos,
    Unknown,
}

fn classify(text: &str, hint: Vendor) -> Platform {
    let t = text.to_lowercase();
    if t.contains("aos-cx")
        || t.contains("arubaos-cx")
        || t.contains("arubanetworks")
        || t.contains("hewlett packard enterprise")
        || t.contains("aruba")
    {
        return Platform::AosCx;
    }
    if t.contains("nx-os") || t.contains("nexus") {
        return Platform::Nxos;
    }
    if t.contains("arista") || (t.contains("eos") && t.contains("arista")) {
        return Platform::Eos;
    }
    if t.contains("junos") || t.contains("juniper") {
        return Platform::Junos;
    }
    if t.contains("linux") || t.contains("ubuntu") || t.contains("debian") || t.contains("red hat")
    {
        return Platform::Unix;
    }
    match hint {
        Vendor::Linux => Platform::Unix,
        Vendor::AosCx => Platform::AosCx,
        Vendor::CiscoNxos => Platform::Nxos,
        Vendor::AristaEos => Platform::Eos,
        Vendor::Junos => Platform::Junos,
        _ => Platform::Unknown,
    }
}

fn prepare_cli(cli: &mut CliSession, hint: Vendor) -> Result<Platform> {
    cli.wait_prompt(Duration::from_secs(20))?;
    maybe_enable(cli);
    // AOS-CX paging: `no page`. Harmless no-ops on other CLIs.
    cli.send("no page")?;
    cli.send("terminal length 0")?;
    cli.send("set cli screen-length 0")?;
    cli.wait(Duration::from_millis(500));
    cli.send("show version")?;
    let _ = cli.wait(Duration::from_secs(4));
    Ok(classify(&cli.since(0), hint))
}

fn maybe_enable(cli: &mut CliSession) {
    let t = cli.since(0);
    let line = t
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim_end();
    if !line.ends_with('>') {
        return;
    }
    let mark = cli.len();
    let _ = cli.send("enable");
    let _ = cli.wait(Duration::from_secs(2));
    if cli.since(mark).to_lowercase().contains("password") {
        let _ = cli.interrupt();
        let _ = cli.wait(Duration::from_millis(400));
    }
}

fn looks_like_license_wall(text: &str) -> bool {
    let t = text.to_lowercase();
    t.contains("end user license")
        || t.contains("license agreement")
        || t.contains("legal agreement")
        || t.contains("do you accept")
        || t.contains("accept (yes/no)")
        || (t.contains("restricted rights") && t.contains("accept"))
}

fn abort_cli_dialog(cli: &mut CliSession) {
    let _ = cli.interrupt();
    let _ = cli.send("q");
    let _ = cli.send("no");
    let _ = cli.wait(Duration::from_millis(600));
}

fn has_unix_echo(text: &str) -> bool {
    text.lines().any(|l| l.trim() == "LATE_UNIX")
}

fn looks_like_unix_prompt(text: &str) -> bool {
    let line = text
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim_end();
    line.ends_with('$') || line.ends_with("$ ") || line.contains("~$")
}

fn try_unix_shell(cli: &mut CliSession, iface: &str, count: u32, bpf: &str) -> Option<Vec<u8>> {
    for cmd in [
        "echo LATE_UNIX",
        "start-shell",
        "start shell",
        "bash",
        "run bash",
        "guestshell",
    ] {
        let mark = cli.len();
        cli.send(cmd).ok()?;
        let _out = cli.wait(Duration::from_secs(3));
        let chunk = cli.since(mark);
        if looks_like_license_wall(&chunk) {
            abort_cli_dialog(cli);
            continue;
        }
        let unixish = has_unix_echo(&chunk)
            || looks_like_unix_prompt(&chunk)
            || chunk.contains("sh-")
            || chunk.contains("bash-");
        if cmd != "echo LATE_UNIX" && unixish {
            cli.send("echo LATE_UNIX").ok()?;
            let confirm = cli.wait(Duration::from_secs(2));
            if !has_unix_echo(&confirm) {
                continue;
            }
            return unix_tcpdump(cli, iface, count, bpf);
        }
        if cmd == "echo LATE_UNIX" && has_unix_echo(&chunk) {
            return unix_tcpdump(cli, iface, count, bpf);
        }
    }
    None
}

fn unix_tcpdump(cli: &mut CliSession, iface: &str, count: u32, bpf: &str) -> Option<Vec<u8>> {
    let extra = if bpf.trim().is_empty() {
        String::new()
    } else {
        format!(" {}", bpf.trim())
    };
    // Short lines: AOS-CX start-shell PTYs wrap long one-liners and break them.
    cli.send(&format!(
        "sudo -n tcpdump -nn -U -s 128 -c {count} -i {iface} -w /tmp/late-pcap.pcap{extra}"
    ))
    .ok()?;
    let _ = cli.wait_until(Duration::from_secs(25), |s| {
        looks_like_unix_prompt(s) || s.to_lowercase().contains("packets captured")
    });
    export_unix_file(cli, "/tmp/late-pcap.pcap")
}

fn aos_cx_filter(bpf: &str) -> String {
    // AOS-CX blocks -i/--interface; only CPU-mirrored traffic is captured.
    let t = bpf.trim();
    if t.is_empty() {
        String::new()
    } else {
        format!(" {t}")
    }
}

fn aos_cx_ended(s: &str) -> bool {
    let l = s.to_lowercase();
    l.contains("ending traffic capture") || l.contains("packets captured")
}

fn aos_cx_enter_shell(cli: &mut CliSession) -> bool {
    let mark = cli.len();
    if cli.send("start-shell").is_err() {
        return false;
    }
    let _ = cli.wait(Duration::from_secs(3));
    let chunk = cli.since(mark);
    if looks_like_license_wall(&chunk) && !looks_like_unix_prompt(&chunk) {
        abort_cli_dialog(cli);
        return false;
    }
    if cli.send("echo LATE_UNIX").is_err() {
        return false;
    }
    let confirm = cli.wait(Duration::from_secs(2));
    if has_unix_echo(&confirm) {
        return true;
    }
    abort_cli_dialog(cli);
    false
}

fn aos_cx_export(cli: &mut CliSession) -> Result<Vec<u8>> {
    if !aos_cx_enter_shell(cli) {
        return Err(LateError::Pcap(AOS_CX_NO_SHELL.into()));
    }
    let mark = cli.len();
    cli.send(
        "echo LATEB64_START; \
         if [ -f /tmp/tcpdump/latepcap.pcap ]; then base64 /tmp/tcpdump/latepcap.pcap; \
         elif [ -f /tmp/tcpdump/latepcap1.pcap ]; then base64 /tmp/tcpdump/latepcap1.pcap; \
         elif [ -f /tmp/tcpdump/latepcap.pcap1 ]; then base64 /tmp/tcpdump/latepcap.pcap1; \
         else f=$(ls -1t /tmp/tcpdump/*.pcap 2>/dev/null | head -n 1); [ -n \"$f\" ] && base64 \"$f\"; fi; \
         echo LATEB64_END",
    )?;
    let out = cli.wait_until(Duration::from_secs(8), has_b64_end_line);
    decode_marked(&cli.since(mark))
        .or_else(|| decode_marked(&out))
        .ok_or_else(|| LateError::Pcap(AOS_CX_NO_SHELL.into()))
}

fn enter_unix_shell(cli: &mut CliSession) -> bool {
    aos_cx_enter_shell(cli)
}

fn live_unix_tcpdump(
    cli: &mut CliSession,
    iface: &str,
    bpf: &str,
    outfile: &Path,
    rx: &mpsc::Receiver<()>,
    running: &AtomicBool,
) -> Result<()> {
    let extra = if bpf.trim().is_empty() {
        String::new()
    } else {
        format!(" {}", bpf.trim())
    };
    cli.send(&format!(
        "sudo -n tcpdump -nn -U -s 128 -i {iface} -w /tmp/late-live.pcap{extra}"
    ))?;
    let _ = cli.wait_until(Duration::from_secs(8), |s| {
        s.to_lowercase().contains("listening on")
    });
    running.store(true, Ordering::SeqCst);
    wait_stop(cli, rx);
    cli.interrupt()?;
    let _ = cli.wait_until(Duration::from_secs(8), |s| {
        looks_like_unix_prompt(s) || s.to_lowercase().contains("packets captured")
    });
    let bytes = export_unix_file(cli, "/tmp/late-live.pcap").ok_or_else(|| {
        LateError::Pcap("SSH shell tcpdump stopped but could not export /tmp/late-live.pcap".into())
    })?;
    write_bytes(outfile, &bytes)
}

fn aos_cx(cli: &mut CliSession, count: u32, bpf: &str) -> Result<Vec<u8>> {
    if aos_cx_enter_shell(cli) {
        return unix_tcpdump(cli, "any", count, bpf).ok_or_else(|| {
            LateError::Pcap("AOS-CX start-shell is up but sudo tcpdump did not write a pcap".into())
        });
    }
    let extra = aos_cx_filter(bpf);
    cli.send("diag utilities tcpdump delete-file latepcap.pcap")?;
    cli.wait(Duration::from_millis(800));
    cli.send(&format!(
        "diag utilities tcpdump command -c {count} -w latepcap.pcap{extra}"
    ))?;
    let captured = cli.wait_until(Duration::from_secs(22), aos_cx_ended);
    if !aos_cx_ended(&captured) {
        let _ = cli.interrupt();
        let _ = cli.wait_until(Duration::from_secs(6), |s| {
            aos_cx_ended(s) || ssh::looks_like_cli_prompt(s)
        });
    }
    aos_cx_export(cli)
}

fn nxos(cli: &mut CliSession, iface: &str, count: u32) -> Result<Vec<u8>> {
    let ifc = if iface == "any" { "inband" } else { iface };
    cli.send(&format!(
        "ethanalyzer local interface {ifc} limit-captured-frames {count} write bootflash:latepcap.pcap"
    ))?;
    cli.wait_until(Duration::from_secs(22), |s| {
        s.to_lowercase().contains("captured") || s.contains('#')
    });
    try_unix_shell(cli, ifc, count, "").ok_or_else(|| {
        LateError::Pcap(
            "NX-OS ethanalyzer ran but the pcap could not be exported (need run bash / guestshell)"
                .into(),
        )
    })
}

fn eos(cli: &mut CliSession, iface: &str, count: u32, bpf: &str) -> Result<Vec<u8>> {
    let extra = if bpf.trim().is_empty() {
        String::new()
    } else {
        format!(" {}", bpf.trim())
    };
    let ifc = if iface == "any" { "any" } else { iface };
    cli.send(&format!(
        "bash timeout 18 tcpdump -nn -s 128 -c {count} -i {ifc} -w /mnt/flash/latepcap.pcap{extra}"
    ))?;
    cli.wait(Duration::from_secs(20));
    let mark = cli.len();
    cli.send(
        "bash echo LATEB64_START; bash base64 /mnt/flash/latepcap.pcap; bash echo LATEB64_END",
    )?;
    let out = cli.wait_until(Duration::from_secs(8), has_b64_end_line);
    decode_marked(&cli.since(mark))
        .or_else(|| decode_marked(&out))
        .ok_or_else(|| LateError::Pcap("EOS bash tcpdump did not return a pcap".into()))
}

fn junos(cli: &mut CliSession, iface: &str, count: u32) -> Result<Vec<u8>> {
    let ifc = if iface == "any" { "em0" } else { iface };
    cli.send(&format!(
        "monitor traffic interface {ifc} write-file /var/tmp/latepcap.pcap count {count} no-resolve"
    ))?;
    cli.wait(Duration::from_secs(20));
    try_unix_shell(cli, ifc, count, "").ok_or_else(|| {
        LateError::Pcap(
            "Junos monitor traffic ran but the pcap could not be exported (need start shell)"
                .into(),
        )
    })
}

pub fn start_live(
    device: &Device,
    profile: &AuthProfile,
    secrets: &SecretStore,
    iface: &str,
    bpf: &str,
    outfile: PathBuf,
) -> Result<LiveCapture> {
    pcap::validate_iface(iface)?;
    pcap::validate_bpf(bpf)?;
    match start_unix_stream(device, profile, secrets, iface, bpf, outfile.clone()) {
        Ok(cap) => Ok(cap),
        Err(_unix_err) => start_cli_worker(device, profile, secrets, iface, bpf, outfile),
    }
}

fn path_is_pcap(path: &Path) -> bool {
    let mut magic = [0u8; 4];
    File::open(path)
        .and_then(|mut f| f.read_exact(&mut magic))
        .is_ok()
        && looks_like_pcap(&magic)
}

fn start_unix_stream(
    device: &Device,
    profile: &AuthProfile,
    secrets: &SecretStore,
    iface: &str,
    bpf: &str,
    outfile: PathBuf,
) -> Result<LiveCapture> {
    let sh = pcap::remote_tcpdump_live_sh(iface, bpf)?;
    let mut child = ssh::spawn_exec(device, profile, secrets, &sh)?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| LateError::Pcap("ssh stdout missing".into()))?;
    let stderr = child.stderr.take();
    let err_buf = Arc::new(Mutex::new(String::new()));
    let err2 = err_buf.clone();
    std::thread::spawn(move || {
        if let Some(mut e) = stderr {
            let _ = e.read_to_string(&mut *err2.lock().unwrap_or_else(|p| p.into_inner()));
        }
    });
    let mut out = File::create(&outfile)?;
    let pump = std::thread::spawn(move || {
        let mut r = stdout;
        let _ = std::io::copy(&mut r, &mut out);
    });
    std::thread::sleep(Duration::from_millis(1200));
    if path_is_pcap(&outfile) {
        return Ok(LiveCapture::remote(
            iface.into(),
            outfile,
            Some(child),
            Some(pump),
            None,
            None,
        ));
    }
    let _ = child.kill();
    let _ = child.wait();
    let _ = pump.join();
    let err = err_buf.lock().unwrap_or_else(|p| p.into_inner()).clone();
    Err(LateError::Pcap(if err.trim().is_empty() {
        "remote unix tcpdump did not stream a pcap".into()
    } else {
        err
    }))
}

fn start_cli_worker(
    device: &Device,
    profile: &AuthProfile,
    secrets: &SecretStore,
    iface: &str,
    bpf: &str,
    outfile: PathBuf,
) -> Result<LiveCapture> {
    let (tx, rx) = mpsc::channel::<()>();
    let running = Arc::new(AtomicBool::new(false));
    let fail = Arc::new(Mutex::new(None::<String>));
    let device = device.clone();
    let profile = profile.clone();
    let secrets = secrets.clone();
    let iface_s = iface.to_string();
    let bpf_s = bpf.to_string();
    let outfile2 = outfile.clone();
    let running2 = running.clone();
    let fail2 = fail.clone();
    let worker = std::thread::spawn(move || {
        let res = cli_live_loop(
            &device, &profile, &secrets, &iface_s, &bpf_s, &outfile2, rx, &running2,
        );
        if let Err(e) = &res {
            *fail2.lock().unwrap_or_else(|p| p.into_inner()) = Some(e.to_string());
        }
        res
    });
    for _ in 0..80 {
        if running.load(Ordering::SeqCst) {
            break;
        }
        if worker.is_finished() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if let Some(e) = fail.lock().unwrap_or_else(|p| p.into_inner()).clone() {
        let _ = worker.join();
        return Err(LateError::Pcap(e));
    }
    if worker.is_finished() {
        return match worker.join() {
            Ok(Ok(())) => Ok(LiveCapture::remote(
                iface.into(),
                outfile,
                None,
                None,
                Some(tx),
                None,
            )),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(LateError::Pcap("SSH capture worker panicked".into())),
        };
    }
    Ok(LiveCapture::remote(
        iface.into(),
        outfile,
        None,
        None,
        Some(tx),
        Some(worker),
    ))
}

fn wait_stop(cli: &mut CliSession, rx: &mpsc::Receiver<()>) {
    loop {
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let _ = cli.wait(Duration::from_millis(50));
            }
        }
    }
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    File::create(path)?
        .write_all(bytes)
        .map_err(|e| LateError::Pcap(e.to_string()))?;
    Ok(())
}

fn export_unix_file(cli: &mut CliSession, remote: &str) -> Option<Vec<u8>> {
    let mark = cli.len();
    cli.send(&format!(
        "echo LATEB64_START; base64 {remote} 2>/dev/null; echo LATEB64_END"
    ))
    .ok()?;
    let out = cli.wait_until(Duration::from_secs(8), has_b64_end_line);
    decode_marked(&cli.since(mark)).or_else(|| decode_marked(&out))
}

fn cli_live_loop(
    device: &Device,
    profile: &AuthProfile,
    secrets: &SecretStore,
    iface: &str,
    bpf: &str,
    outfile: &Path,
    rx: mpsc::Receiver<()>,
    running: &AtomicBool,
) -> Result<()> {
    let mut cli = CliSession::open(device, profile, secrets)?;
    let kind = prepare_cli(&mut cli, device.vendor)?;
    let extra = if bpf.trim().is_empty() {
        String::new()
    } else {
        format!(" {}", bpf.trim())
    };
    match kind {
        Platform::AosCx => {
            // 6200/10.11: start-shell + sudo tcpdump works. `diag utilities tcpdump` is not valid here.
            if enter_unix_shell(&mut cli) {
                live_unix_tcpdump(&mut cli, iface, bpf, outfile, &rx, running)
            } else {
                let extra = aos_cx_filter(bpf);
                cli.send("diag utilities tcpdump delete-file latepcap.pcap")?;
                cli.wait(Duration::from_millis(600));
                cli.send(&format!(
                    "diag utilities tcpdump command -w latepcap.pcap{extra}"
                ))?;
                running.store(true, Ordering::SeqCst);
                wait_stop(&mut cli, &rx);
                cli.interrupt()?;
                let _ = cli.wait_until(Duration::from_secs(8), |s| {
                    aos_cx_ended(s) || ssh::looks_like_cli_prompt(s)
                });
                let bytes = aos_cx_export(&mut cli)?;
                write_bytes(outfile, &bytes)
            }
        }
        Platform::Nxos => {
            let ifc = if iface == "any" { "inband" } else { iface };
            cli.send(&format!(
                "ethanalyzer local interface {ifc} write bootflash:latepcap.pcap"
            ))?;
            running.store(true, Ordering::SeqCst);
            wait_stop(&mut cli, &rx);
            cli.interrupt()?;
            cli.wait(Duration::from_secs(2));
            if let Some(bytes) = try_unix_shell(&mut cli, ifc, 50, "") {
                write_bytes(outfile, &bytes)
            } else {
                Err(LateError::Pcap(
                    "NX-OS capture stopped but pcap export failed".into(),
                ))
            }
        }
        Platform::Eos => {
            let ifc = if iface == "any" { "any" } else { iface };
            cli.send(&format!(
                "bash tcpdump -nn -s 128 -i {ifc} -w /mnt/flash/latepcap.pcap{extra}"
            ))?;
            running.store(true, Ordering::SeqCst);
            wait_stop(&mut cli, &rx);
            cli.interrupt()?;
            cli.wait(Duration::from_secs(2));
            let bytes =
                export_unix_file(&mut cli, "/mnt/flash/latepcap.pcap").ok_or_else(|| {
                    LateError::Pcap("EOS capture stopped but pcap export failed".into())
                })?;
            write_bytes(outfile, &bytes)
        }
        Platform::Junos => {
            let ifc = if iface == "any" { "em0" } else { iface };
            cli.send(&format!(
                "monitor traffic interface {ifc} write-file /var/tmp/latepcap.pcap no-resolve"
            ))?;
            running.store(true, Ordering::SeqCst);
            wait_stop(&mut cli, &rx);
            cli.interrupt()?;
            cli.wait(Duration::from_secs(2));
            cli.send("start shell")?;
            cli.wait(Duration::from_secs(2));
            let bytes = export_unix_file(&mut cli, "/var/tmp/latepcap.pcap").ok_or_else(|| {
                LateError::Pcap("Junos capture stopped but pcap export failed".into())
            })?;
            write_bytes(outfile, &bytes)
        }
        Platform::Unix | Platform::Unknown => {
            cli.send(&format!(
                "tcpdump -nn -U -s 128 -i {iface} -w /tmp/late-live.pcap{extra}"
            ))?;
            running.store(true, Ordering::SeqCst);
            wait_stop(&mut cli, &rx);
            cli.interrupt()?;
            cli.wait(Duration::from_secs(2));
            let bytes = export_unix_file(&mut cli, "/tmp/late-live.pcap").ok_or_else(|| {
                LateError::Pcap("remote tcpdump stopped but pcap export failed".into())
            })?;
            write_bytes(outfile, &bytes)
        }
    }
}

fn has_b64_end_line(text: &str) -> bool {
    text.lines().any(|l| l.trim() == "LATEB64_END")
        && text.lines().any(|l| l.trim() == "LATEB64_START")
}

fn decode_marked(text: &str) -> Option<Vec<u8>> {
    let mut collecting = false;
    let mut body = String::new();
    for line in text.lines() {
        let t = line.trim();
        if !collecting {
            if t == "LATEB64_START" {
                collecting = true;
            }
            continue;
        }
        if t == "LATEB64_END" {
            let bytes = STANDARD.decode(body).ok()?;
            return looks_like_pcap(&bytes).then_some(bytes);
        }
        body.push_str(t);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_aruba_banner() {
        let t = "Hewlett Packard Enterprise Development LP  aruba  AOS-CX";
        assert_eq!(classify(t, Vendor::Generic), Platform::AosCx);
    }

    #[test]
    fn fingerprint_hpe_motd() {
        let t = "Copyright (C) 2024 Hewlett Packard Enterprise Development LP.\nRestricted Rights Legend";
        assert_eq!(classify(t, Vendor::Generic), Platform::AosCx);
    }

    #[test]
    fn fingerprint_aruba_motd() {
        assert_eq!(classify("aruba", Vendor::Generic), Platform::AosCx);
    }

    #[test]
    fn fingerprint_nxos() {
        assert_eq!(
            classify("Cisco NX-OS(tm) nexus", Vendor::Generic),
            Platform::Nxos
        );
    }

    #[test]
    fn hint_used_when_banner_empty() {
        assert_eq!(classify("", Vendor::Junos), Platform::Junos);
    }

    #[test]
    fn license_wall_detected_without_being_in_error() {
        assert!(looks_like_license_wall(
            "END USER LICENSE AGREEMENT\nDo you accept (yes/no)?"
        ));
        let err = AOS_CX_NO_SHELL.to_lowercase();
        assert!(!err.contains("license"));
        assert!(!err.contains("eula"));
        assert!(!err.contains("restricted rights"));
    }

    #[test]
    fn decode_pcap_magic_as_b64() {
        // little-endian pcap magic d4 c3 b2 a1
        let raw = [0xd4u8, 0xc3, 0xb2, 0xa1, 0, 0, 0, 0];
        let b64 = STANDARD.encode(raw);
        let text = format!("junk\nLATEB64_START\n{b64}\nLATEB64_END\n#");
        let got = decode_marked(&text).unwrap();
        assert!(looks_like_pcap(&got));
        let echoed = format!(
            "echo LATEB64_START; base64 /tmp/x; echo LATEB64_END\nLATEB64_START\n{b64}\nLATEB64_END\n$ "
        );
        assert!(decode_marked(&echoed).is_some());
    }
}
