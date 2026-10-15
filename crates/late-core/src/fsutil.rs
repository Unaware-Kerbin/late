//! Create directories 0700 and write files 0600 without a umask-readable window.

use crate::error::Result;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

pub fn mkdir_private(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        let mut p = fs::metadata(path)?.permissions();
        p.set_mode(0o700);
        fs::set_permissions(path, p)?;
    }
    Ok(())
}

pub fn write_private(path: &Path, data: impl AsRef<[u8]>) -> Result<()> {
    if let Some(parent) = path.parent() {
        mkdir_private(parent)?;
    }
    // Unique tmp per call — parallel writers to the same final path (e.g. two
    // session.open → ssh-known/*.known) must not share `{name}.tmp` or one
    // rename leaves the other with ENOENT.
    let tmp = path.with_file_name(format!(
        "{}.{}.tmp",
        path.file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_else(|| "secret".into()),
        uuid::Uuid::new_v4()
    ));
    {
        let mut opts = OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        opts.mode(0o600);
        let mut f = opts.open(&tmp)?;
        f.write_all(data.as_ref())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    chmod_private(path)
}

/// Create-or-append with 0600. Existing files are chmod'd after write.
pub fn append_private(path: &Path, data: impl AsRef<[u8]>) -> Result<()> {
    if let Some(parent) = path.parent() {
        mkdir_private(parent)?;
    }
    let mut opts = OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    opts.mode(0o600);
    let mut f = opts.open(path)?;
    f.write_all(data.as_ref())?;
    chmod_private(path)
}

pub fn chmod_private(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        let mut p = fs::metadata(path)?.permissions();
        p.set_mode(0o600);
        fs::set_permissions(path, p)?;
    }
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    use std::thread;

    #[test]
    fn write_private_parallel_same_path_no_enoent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shared.known");
        let barrier = Arc::new(Barrier::new(8));
        let mut handles = Vec::new();
        for i in 0..8 {
            let path = path.clone();
            let barrier = barrier.clone();
            handles.push(thread::spawn(move || {
                barrier.wait();
                write_private(&path, format!("line-{i}\n").as_bytes()).expect("write_private")
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert!(path.is_file());
        assert!(!fs::read_to_string(&path).unwrap().is_empty());
    }
}
