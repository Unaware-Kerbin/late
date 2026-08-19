use crate::error::{LateError, Result};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use tokio::sync::{broadcast, mpsc};

pub struct LocalPty {
    pub tx: mpsc::Sender<Vec<u8>>,
    pub rx: broadcast::Receiver<Vec<u8>>,
    pub resize: mpsc::Sender<(u16, u16)>,
    pub close: mpsc::Sender<()>,
}

pub fn discover_shells() -> Vec<(String, String)> {
    let candidates = [
        ("bash", "/bin/bash"),
        ("zsh", "/bin/zsh"),
        ("fish", "/usr/bin/fish"),
        ("sh", "/bin/sh"),
        ("python", "/usr/bin/python3"),
        ("node", "/usr/bin/node"),
    ];
    candidates
        .into_iter()
        .filter(|(_, p)| std::path::Path::new(p).exists())
        .map(|(n, p)| (n.to_string(), p.to_string()))
        .collect()
}

pub fn open_local(shell: Option<&str>, cols: u16, rows: u16) -> Result<LocalPty> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| LateError::Message(e.to_string()))?;

    let exe = shell
        .map(|s| s.to_string())
        .or_else(|| std::env::var("SHELL").ok())
        .unwrap_or_else(|| "/bin/bash".into());
    let cmd = CommandBuilder::new(exe);
    let mut child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| LateError::Message(e.to_string()))?;

    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| LateError::Message(e.to_string()))?;
    let mut writer = pair
        .master
        .take_writer()
        .map_err(|e| LateError::Message(e.to_string()))?;
    let master = pair.master;

    let (in_tx, mut in_rx) = mpsc::channel::<Vec<u8>>(64);
    let (out_tx, _) = broadcast::channel::<Vec<u8>>(256);
    let (resize_tx, mut resize_rx) = mpsc::channel::<(u16, u16)>(8);
    let (close_tx, mut close_rx) = mpsc::channel::<()>(1);
    let out_tx2 = out_tx.clone();

    std::thread::spawn(move || {
        loop {
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
                    rows: r,
                    cols: c,
                    pixel_width: 0,
                    pixel_height: 0,
                });
            }
            std::thread::sleep(std::time::Duration::from_millis(8));
        }
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

    Ok(LocalPty {
        tx: in_tx,
        rx: out_tx.subscribe(),
        resize: resize_tx,
        close: close_tx,
    })
}
