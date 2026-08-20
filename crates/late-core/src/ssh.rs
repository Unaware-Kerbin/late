use crate::config::LatePaths;
use crate::error::{LateError, Result};
use crate::known_hosts::{
    host_port_key, pick_keyscan_line, sanitized_host_key_algo, HostKeyCheck, KnownHosts,
};
use crate::secrets::SecretStore;
use crate::types::{AuthProfile, Device};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::fs;
use std::io::{Read, Write};
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};
use tokio::sync::{broadcast, mpsc};
use zeroize::Zeroize;

pub struct SshIo {
    pub tx: mpsc::Sender<Vec<u8>>,
    pub rx: broadcast::Receiver<Vec<u8>>,
    pub resize: mpsc::Sender<(u32, u32)>,
    pub close: mpsc::Sender<()>,
}

pub struct SshConnectOpts {
    pub accept_unknown_host: bool,
    pub replace_host_key: bool,
}

pub fn probe_fingerprint(host: &str, port: u16) -> Result<String> {
    Ok(probe_keyscan(host, port)?.2)
}

fn probe_keyscan(host: &str, port: u16) -> Result<(String, String, String)> {
    let out = Command::new("ssh-keyscan")
        .args([
            "-T",
            "5",
            "-t",
            "ed25519,ecdsa,rsa",
            "-p",
            &port.to_string(),
            host,
        ])
        .output()
        .map_err(|e| LateError::Ssh(format!("ssh-keyscan: {e}")))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = pick_keyscan_line(&text)?;
    let (algo, fp) = KnownHosts::fingerprint_keyscan_line(&line)?;
    Ok((line, algo, fp))
}

pub(crate) fn confined_identity(profile: &AuthProfile) -> Result<Option<PathBuf>> {
    match profile
        .key_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(key) => Ok(Some(crate::confine::confine_identity_path(key)?)),
        None => Ok(None),
    }
}

fn write_openssh_known_hosts(
    paths: &LatePaths,
    host: &str,
    port: u16,
    line: &str,
) -> Result<PathBuf> {
    let dir = paths.data.join("ssh-known");
    crate::fsutil::mkdir_private(&dir)?;
    let safe: String = host
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'))
        .collect();
    let path = dir.join(format!("{safe}_{port}.known"));
    crate::fsutil::write_private(&path, format!("{line}\n"))?;
    Ok(path)
}

/// OpenSSH will only connect if this host was already pinned in Late's TOML store.
pub(crate) fn pinned_known_hosts_file(host: &str, port: u16) -> Result<(PathBuf, String)> {
    let paths = LatePaths::discover();
    let known = KnownHosts::load(&paths)?;
    let (line, algo, fp) = probe_keyscan(host, port)?;
    let hk = host_port_key(host, port);
    match known.check(&hk, &fp) {
        HostKeyCheck::Match => {}
        HostKeyCheck::Unknown => {
            return Err(LateError::Ssh(format!(
                "host {hk} is not pinned; open an SSH session first"
            )));
        }
        HostKeyCheck::Mismatch { pinned, presented } => {
            return Err(LateError::HostKeyMismatch {
                host: hk,
                pinned,
                presented,
            });
        }
    }
    let path = write_openssh_known_hosts(&paths, host, port, &line)?;
    Ok((path, algo))
}

pub fn open_ssh(
    device: &Device,
    profile: &AuthProfile,
    secrets: &SecretStore,
    known: &mut KnownHosts,
    paths: &LatePaths,
    opts: SshConnectOpts,
    cols: u32,
    rows: u32,
) -> Result<(SshIo, String)> {
    let host = device
        .host
        .clone()
        .ok_or_else(|| LateError::Ssh("device has no host".into()))?;
    let port = device.port.unwrap_or(22);
    let hk = host_port_key(&host, port);
    let (keyscan_line, algo, presented) = probe_keyscan(&host, port)?;
    match known.check(&hk, &presented) {
        HostKeyCheck::Match => {}
        HostKeyCheck::Unknown => {
            if !opts.accept_unknown_host {
                return Err(LateError::HostKeyUntrusted {
                    host: hk,
                    presented,
                });
            }
            known.pin(&hk, &presented);
            known.save(paths)?;
        }
        HostKeyCheck::Mismatch { pinned, presented } => {
            if !opts.replace_host_key {
                return Err(LateError::HostKeyMismatch {
                    host: hk,
                    pinned,
                    presented,
                });
            }
            known.pin(&hk, &presented);
            known.save(paths)?;
        }
    }
    let kh = write_openssh_known_hosts(paths, &host, port, &keyscan_line)?;

    let mut password = if profile.has_password {
        secrets.get(&profile.id)?
    } else {
        None
    };

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: rows as u16,
            cols: cols as u16,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| LateError::Ssh(e.to_string()))?;

    let mut cmd = CommandBuilder::new("ssh");
    cmd.arg("-tt");
    apply_strict_host_opts_pty(&mut cmd, &kh, &algo);
    cmd.arg("-p");
    cmd.arg(port.to_string());
    if let Some(key) = confined_identity(profile)? {
        cmd.arg("-i");
        cmd.arg(key.as_os_str());
    }
    if !profile.use_agent {
        cmd.arg("-o");
        cmd.arg("IdentitiesOnly=yes");
    }
    cmd.arg(format!("{}@{}", profile.username, host));
    if profile.has_password {
        cmd.arg("-o");
        cmd.arg("PreferredAuthentications=keyboard-interactive,password");
        cmd.arg("-o");
        cmd.arg("NumberOfPasswordPrompts=1");
    }

    let _askpass = if let Some(ref pw) = password {
        let (wrapped, guard) = wrap_pty_password(cmd, pw)?;
        cmd = wrapped;
        guard
    } else {
        AskpassCleanup::none()
    };
    if let Some(mut pw) = password.take() {
        pw.zeroize();
    }

    let mut child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| LateError::Ssh(e.to_string()))?;

    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| LateError::Ssh(e.to_string()))?;
    let mut writer = pair
        .master
        .take_writer()
        .map_err(|e| LateError::Ssh(e.to_string()))?;
    let master = pair.master;

    let (in_tx, mut in_rx) = mpsc::channel::<Vec<u8>>(256);
    let (out_tx, _) = broadcast::channel::<Vec<u8>>(256);
    let (resize_tx, mut resize_rx) = mpsc::channel::<(u32, u32)>(8);
    let (close_tx, mut close_rx) = mpsc::channel::<()>(1);
    let out_tx2 = out_tx.clone();

    std::thread::spawn(move || loop {
        if close_rx.try_recv().is_ok() {
            let _ = child.kill();
            break;
        }
        while let Ok(b) = in_rx.try_recv() {
            let _ = writer.write_all(&b);
            let _ = writer.flush();
        }
        if let Ok((c, r)) = resize_rx.try_recv() {
            let _ = master.resize(PtySize {
                rows: r as u16,
                cols: c as u16,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
        std::thread::sleep(std::time::Duration::from_millis(8));
    });

    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let _ = out_tx2.send(buf[..n].to_vec());
                }
                Err(_) => break,
            }
        }
    });

    Ok((
        SshIo {
            tx: in_tx,
            rx: out_tx.subscribe(),
            resize: resize_tx,
            close: close_tx,
        },
        presented,
    ))
}

pub fn exec_bytes(
    device: &Device,
    profile: &AuthProfile,
    secrets: &SecretStore,
    command: &str,
    timeout: std::time::Duration,
    max_stdout: usize,
) -> Result<(Vec<u8>, String)> {
    let (out, err, ok) = exec_bytes_result(device, profile, secrets, command, timeout, max_stdout)?;
    if !ok && !crate::pcap::looks_like_pcap(&out) {
        return Err(LateError::Ssh(crate::pcap::explain_remote_capture_failure(
            &out, &err,
        )));
    }
    Ok((out, err))
}

pub fn exec_bytes_result(
    device: &Device,
    profile: &AuthProfile,
    secrets: &SecretStore,
    command: &str,
    timeout: std::time::Duration,
    max_stdout: usize,
) -> Result<(Vec<u8>, String, bool)> {
    let host = device
        .host
        .clone()
        .ok_or_else(|| LateError::Ssh("device has no host".into()))?;
    let port = device.port.unwrap_or(22);
    let mut password = if profile.has_password {
        secrets.get(&profile.id)?
    } else {
        None
    };
    let mut cmd = ssh_command_with_secret("ssh", password.as_deref())?;
    cmd.arg("-T");
    if profile.has_password {
        cmd.arg("-o")
            .arg("PreferredAuthentications=keyboard-interactive,password");
        cmd.arg("-o").arg("NumberOfPasswordPrompts=1");
    } else {
        cmd.arg("-o").arg("BatchMode=yes");
    }
    apply_strict_host_opts_std(&mut cmd, &host, port)?;
    cmd.arg("-o").arg("LogLevel=ERROR");
    cmd.arg("-o")
        .arg("ConnectTimeout=8")
        .arg("-p")
        .arg(port.to_string());
    if let Some(key) = confined_identity(profile)? {
        cmd.arg("-i").arg(key);
    }
    if !profile.use_agent {
        cmd.arg("-o").arg("IdentitiesOnly=yes");
    }
    cmd.arg(format!("{}@{}", profile.username, host));
    cmd.arg(command);
    if let Some(mut pw) = password.take() {
        pw.zeroize();
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| LateError::Ssh(format!("ssh exec: {e}")))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let out_h = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(r) = stdout {
            use std::io::Read;
            let _ = r.take(max_stdout as u64 + 1).read_to_end(&mut buf);
        }
        buf
    });
    let err_h = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(mut r) = stderr {
            use std::io::Read;
            let _ = r.read_to_string(&mut s);
        }
        s
    });
    let start = std::time::Instant::now();
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| LateError::Ssh(e.to_string()))?
        {
            let out = out_h.join().unwrap_or_default();
            let err = err_h.join().unwrap_or_default();
            if out.len() > max_stdout {
                return Err(LateError::Ssh("remote capture exceeded size limit".into()));
            }
            return Ok((out, err, status.success()));
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            let out = out_h.join().unwrap_or_default();
            let err = err_h.join().unwrap_or_default();
            if crate::pcap::looks_like_pcap(&out) {
                return Ok((out, err, true));
            }
            return Err(LateError::Ssh(format!(
                "remote capture timed out: {}",
                crate::pcap::explain_remote_capture_failure(&out, &err)
            )));
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
}

pub fn spawn_exec(
    device: &Device,
    profile: &AuthProfile,
    secrets: &SecretStore,
    command: &str,
) -> Result<std::process::Child> {
    let host = device
        .host
        .clone()
        .ok_or_else(|| LateError::Ssh("device has no host".into()))?;
    let port = device.port.unwrap_or(22);
    let mut password = if profile.has_password {
        secrets.get(&profile.id)?
    } else {
        None
    };
    let mut cmd = ssh_command_with_secret("ssh", password.as_deref())?;
    cmd.arg("-T");
    if profile.has_password {
        cmd.arg("-o")
            .arg("PreferredAuthentications=keyboard-interactive,password");
        cmd.arg("-o").arg("NumberOfPasswordPrompts=1");
    } else {
        cmd.arg("-o").arg("BatchMode=yes");
    }
    apply_strict_host_opts_std(&mut cmd, &host, port)?;
    cmd.arg("-o").arg("LogLevel=ERROR");
    cmd.arg("-o")
        .arg("ConnectTimeout=8")
        .arg("-p")
        .arg(port.to_string());
    if let Some(key) = confined_identity(profile)? {
        cmd.arg("-i").arg(key);
    }
    if !profile.use_agent {
        cmd.arg("-o").arg("IdentitiesOnly=yes");
    }
    cmd.arg(format!("{}@{}", profile.username, host));
    cmd.arg(command);
    if let Some(mut pw) = password.take() {
        pw.zeroize();
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    cmd.spawn()
        .map_err(|e| LateError::Ssh(format!("ssh exec: {e}")))
}

/// Interactive SSH CLI (network OS or unix). Used when `ssh -T` cannot run a unix one-liner.
pub struct CliSession {
    child: Box<dyn portable_pty::Child + Send>,
    writer: Box<dyn Write + Send>,
    rx: std::sync::mpsc::Receiver<Vec<u8>>,
    acc: Vec<u8>,
}

impl Drop for CliSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

impl CliSession {
    pub fn open(device: &Device, profile: &AuthProfile, secrets: &SecretStore) -> Result<Self> {
        let host = device
            .host
            .clone()
            .ok_or_else(|| LateError::Ssh("device has no host".into()))?;
        let port = device.port.unwrap_or(22);
        let mut password = if profile.has_password {
            secrets.get(&profile.id)?
        } else {
            None
        };
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: 40,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| LateError::Ssh(e.to_string()))?;
        let (kh, algo) = pinned_known_hosts_file(&host, port)?;
        let mut cmd = CommandBuilder::new("ssh");
        cmd.arg("-tt");
        apply_strict_host_opts_pty(&mut cmd, &kh, &algo);
        cmd.arg("-p");
        cmd.arg(port.to_string());
        if let Some(key) = confined_identity(profile)? {
            cmd.arg("-i");
            cmd.arg(key.as_os_str());
        }
        if !profile.use_agent {
            cmd.arg("-o");
            cmd.arg("IdentitiesOnly=yes");
        }
        cmd.arg(format!("{}@{}", profile.username, host));
        if profile.has_password {
            cmd.arg("-o");
            cmd.arg("PreferredAuthentications=keyboard-interactive,password");
            cmd.arg("-o");
            cmd.arg("NumberOfPasswordPrompts=1");
        }
        let _askpass = if let Some(ref pw) = password {
            let (wrapped, guard) = wrap_pty_password(cmd, pw)?;
            cmd = wrapped;
            guard
        } else {
            AskpassCleanup::none()
        };
        if let Some(mut pw) = password.take() {
            pw.zeroize();
        }
        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| LateError::Ssh(e.to_string()))?;
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| LateError::Ssh(e.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| LateError::Ssh(e.to_string()))?;
        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            child,
            writer,
            rx,
            acc: Vec::new(),
        })
    }

    pub fn len(&self) -> usize {
        self.acc.len()
    }

    pub fn since(&self, mark: usize) -> String {
        String::from_utf8_lossy(&self.acc.get(mark..).unwrap_or(&[])).into_owned()
    }

    pub fn send(&mut self, line: &str) -> Result<()> {
        self.writer
            .write_all(line.as_bytes())
            .and_then(|_| self.writer.write_all(b"\r\n"))
            .and_then(|_| self.writer.flush())
            .map_err(|e| LateError::Ssh(e.to_string()))
    }

    pub fn interrupt(&mut self) -> Result<()> {
        self.writer
            .write_all(&[0x03])
            .and_then(|_| self.writer.flush())
            .map_err(|e| LateError::Ssh(e.to_string()))
    }

    pub fn wait(&mut self, timeout: std::time::Duration) -> String {
        let mark = self.acc.len();
        self.pump(timeout);
        self.since(mark)
    }

    pub fn wait_until(
        &mut self,
        timeout: std::time::Duration,
        pred: impl Fn(&str) -> bool,
    ) -> String {
        let mark = self.acc.len();
        let start = std::time::Instant::now();
        while start.elapsed() < timeout {
            self.pump(std::time::Duration::from_millis(120));
            if pred(&self.since(mark)) {
                break;
            }
        }
        self.since(mark)
    }

    pub fn wait_prompt(&mut self, timeout: std::time::Duration) -> Result<()> {
        let start = std::time::Instant::now();
        while start.elapsed() < timeout {
            self.pump(std::time::Duration::from_millis(120));
            let t = String::from_utf8_lossy(&self.acc);
            if looks_like_cli_prompt(&t) {
                return Ok(());
            }
        }
        Err(LateError::Ssh(
            "no CLI prompt from SSH target (login banner only?)".into(),
        ))
    }

    fn pump(&mut self, slice: std::time::Duration) {
        let start = std::time::Instant::now();
        while start.elapsed() < slice {
            match self.rx.recv_timeout(std::time::Duration::from_millis(40)) {
                Ok(chunk) => {
                    let text = String::from_utf8_lossy(&chunk);
                    self.acc.extend_from_slice(&chunk);
                    let tail =
                        String::from_utf8_lossy(&self.acc[self.acc.len().saturating_sub(240)..]);
                    // AOS-CX uses `-- MORE --`; NX-OS/IOS use `--More--`.
                    if looks_like_more_prompt(&text) || looks_like_more_prompt(&tail) {
                        let _ = self.writer.write_all(b" ");
                        let _ = self.writer.flush();
                    }
                    if self.acc.len() > 2 * 1024 * 1024 {
                        let drop = self.acc.len() - 1024 * 1024;
                        self.acc.drain(..drop);
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    }
}

pub(crate) fn looks_like_cli_prompt(text: &str) -> bool {
    if looks_like_more_prompt(text) {
        return false;
    }
    let line = text
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim_end();
    line.ends_with('#')
        || line.ends_with('>')
        || line.ends_with('$')
        || line.ends_with("% ")
        || line.ends_with('%')
}

pub(crate) fn looks_like_more_prompt(text: &str) -> bool {
    let l = text.to_ascii_lowercase();
    l.contains("--more--") || l.contains("-- more --")
}

#[cfg(test)]
mod prompt_tests {
    use super::*;

    #[test]
    fn aos_cx_more_prompt() {
        assert!(looks_like_more_prompt(
            "-- MORE --, next page: Space, next line: Enter, quit: q"
        ));
        assert!(looks_like_more_prompt("--More--"));
        assert!(!looks_like_cli_prompt("-- MORE --"));
        assert!(looks_like_cli_prompt("6200F#"));
    }
}

#[cfg(test)]
mod askpass_tests {
    use super::*;

    #[test]
    fn askpass_guard_does_not_delete_before_ssh_can_exec() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = LatePaths {
            config: tmp.path().to_path_buf(),
            data: tmp.path().to_path_buf(),
        };
        let (script, guard) = write_askpass_in("secret", &paths).unwrap();
        let pwfile = script.with_extension("pw");
        assert!(script.is_file());
        assert!(pwfile.is_file());
        drop(guard);
        assert!(
            script.is_file(),
            "ASKPASS script must still exist when connect() returns so OpenSSH can exec it"
        );
        assert!(pwfile.is_file());
        remove_askpass_files(&[script.clone(), pwfile.clone()]);
        assert!(!script.exists());
        assert!(!pwfile.exists());
    }

    #[test]
    fn askpass_sweep_removes_stale_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("askpass");
        crate::fsutil::mkdir_private(&dir).unwrap();
        let leftover = dir.join("old.pw");
        crate::fsutil::write_private(&leftover, b"x").unwrap();
        sweep_stale_askpass(&dir, Duration::ZERO);
        assert!(!leftover.exists());
    }
}

/// Password file + helper live until OpenSSH execs the script (which self-deletes)
/// or until a delayed reap. Immediate Drop after spawn races pre-auth banners
/// (e.g. Aruba) and yields `ssh_askpass: No such file or directory`.
const ASKPASS_REAP: Duration = Duration::from_secs(120);

struct AskpassCleanup {
    files: Vec<PathBuf>,
}

impl AskpassCleanup {
    fn none() -> Self {
        Self { files: Vec::new() }
    }
}

impl Drop for AskpassCleanup {
    fn drop(&mut self) {
        schedule_askpass_reap(std::mem::take(&mut self.files), ASKPASS_REAP);
    }
}

fn remove_askpass_files(files: &[PathBuf]) {
    for path in files {
        let _ = fs::remove_file(path);
    }
}

fn schedule_askpass_reap(files: Vec<PathBuf>, delay: Duration) {
    if files.is_empty() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("late-askpass-reap".into())
        .spawn(move || {
            std::thread::sleep(delay);
            remove_askpass_files(&files);
        });
}

fn sweep_stale_askpass(dir: &Path, max_age: Duration) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    for ent in rd.flatten() {
        let path = ent.path();
        let ext = path.extension().and_then(|e| e.to_str());
        if ext != Some("pw") && ext != Some("sh") {
            continue;
        }
        let Ok(meta) = ent.metadata() else { continue };
        let Ok(modified) = meta.modified() else { continue };
        if now.duration_since(modified).unwrap_or(Duration::ZERO) >= max_age {
            let _ = fs::remove_file(path);
        }
    }
}

/// `Command` plus ASKPASS files reaped after a delay (or by the helper itself).
pub(crate) struct SecretCommand {
    cmd: Command,
    _askpass: AskpassCleanup,
}

impl Deref for SecretCommand {
    type Target = Command;
    fn deref(&self) -> &Command {
        &self.cmd
    }
}

impl DerefMut for SecretCommand {
    fn deref_mut(&mut self) -> &mut Command {
        &mut self.cmd
    }
}

fn write_askpass(password: &str) -> Result<(PathBuf, AskpassCleanup)> {
    write_askpass_in(password, &LatePaths::discover())
}

fn write_askpass_in(password: &str, paths: &LatePaths) -> Result<(PathBuf, AskpassCleanup)> {
    let id = uuid::Uuid::new_v4();
    let dir = paths.data.join("askpass");
    crate::fsutil::mkdir_private(&dir)?;
    sweep_stale_askpass(&dir, ASKPASS_REAP);
    let pwfile = dir.join(format!("{id}.pw"));
    let script = dir.join(format!("{id}.sh"));
    let guard = AskpassCleanup {
        files: vec![pwfile.clone(), script.clone()],
    };
    crate::fsutil::write_private(&pwfile, password.as_bytes())?;
    let body = format!(
        "#!/bin/sh\ncat \"{pw}\"\n: > \"{pw}\"\nrm -f \"{pw}\" \"$0\"\n",
        pw = pwfile.display()
    );
    crate::fsutil::write_private(&script, body.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script)?.permissions();
        perms.set_mode(0o700);
        fs::set_permissions(&script, perms)?;
    }
    Ok((script, guard))
}

fn wrap_pty_password(
    existing: CommandBuilder,
    pw: &str,
) -> Result<(CommandBuilder, AskpassCleanup)> {
    let (script, guard) = write_askpass(pw)?;
    let mut c = existing;
    c.env("SSH_ASKPASS", script.to_string_lossy().as_ref());
    c.env("SSH_ASKPASS_REQUIRE", "force");
    if std::env::var_os("DISPLAY").is_none() {
        c.env("DISPLAY", ":0");
    }
    Ok((c, guard))
}

pub(crate) fn ssh_command_with_secret(
    program: &str,
    password: Option<&str>,
) -> Result<SecretCommand> {
    let mut cmd = Command::new(program);
    let askpass = if let Some(pw) = password {
        let (script, guard) = write_askpass(pw)?;
        cmd.env("SSH_ASKPASS", script.as_os_str());
        cmd.env("SSH_ASKPASS_REQUIRE", "force");
        if std::env::var_os("DISPLAY").is_none() {
            cmd.env("DISPLAY", ":0");
        }
        guard
    } else {
        AskpassCleanup::none()
    };
    Ok(SecretCommand {
        cmd,
        _askpass: askpass,
    })
}

pub(crate) fn apply_strict_host_opts_std(cmd: &mut Command, host: &str, port: u16) -> Result<()> {
    let (kh, algo) = pinned_known_hosts_file(host, port)?;
    apply_strict_std(cmd, &kh, &algo);
    Ok(())
}

fn apply_strict_std(cmd: &mut Command, kh: &Path, algo: &str) {
    cmd.arg("-o").arg("StrictHostKeyChecking=yes");
    cmd.arg("-o")
        .arg(format!("UserKnownHostsFile={}", kh.display()));
    cmd.arg("-o").arg("GlobalKnownHostsFile=/dev/null");
    cmd.arg("-o").arg("UpdateHostKeys=no");
    if let Some(a) = sanitized_host_key_algo(algo) {
        cmd.arg("-o").arg(format!("HostKeyAlgorithms={a}"));
    }
}

fn apply_strict_host_opts_pty(cmd: &mut CommandBuilder, kh: &Path, algo: &str) {
    cmd.arg("-o");
    cmd.arg("StrictHostKeyChecking=yes");
    cmd.arg("-o");
    cmd.arg(format!("UserKnownHostsFile={}", kh.display()));
    cmd.arg("-o");
    cmd.arg("GlobalKnownHostsFile=/dev/null");
    cmd.arg("-o");
    cmd.arg("UpdateHostKeys=no");
    if let Some(a) = sanitized_host_key_algo(algo) {
        cmd.arg("-o");
        cmd.arg(format!("HostKeyAlgorithms={a}"));
    }
}
