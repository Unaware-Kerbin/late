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
    let tmp = path.with_file_name(format!(
        "{}.tmp",
        path.file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_else(|| "secret".into())
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
    #[cfg(unix)]
    {
        let mut p = fs::metadata(path)?.permissions();
        p.set_mode(0o600);
        fs::set_permissions(path, p)?;
    }
    Ok(())
}
