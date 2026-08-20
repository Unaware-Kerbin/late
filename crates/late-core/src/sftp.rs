use crate::error::{LateError, Result};
use crate::secrets::SecretStore;
use crate::types::AuthProfile;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SftpEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirListing {
    pub path: String,
    pub entries: Vec<SftpEntry>,
}

fn ssh_base(
    profile: &AuthProfile,
    host: &str,
    port: u16,
    secrets: &SecretStore,
) -> Result<Command> {
    let pw = if profile.has_password {
        secrets.get(&profile.id)?
    } else {
        None
    };
    let mut cmd = crate::ssh::ssh_command_with_secret("ssh", pw.as_deref())?;
    if !profile.has_password {
        cmd.arg("-o").arg("BatchMode=yes");
    }
    crate::ssh::apply_strict_host_opts_std(&mut cmd, host, port)?;
    cmd.arg("-p").arg(port.to_string());
    if let Some(key) = &profile.key_path {
        cmd.arg("-i").arg(key);
    }
    cmd.arg(format!("{}@{}", profile.username, host));
    Ok(cmd)
}

pub fn list_dir(
    profile: &AuthProfile,
    secrets: &SecretStore,
    host: &str,
    port: u16,
    path: &str,
) -> Result<Vec<SftpEntry>> {
    let remote = if path.trim().is_empty() { "/" } else { path };
    let mut cmd = ssh_base(profile, host, port, secrets)?;
    cmd.arg(format!("ls -la {}", shell_escape(remote)));
    let out = cmd.output().map_err(|e| LateError::Sftp(e.to_string()))?;
    if !out.status.success() {
        return Err(LateError::Sftp(
            String::from_utf8_lossy(&out.stderr).into_owned(),
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(parse_ls(remote, &text))
}

pub fn parse_ls(dir: &str, text: &str) -> Vec<SftpEntry> {
    let mut entries = Vec::new();
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() || line.to_ascii_lowercase().starts_with("total ") {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 8 {
            continue;
        }
        let mode = parts[0];
        if !mode
            .chars()
            .next()
            .is_some_and(|c| matches!(c, 'd' | '-' | 'l' | 'b' | 'c' | 'p' | 's'))
        {
            continue;
        }
        let name_idx = if parts.len() >= 9 { 8 } else { 7 };
        let name = parts[name_idx..].join(" ");
        if name.is_empty() || name == "." || name == ".." {
            continue;
        }
        if name.contains('/') || name.contains('\\') || name.contains('\0') {
            continue;
        }
        let is_dir = mode.starts_with('d') || (mode.starts_with('l') && line.ends_with('/'));
        let size = parts[4].parse().unwrap_or(0);
        entries.push(SftpEntry {
            path: join_remote(dir, &name),
            name,
            is_dir,
            size,
        });
    }
    sort_entries(&mut entries);
    entries
}

pub fn default_local_dir() -> String {
    dirs::home_dir()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("/"))
        .to_string_lossy()
        .into_owned()
}

pub fn list_local(path: &str) -> Result<DirListing> {
    let raw = if path.trim().is_empty() {
        default_local_dir()
    } else {
        path.to_string()
    };
    let p = PathBuf::from(&raw);
    if !p.exists() {
        return Err(LateError::Sftp(format!(
            "local path not found: {}",
            p.display()
        )));
    }
    if !p.is_dir() {
        return Err(LateError::Sftp(format!("not a directory: {}", p.display())));
    }
    let canon = p.canonicalize().unwrap_or(p.clone());
    let mut roots = Vec::new();
    if let Some(home) = dirs::home_dir() {
        roots.push(home);
    }
    crate::confine::confine_dir(&canon, &roots).map_err(|e| LateError::Sftp(e.to_string()))?;
    let mut entries = Vec::new();
    let rd = std::fs::read_dir(&canon).map_err(|e| LateError::Sftp(e.to_string()))?;
    for ent in rd.flatten() {
        let name = ent.file_name().to_string_lossy().into_owned();
        if name == "." || name == ".." {
            continue;
        }
        let meta = ent.metadata().ok();
        let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
        let size = meta.map(|m| m.len()).unwrap_or(0);
        entries.push(SftpEntry {
            name,
            path: ent.path().to_string_lossy().into_owned(),
            is_dir,
            size,
        });
    }
    sort_entries(&mut entries);
    Ok(DirListing {
        path: canon.to_string_lossy().into_owned(),
        entries,
    })
}

fn sort_entries(entries: &mut [SftpEntry]) {
    entries.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir).then_with(|| {
            a.name
                .to_ascii_lowercase()
                .cmp(&b.name.to_ascii_lowercase())
        })
    });
}

fn join_remote(dir: &str, name: &str) -> String {
    if name == ".." {
        return parent_remote(dir);
    }
    if dir.is_empty() || dir == "/" {
        format!("/{name}").replace("//", "/")
    } else if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

fn parent_remote(dir: &str) -> String {
    let t = dir.trim_end_matches('/');
    if t.is_empty() || t == "/" {
        return "/".into();
    }
    match t.rsplit_once('/') {
        Some(("", _)) => "/".into(),
        Some((p, _)) => p.to_string(),
        None => "/".into(),
    }
}

pub fn download(
    profile: &AuthProfile,
    secrets: &SecretStore,
    host: &str,
    port: u16,
    remote: &str,
    local: &str,
) -> Result<()> {
    if Path::new(local).is_dir() {
        return Err(LateError::Sftp(
            "download destination is a directory; pick a file path".into(),
        ));
    }
    scp(profile, secrets, host, port, remote, local, false)
}

pub fn upload(
    profile: &AuthProfile,
    secrets: &SecretStore,
    host: &str,
    port: u16,
    local: &str,
    remote: &str,
) -> Result<()> {
    if !Path::new(local).is_file() {
        return Err(LateError::Sftp(format!("local file not found: {local}")));
    }
    scp(profile, secrets, host, port, local, remote, true)
}

fn scp(
    profile: &AuthProfile,
    secrets: &SecretStore,
    host: &str,
    port: u16,
    src: &str,
    dst: &str,
    upload: bool,
) -> Result<()> {
    let remote = format!(
        "{}@{}:{}",
        profile.username,
        host,
        if upload { dst } else { src }
    );
    let (from, to) = if upload {
        (src.to_string(), remote)
    } else {
        (remote, dst.to_string())
    };
    let pw = if profile.has_password {
        secrets.get(&profile.id)?
    } else {
        None
    };
    let mut cmd = crate::ssh::ssh_command_with_secret("scp", pw.as_deref())?;
    cmd.arg("-P").arg(port.to_string());
    if let Some(key) = &profile.key_path {
        cmd.arg("-i").arg(key);
    }
    crate::ssh::apply_strict_host_opts_std(&mut cmd, host, port)?;
    cmd.arg(&from).arg(&to);
    let out = cmd.output().map_err(|e| LateError::Sftp(e.to_string()))?;
    if !out.status.success() {
        return Err(LateError::Sftp(
            String::from_utf8_lossy(&out.stderr).into_owned(),
        ));
    }
    Ok(())
}

pub fn mkdir(
    profile: &AuthProfile,
    secrets: &SecretStore,
    host: &str,
    port: u16,
    path: &str,
) -> Result<()> {
    let mut cmd = ssh_base(profile, host, port, secrets)?;
    cmd.arg(format!("mkdir -p {}", shell_escape(path)));
    let out = cmd.output().map_err(|e| LateError::Sftp(e.to_string()))?;
    if !out.status.success() {
        return Err(LateError::Sftp(
            String::from_utf8_lossy(&out.stderr).into_owned(),
        ));
    }
    Ok(())
}

pub fn remove(
    profile: &AuthProfile,
    secrets: &SecretStore,
    host: &str,
    port: u16,
    path: &str,
    dir: bool,
) -> Result<()> {
    let mut cmd = ssh_base(profile, host, port, secrets)?;
    cmd.arg(format!(
        "{} {}",
        if dir { "rm -rf" } else { "rm -f" },
        shell_escape(path)
    ));
    let out = cmd.output().map_err(|e| LateError::Sftp(e.to_string()))?;
    if !out.status.success() {
        return Err(LateError::Sftp(
            String::from_utf8_lossy(&out.stderr).into_owned(),
        ));
    }
    Ok(())
}

fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ls_gnu_and_iso() {
        let gnu = "\
total 12
drwxr-xr-x  3 root root 4096 Jan  1 12:00 .
drwxr-xr-x 18 root root 4096 Jan  1 11:00 ..
drwxr-xr-x  2 root root 4096 Jan  1 12:00 etc
-rw-r--r--  1 root root  123 Jan  1 12:00 note with spaces.txt
";
        let entries = parse_ls("/var", gnu);
        assert_eq!(entries.len(), 2);
        assert!(entries[0].is_dir);
        assert_eq!(entries[0].name, "etc");
        assert_eq!(entries[0].path, "/var/etc");
        assert_eq!(entries[1].name, "note with spaces.txt");
        assert!(!entries[1].is_dir);
        assert_eq!(entries[1].size, 123);

        let iso = "\
-rw-r--r-- 1 lab lab 8 2024-01-01 12:00:00 readme
drwxr-xr-x 2 lab lab 64 2024-01-01 12:00:00 bin
";
        let iso_entries = parse_ls("/", iso);
        assert_eq!(iso_entries[0].name, "bin");
        assert!(iso_entries[0].is_dir);
        assert_eq!(iso_entries[1].name, "readme");
    }

    #[test]
    fn list_local_tmp() {
        let home = dirs::home_dir().expect("home");
        let dir = tempfile::TempDir::new_in(&home).unwrap();
        std::fs::write(dir.path().join("a.txt"), b"hi").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let listing = list_local(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(listing.entries.len(), 2);
        assert!(listing.entries[0].is_dir);
        assert_eq!(listing.entries[0].name, "sub");
        assert_eq!(listing.entries[1].name, "a.txt");
        assert_eq!(listing.entries[1].size, 2);
    }
}
