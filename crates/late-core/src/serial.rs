use crate::error::{LateError, Result};
use std::io::{Read, Write};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc};

pub struct SerialIo {
    pub tx: mpsc::Sender<Vec<u8>>,
    pub break_tx: mpsc::Sender<()>,
    pub rx: broadcast::Receiver<Vec<u8>>,
    pub close: mpsc::Sender<()>,
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

fn serial_err(path: &str, e: impl std::fmt::Display) -> LateError {
    let msg = e.to_string();
    if msg.to_ascii_lowercase().contains("permission denied") {
        LateError::Serial(format!(
            "{path}: permission denied. This port is owned by the dialout group. Run:  sudo usermod -aG dialout $USER   then log out and back in (or reboot). Until then:  sudo chmod 666 {path}  is a temporary workaround."
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
        if close_rx.try_recv().is_ok() {
            return true;
        }
        while brk_rx.try_recv().is_ok() {
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
    let writer = open_port(path, baud)?;
    let reader = writer
        .try_clone()
        .map_err(|e| LateError::Serial(e.to_string()))?;

    let (in_tx, mut in_rx) = mpsc::channel::<Vec<u8>>(64);
    let (brk_tx, mut brk_rx) = mpsc::channel::<()>(8);
    let (out_tx, _) = broadcast::channel::<Vec<u8>>(64);
    let (close_tx, mut close_rx) = mpsc::channel::<()>(1);
    let out_tx2 = out_tx.clone();
    let path = path.to_string();

    std::thread::spawn(move || {
        let mut writer = Some(writer);
        let mut reader = Some(reader);
        let mut last_wait_notice = Instant::now() - Duration::from_secs(10);
        loop {
            if close_rx.try_recv().is_ok() {
                break;
            }
            let (w, r) = match (writer.take(), reader.take()) {
                (Some(w), Some(r)) => (w, r),
                _ => match open_port(&path, baud) {
                    Ok(w) => match w.try_clone() {
                        Ok(r) => {
                            notice(&out_tx2, &format!("serial reconnected ({path})"));
                            (w, r)
                        }
                        Err(e) => {
                            if last_wait_notice.elapsed() >= Duration::from_secs(4) {
                                notice(&out_tx2, &format!("waiting for {path}: {e}"));
                                last_wait_notice = Instant::now();
                            }
                            std::thread::sleep(Duration::from_millis(400));
                            continue;
                        }
                    },
                    Err(e) => {
                        if last_wait_notice.elapsed() >= Duration::from_secs(4) {
                            notice(&out_tx2, &format!("waiting for {path}: {e}"));
                            last_wait_notice = Instant::now();
                        }
                        std::thread::sleep(Duration::from_millis(400));
                        continue;
                    }
                },
            };
            let user_closed = run_port(w, r, &mut in_rx, &mut brk_rx, &mut close_rx, &out_tx2);
            if user_closed {
                break;
            }
            notice(&out_tx2, &format!("serial port lost — reconnecting {path}…"));
        }
    });

    Ok(SerialIo {
        tx: in_tx,
        break_tx: brk_tx,
        rx: out_tx.subscribe(),
        close: close_tx,
    })
}
