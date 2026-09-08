use crate::error::{LateError, Result};
use parking_lot::Mutex;
use std::collections::HashSet;
use std::io::{Read, Write};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc};

/// Paths this process currently holds (exclusive flock / TIOCEXCL). Cleared only after
/// the reader thread drops the OS handles — not merely when the UI session is removed.
fn held_ports() -> &'static Mutex<HashSet<String>> {
    static HELD: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    HELD.get_or_init(|| Mutex::new(HashSet::new()))
}

pub struct SerialIo {
    pub tx: mpsc::Sender<Vec<u8>>,
    pub break_tx: mpsc::Sender<()>,
    pub rx: broadcast::Receiver<Vec<u8>>,
    pub close: mpsc::Sender<()>,
    /// Join to wait until the serial FD/flock is fully released.
    pub join: Option<std::thread::JoinHandle<()>>,
}

impl SerialIo {
    /// Signal stop and block until the worker has dropped the port handles.
    pub fn release(mut self) {
        let _ = self.close.try_send(());
        // Disconnect wakes the worker even if try_send raced.
        drop(self.close);
        drop(self.tx);
        drop(self.break_tx);
        if let Some(h) = self.join.take() {
            let _ = h.join();
        }
    }
}

pub fn list_serial_ports() -> Vec<String> {
    let mut ports: Vec<String> = serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.port_name)
        .collect();
    if let Ok(dir) = std::fs::read_dir("/dev") {
        for ent in dir.flatten() {
            let name = ent.file_name();
            let n = name.to_string_lossy();
            if n.starts_with("ttyUSB") || n.starts_with("ttyACM") || n.starts_with("ttyS") {
                let path = format!("/dev/{n}");
                if !ports.iter().any(|p| p == &path) {
                    ports.push(path);
                }
            }
        }
    }
    ports.sort();
    ports.dedup();
    ports
}

fn normalize_path(path: &str) -> String {
    std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string())
}

fn serial_err(path: &str, e: impl std::fmt::Display) -> LateError {
    let msg = e.to_string();
    let lower = msg.to_ascii_lowercase();
    if lower.contains("permission denied") {
        LateError::Serial(format!(
            "{path}: permission denied. This port is owned by the dialout group. Run:  sudo usermod -aG dialout $USER   then log out and back in (or reboot). Until then:  sudo chmod 666 {path}  is a temporary workaround."
        ))
    } else if lower.contains("exclusive lock")
        || lower.contains("resource busy")
        || lower.contains("device or resource busy")
        || lower.contains("busy")
        || lower.contains("in use")
    {
        LateError::Serial(format!(
            "{path}: serial port is busy on your computer — another process holds it (not a stale Late handle). Close that program or unplug/replug the adapter, then try again."
        ))
    } else {
        LateError::Serial(format!("{path}: {msg}"))
    }
}

fn open_port(path: &str, baud: u32) -> Result<Box<dyn serialport::SerialPort>> {
    serialport::new(path, baud)
        .timeout(Duration::from_millis(50))
        .open()
        .map_err(|e| serial_err(path, e))
}

fn notice(out: &broadcast::Sender<Vec<u8>>, msg: &str) {
    let _ = out.send(format!("\r\n[late] {msg}\r\n").into_bytes());
}

fn pulse_break(port: &dyn serialport::SerialPort) {
    let _ = port.set_break();
    std::thread::sleep(Duration::from_millis(350));
    let _ = port.clear_break();
}

fn close_requested(close_rx: &mut mpsc::Receiver<()>) -> bool {
    match close_rx.try_recv() {
        Ok(()) => true,
        Err(mpsc::error::TryRecvError::Disconnected) => true,
        Err(mpsc::error::TryRecvError::Empty) => false,
    }
}

/// Returns true if the user closed the session; false if the port dropped.
fn run_port(
    mut writer: Box<dyn serialport::SerialPort>,
    mut reader: Box<dyn serialport::SerialPort>,
    in_rx: &mut mpsc::Receiver<Vec<u8>>,
    brk_rx: &mut mpsc::Receiver<()>,
    close_rx: &mut mpsc::Receiver<()>,
    out_tx: &broadcast::Sender<Vec<u8>>,
) -> bool {
    let mut buf = [0u8; 4096];
    loop {
        if close_requested(close_rx) {
            // Drop handles before returning so flock/TIOCEXCL clear promptly.
            drop(writer);
            drop(reader);
            return true;
        }
        while brk_rx.try_recv().is_ok() {
            if close_requested(close_rx) {
                drop(writer);
                drop(reader);
                return true;
            }
            pulse_break(writer.as_ref());
        }
        while let Ok(bytes) = in_rx.try_recv() {
            if writer.write_all(&bytes).is_err() || writer.flush().is_err() {
                return false;
            }
        }
        match reader.read(&mut buf) {
            Ok(0) => return false,
            Ok(n) => {
                let _ = out_tx.send(buf[..n].to_vec());
            }
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return false,
        }
    }
}

pub fn open_serial(path: &str, baud: u32) -> Result<SerialIo> {
    let key = normalize_path(path);
    {
        let mut held = held_ports().lock();
        if held.contains(&key) {
            return Err(LateError::Serial(format!(
                "{path}: already open in Late. Close that serial session first (the port is still held on your computer until Late releases it)."
            )));
        }
        // Reserve before open so two concurrent opens of the same path fail closed.
        held.insert(key.clone());
    }

    let writer = match open_port(path, baud) {
        Ok(w) => w,
        Err(e) => {
            held_ports().lock().remove(&key);
            return Err(e);
        }
    };
    let reader = match writer.try_clone() {
        Ok(r) => r,
        Err(e) => {
            drop(writer);
            held_ports().lock().remove(&key);
            return Err(LateError::Serial(e.to_string()));
        }
    };

    let (in_tx, mut in_rx) = mpsc::channel::<Vec<u8>>(64);
    let (brk_tx, mut brk_rx) = mpsc::channel::<()>(8);
    let (out_tx, _) = broadcast::channel::<Vec<u8>>(64);
    let (close_tx, mut close_rx) = mpsc::channel::<()>(1);
    let out_tx2 = out_tx.clone();
    let path_owned = path.to_string();
    let held_key = Arc::new(key);

    let join = std::thread::spawn({
        let held_key = Arc::clone(&held_key);
        move || {
            let release_held = || {
                held_ports().lock().remove(held_key.as_ref());
            };
            let mut writer = Some(writer);
            let mut reader = Some(reader);
            let mut last_wait_notice = Instant::now() - Duration::from_secs(10);
            loop {
                if close_requested(&mut close_rx) {
                    break;
                }
                let (w, r) = match (writer.take(), reader.take()) {
                    (Some(w), Some(r)) => (w, r),
                    _ => match open_port(&path_owned, baud) {
                        Ok(w) => match w.try_clone() {
                            Ok(r) => {
                                notice(&out_tx2, &format!("serial reconnected ({path_owned})"));
                                (w, r)
                            }
                            Err(e) => {
                                if last_wait_notice.elapsed() >= Duration::from_secs(4) {
                                    notice(&out_tx2, &format!("waiting for {path_owned}: {e}"));
                                    last_wait_notice = Instant::now();
                                }
                                std::thread::sleep(Duration::from_millis(200));
                                continue;
                            }
                        },
                        Err(e) => {
                            if close_requested(&mut close_rx) {
                                break;
                            }
                            if last_wait_notice.elapsed() >= Duration::from_secs(4) {
                                notice(&out_tx2, &format!("waiting for {path_owned}: {e}"));
                                last_wait_notice = Instant::now();
                            }
                            std::thread::sleep(Duration::from_millis(200));
                            continue;
                        }
                    },
                };
                let user_closed =
                    run_port(w, r, &mut in_rx, &mut brk_rx, &mut close_rx, &out_tx2);
                if user_closed {
                    break;
                }
                notice(
                    &out_tx2,
                    &format!("serial port lost — reconnecting {path_owned}…"),
                );
            }
            // Ensure any retained handles are dropped before clearing the held set.
            drop(writer);
            drop(reader);
            release_held();
        }
    });

    Ok(SerialIo {
        tx: in_tx,
        break_tx: brk_tx,
        rx: out_tx.subscribe(),
        close: close_tx,
        join: Some(join),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::time::Duration;

    /// Keep a PTY slave path alive (master held by child) for serialport open/close tests.
    struct PtyPair {
        path: String,
        child: std::process::Child,
    }

    impl PtyPair {
        fn spawn() -> Option<Self> {
            let mut child = Command::new("python3")
                .args([
                    "-c",
                    r#"
import os, pty, sys, time
master, slave = pty.openpty()
sys.stdout.write(os.ttyname(slave) + "\n")
sys.stdout.flush()
try:
    while True:
        time.sleep(1)
except Exception:
    pass
os.close(master)
os.close(slave)
"#,
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .ok()?;
            let mut stdout = child.stdout.take()?;
            let mut buf = [0u8; 128];
            // Blocking read of one line
            let mut line = Vec::new();
            loop {
                let n = std::io::Read::read(&mut stdout, &mut buf).ok()?;
                if n == 0 {
                    let _ = child.kill();
                    return None;
                }
                line.extend_from_slice(&buf[..n]);
                if line.contains(&b'\n') {
                    break;
                }
            }
            let path = String::from_utf8_lossy(&line)
                .lines()
                .next()?
                .trim()
                .to_string();
            if path.is_empty() || !path.starts_with("/dev/") {
                let _ = child.kill();
                return None;
            }
            // Reattach stdout drain so the child isn't blocked later
            std::thread::spawn(move || {
                let mut sink = stdout;
                let mut b = [0u8; 64];
                while std::io::Read::read(&mut sink, &mut b).unwrap_or(0) > 0 {}
            });
            Some(Self { path, child })
        }
    }

    impl Drop for PtyPair {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    #[test]
    fn open_close_reopen_releases_port() {
        let Some(pty) = PtyPair::spawn() else {
            eprintln!("skip: could not allocate PTY");
            return;
        };
        let path = &pty.path;

        let io = open_serial(path, 115200).expect("first open");
        // While held, a second open must fail closed with a clear Late error.
        match open_serial(path, 115200) {
            Ok(extra) => {
                extra.release();
                panic!("second open while held should fail");
            }
            Err(busy) => {
                let busy_msg = busy.to_string();
                assert!(
                    busy_msg.contains("already open in Late") || busy_msg.contains("busy"),
                    "unexpected busy message: {busy_msg}"
                );
            }
        }

        io.release();

        let io2 = open_serial(path, 115200).expect("reopen after release");
        io2.release();

        // Third cycle — still no app restart required.
        let io3 = open_serial(path, 115200).expect("third open");
        io3.release();
    }

    #[test]
    fn signal_close_without_release_join_still_frees_via_disconnect() {
        let Some(pty) = PtyPair::spawn() else {
            eprintln!("skip: could not allocate PTY");
            return;
        };
        let path = pty.path.clone();
        let mut io = open_serial(&path, 9600).expect("open");
        let _ = io.close.try_send(());
        // Mimic session close: drop close/input senders, then join worker.
        drop(std::mem::replace(
            &mut io.close,
            mpsc::channel::<()>(1).0,
        ));
        let join = io.join.take();
        drop(io);
        if let Some(h) = join {
            h.join().expect("worker join");
        }
        open_serial(&path, 9600)
            .expect("reopen after close+join")
            .release();
    }

    #[test]
    fn foreign_holder_fails_closed_clearly() {
        let Some(pty) = PtyPair::spawn() else {
            eprintln!("skip: could not allocate PTY");
            return;
        };
        // Hold exclusive lock outside Late.
        let foreign = serialport::new(&pty.path, 9600)
            .timeout(Duration::from_millis(50))
            .open();
        let Ok(mut foreign) = foreign else {
            eprintln!("skip: serialport could not open PTY ({})", pty.path);
            return;
        };
        let _ = foreign.write_all(b"");
        match open_serial(&pty.path, 9600) {
            Ok(io) => {
                io.release();
                panic!("should fail while foreign holds");
            }
            Err(err) => {
                let msg = err.to_string();
                assert!(
                    msg.contains("busy on your computer") || msg.contains("busy"),
                    "expected clear busy copy, got: {msg}"
                );
            }
        }
        drop(foreign);
        open_serial(&pty.path, 9600)
            .expect("open after foreign drop")
            .release();
    }
}
