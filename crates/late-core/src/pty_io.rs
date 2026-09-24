//! Interactive PTY stdin. Block on the next keystroke instead of sleeping 8ms.

use std::io::Write;
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::sync::mpsc;

enum Ev<R> {
    Input(Vec<u8>),
    Resize(R),
    Stop,
}

/// Drain stdin/resize/close onto a PTY master. Returns after close or disconnect.
pub fn pump_writes<W, R>(
    mut writer: W,
    mut in_rx: mpsc::Receiver<Vec<u8>>,
    mut resize_rx: mpsc::Receiver<R>,
    mut close_rx: mpsc::Receiver<()>,
    mut on_resize: impl FnMut(R),
    handle: Option<Handle>,
) where
    W: Write,
{
    loop {
        match next_event(&mut in_rx, &mut resize_rx, &mut close_rx, handle.as_ref()) {
            Ev::Input(buf) => {
                if writer.write_all(&buf).is_err() {
                    break;
                }
                while let Ok(more) = in_rx.try_recv() {
                    if writer.write_all(&more).is_err() {
                        return;
                    }
                }
                let _ = writer.flush();
            }
            Ev::Resize(sz) => {
                let mut last = sz;
                while let Ok(more) = resize_rx.try_recv() {
                    last = more;
                }
                on_resize(last);
            }
            Ev::Stop => break,
        }
    }
}

fn next_event<R>(
    in_rx: &mut mpsc::Receiver<Vec<u8>>,
    resize_rx: &mut mpsc::Receiver<R>,
    close_rx: &mut mpsc::Receiver<()>,
    handle: Option<&Handle>,
) -> Ev<R> {
    if let Some(h) = handle {
        return h.block_on(async {
            tokio::select! {
                b = in_rx.recv() => b.map(Ev::Input).unwrap_or(Ev::Stop),
                r = resize_rx.recv() => match r {
                    Some(sz) => {
                        let mut last = sz;
                        while let Ok(more) = resize_rx.try_recv() {
                            last = more;
                        }
                        Ev::Resize(last)
                    }
                    None => Ev::Stop,
                },
                _ = close_rx.recv() => Ev::Stop,
            }
        });
    }
    loop {
        match close_rx.try_recv() {
            Ok(()) | Err(mpsc::error::TryRecvError::Disconnected) => return Ev::Stop,
            Err(mpsc::error::TryRecvError::Empty) => {}
        }
        if let Ok(sz) = resize_rx.try_recv() {
            let mut last = sz;
            while let Ok(more) = resize_rx.try_recv() {
                last = more;
            }
            return Ev::Resize(last);
        }
        match in_rx.try_recv() {
            Ok(buf) => return Ev::Input(buf),
            Err(mpsc::error::TryRecvError::Disconnected) => return Ev::Stop,
            Err(mpsc::error::TryRecvError::Empty) => {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    struct Sink(Arc<Mutex<Vec<u8>>>);

    impl Write for Sink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pump_writes_keystroke_without_poll_sleep() {
        let (in_tx, in_rx) = mpsc::channel::<Vec<u8>>(8);
        let (_resize_tx, resize_rx) = mpsc::channel::<(u32, u32)>(1);
        let (close_tx, close_rx) = mpsc::channel::<()>(1);
        let got = Arc::new(Mutex::new(Vec::new()));
        let sink = Sink(got.clone());
        let handle = Handle::current();
        let join = std::thread::spawn(move || {
            pump_writes(sink, in_rx, resize_rx, close_rx, |_| {}, Some(handle));
        });

        let t0 = Instant::now();
        in_tx.send(b"ab".to_vec()).await.unwrap();
        let deadline = Instant::now() + Duration::from_millis(80);
        loop {
            if got.lock().unwrap().as_slice() == b"ab" {
                break;
            }
            if Instant::now() > deadline {
                panic!("PTY write did not flush promptly");
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        assert!(
            t0.elapsed() < Duration::from_millis(50),
            "keystroke waited {:?}",
            t0.elapsed()
        );

        drop(in_tx);
        drop(close_tx);
        join.join().unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pump_writes_keeps_latest_resize() {
        let (_in_tx, in_rx) = mpsc::channel::<Vec<u8>>(8);
        let (resize_tx, resize_rx) = mpsc::channel::<(u32, u32)>(8);
        let (close_tx, close_rx) = mpsc::channel::<()>(1);
        let got = Arc::new(Mutex::new(Vec::new()));
        let sizes = Arc::new(Mutex::new(Vec::new()));
        let sink = Sink(got);
        let handle = Handle::current();
        let sizes_thread = sizes.clone();
        let join = std::thread::spawn(move || {
            pump_writes(
                sink,
                in_rx,
                resize_rx,
                close_rx,
                |sz| sizes_thread.lock().unwrap().push(sz),
                Some(handle),
            );
        });

        resize_tx.send((80, 24)).await.unwrap();
        resize_tx.send((120, 36)).await.unwrap();
        resize_tx.send((132, 43)).await.unwrap();
        let deadline = Instant::now() + Duration::from_millis(80);
        loop {
            if sizes.lock().unwrap().last() == Some(&(132, 43)) {
                break;
            }
            if Instant::now() > deadline {
                panic!("resize did not apply, got {:?}", sizes.lock().unwrap());
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        drop(resize_tx);
        drop(close_tx);
        join.join().unwrap();
        let applied = sizes.lock().unwrap().clone();
        assert_eq!(*applied.last().unwrap(), (132, 43));
        assert!(
            applied.len() <= 3,
            "resize storm should coalesce, got {applied:?}"
        );
    }
}
