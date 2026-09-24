//! SSH connect: one-time vs save. Passwords never hit the vault on one-time.

use crate::error::{LateError, Result};
use crate::inventory::InventoryStore;
use crate::secrets::SecretStore;
use crate::types::{AuthProfile, Device, Vendor};
use uuid::Uuid;
use zeroize::Zeroize;

/// Login fields for this connect attempt. `password` is zeroized on drop.
pub struct ConnectLogin {
    pub username: Option<String>,
    pub password: Option<String>,
    pub key_path: Option<String>,
    pub save_session: bool,
    pub save_password: bool,
}

impl Drop for ConnectLogin {
    fn drop(&mut self) {
        if let Some(ref mut pw) = self.password {
            pw.zeroize();
        }
    }
}

pub struct PreparedSsh {
    pub device: Device,
    pub profile: AuthProfile,
    /// In-memory password for this session. `None` means use the vault (if any).
    pub session_password: Option<String>,
    pub save_session: bool,
    pub save_password: bool,
}

impl Drop for PreparedSsh {
    fn drop(&mut self) {
        if let Some(ref mut pw) = self.session_password {
            pw.zeroize();
        }
    }
}

/// Resolve device + profile in memory. Does not write inventory or `secrets.json`.
pub fn prepare_ssh(
    inventory: &InventoryStore,
    _secrets: &SecretStore,
    device_id: Option<&str>,
    host: Option<&str>,
    port: Option<u16>,
    name: Option<&str>,
    vendor: Option<Vendor>,
    login: ConnectLogin,
) -> Result<PreparedSsh> {
    let typed_pw = login
        .password
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let typed_user = login
        .username
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let typed_key = login
        .key_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let requested_id = device_id.map(str::trim).filter(|s| !s.is_empty());
    let existing = requested_id.and_then(|id| inventory.get(id).ok());

    let host = host
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| existing.as_ref().and_then(|d| d.host.clone()));
    let host = host.ok_or_else(|| LateError::Message("hostname or IP is required".into()))?;
    let port = port
        .filter(|p| *p > 0)
        .or_else(|| existing.as_ref().and_then(|d| d.port))
        .unwrap_or(22);

    let saved_profile = existing
        .as_ref()
        .and_then(|d| d.auth_profile_id.as_deref())
        .and_then(|id| inventory.get_auth(id).ok());

    let username = typed_user
        .map(|s| s.to_string())
        .or_else(|| saved_profile.as_ref().map(|p| p.username.clone()))
        .ok_or_else(|| LateError::Message("username is required".into()))?;

    let key_path = typed_key
        .map(|s| s.to_string())
        .or_else(|| saved_profile.as_ref().and_then(|p| p.key_path.clone()));

    let saved_has_password = saved_profile.as_ref().is_some_and(|p| p.has_password);
    let has_key = key_path.as_ref().is_some_and(|k| !k.is_empty());
    let use_agent =
        saved_profile.as_ref().is_some_and(|p| p.use_agent) && typed_pw.is_none() && !has_key;

    if typed_pw.is_none() && !has_key && !saved_has_password && !use_agent {
        return Err(LateError::Message(
            "enter a password (or a key path) so Late can log in".into(),
        ));
    }

    // Never persist a password on one-time, even if the checkbox was left on.
    let persist_device = login.save_session;
    let persist_password = persist_device && login.save_password && typed_pw.is_some();

    let session_name = name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| existing.as_ref().map(|d| d.name.clone()))
        .unwrap_or_else(|| host.clone());
    let vendor = vendor.unwrap_or_else(|| {
        existing
            .as_ref()
            .map(|d| d.vendor)
            .unwrap_or(Vendor::Generic)
    });

    let profile_id = saved_profile
        .as_ref()
        .map(|p| p.id.clone())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let has_password = persist_password || typed_pw.is_some() || saved_has_password;
    let mut device = existing.unwrap_or_else(|| {
        let mut d = Device::new_ssh(&session_name, &host, vendor);
        if let Some(id) = requested_id {
            d.id = id.to_string();
        }
        d
    });
    device.name = session_name.clone();
    device.host = Some(host);
    device.port = Some(port);
    device.vendor = vendor;
    device.kind = crate::types::DeviceKind::Ssh;
    device.auth_profile_id = Some(profile_id.clone());
    let profile = AuthProfile {
        id: profile_id,
        name: format!("{session_name} login"),
        username,
        key_path,
        use_agent: !has_password && !has_key,
        has_password,
    };
    Ok(PreparedSsh {
        device,
        profile,
        session_password: typed_pw,
        save_session: persist_device,
        save_password: persist_password,
    })
}

/// Write inventory / vault after SSH actually opened. No-op for one-time.
pub fn commit_ssh(
    inventory: &InventoryStore,
    secrets: &SecretStore,
    prepared: &PreparedSsh,
) -> Result<(Device, AuthProfile)> {
    if !prepared.save_session {
        return Ok((prepared.device.clone(), prepared.profile.clone()));
    }
    let mut profile = prepared.profile.clone();
    if prepared.save_password {
        if let Some(ref pw) = prepared.session_password {
            secrets.set(&profile.id, pw)?;
            profile.has_password = true;
        }
    } else {
        profile.has_password = secrets.get(&profile.id)?.is_some();
        profile.use_agent =
            !profile.has_password && !profile.key_path.as_deref().is_some_and(|k| !k.is_empty());
    }
    let profile = inventory.upsert_auth(profile)?;
    let mut device = prepared.device.clone();
    device.auth_profile_id = Some(profile.id.clone());
    let device = inventory.upsert_device(device)?;
    Ok((device, profile))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LatePaths;
    use std::fs;

    fn isolated() -> (InventoryStore, SecretStore, LatePaths, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("late-connect-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let paths = LatePaths {
            config: dir.clone(),
            data: dir.clone(),
        };
        paths.ensure().unwrap();
        (
            InventoryStore::new(paths.clone()),
            SecretStore::new(paths.clone()),
            paths,
            dir,
        )
    }

    fn secrets_blob(paths: &LatePaths) -> String {
        fs::read_to_string(paths.secrets_file()).unwrap_or_default()
    }

    #[test]
    fn one_time_does_not_persist_password_or_device() {
        let (inv, secrets, paths, dir) = isolated();
        let one_time = "one-time-lab-password-xyz";
        let prepared = prepare_ssh(
            &inv,
            &secrets,
            None,
            Some("10.1.0.12"),
            Some(22),
            Some("lab-sw"),
            Some(Vendor::AosCx),
            ConnectLogin {
                username: Some("admin".into()),
                password: Some(one_time.into()),
                key_path: None,
                save_session: false,
                save_password: true, // must still not persist
            },
        )
        .unwrap();
        assert!(!prepared.save_session);
        assert!(!prepared.save_password);
        let _ = commit_ssh(&inv, &secrets, &prepared).unwrap();
        assert_eq!(prepared.session_password.as_deref(), Some(one_time));
        assert!(inv.load().unwrap().devices.is_empty());
        assert!(inv.load_auth().unwrap().is_empty());
        let blob = secrets_blob(&paths);
        assert!(
            !blob.contains(one_time),
            "one-time password must not be written to secrets.json"
        );
        assert!(secrets.get(&prepared.profile.id).unwrap().is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn save_persists_password() {
        let (inv, secrets, paths, dir) = isolated();
        let pw = "saved-lab-password-xyz";
        let prepared = prepare_ssh(
            &inv,
            &secrets,
            None,
            Some("10.1.0.10"),
            Some(22),
            Some("clab"),
            Some(Vendor::Linux),
            ConnectLogin {
                username: Some("root".into()),
                password: Some(pw.into()),
                key_path: None,
                save_session: true,
                save_password: true,
            },
        )
        .unwrap();
        assert!(prepared.save_session);
        assert!(prepared.save_password);
        assert!(
            inv.load().unwrap().devices.is_empty(),
            "must not write until commit"
        );
        let (device, profile) = commit_ssh(&inv, &secrets, &prepared).unwrap();
        assert_eq!(inv.load().unwrap().devices.len(), 1);
        assert_eq!(device.host.as_deref(), Some("10.1.0.10"));
        assert_eq!(secrets.get(&profile.id).unwrap().as_deref(), Some(pw));
        let mode = fs::metadata(paths.secrets_file()).unwrap().permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(mode.mode() & 0o777, 0o600);
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn save_session_without_password_ok() {
        let (inv, secrets, _paths, dir) = isolated();
        let prepared = prepare_ssh(
            &inv,
            &secrets,
            None,
            Some("192.0.2.10"),
            Some(22),
            Some("edge"),
            None,
            ConnectLogin {
                username: Some("admin".into()),
                password: None,
                key_path: Some("/home/you/.ssh/id_ed25519".into()),
                save_session: true,
                save_password: true,
            },
        )
        .unwrap();
        assert!(prepared.save_session);
        assert!(!prepared.save_password);
        let (_, profile) = commit_ssh(&inv, &secrets, &prepared).unwrap();
        assert!(!profile.has_password);
        assert!(secrets.get(&profile.id).unwrap().is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn prompt_on_connect_resolves_session_auth() {
        let (inv, secrets, paths, dir) = isolated();
        let profile = inv
            .upsert_auth(AuthProfile {
                id: String::new(),
                name: "6200F login".into(),
                username: "admin".into(),
                key_path: None,
                use_agent: false,
                has_password: true,
            })
            .unwrap();
        secrets.set(&profile.id, "vault-only-secret").unwrap();
        let mut d = Device::new_ssh("6200F", "10.1.0.12", Vendor::AosCx);
        d.auth_profile_id = Some(profile.id.clone());
        let d = inv.upsert_device(d).unwrap();

        let override_pw = "prompt-override-this-time";
        let prepared = prepare_ssh(
            &inv,
            &secrets,
            Some(&d.id),
            None,
            None,
            None,
            None,
            ConnectLogin {
                username: Some("admin".into()),
                password: Some(override_pw.into()),
                key_path: None,
                save_session: false,
                save_password: false,
            },
        )
        .unwrap();
        assert_eq!(prepared.profile.username, "admin");
        assert_eq!(prepared.device.host.as_deref(), Some("10.1.0.12"));
        assert_eq!(prepared.session_password.as_deref(), Some(override_pw));
        assert!(!prepared.save_password);
        let _ = commit_ssh(&inv, &secrets, &prepared).unwrap();
        assert_eq!(
            secrets.get(&profile.id).unwrap().as_deref(),
            Some("vault-only-secret"),
            "one-time override must not replace the vault password"
        );
        assert!(!secrets_blob(&paths).contains(override_pw));

        let saved = prepare_ssh(
            &inv,
            &secrets,
            Some(&d.id),
            None,
            None,
            None,
            None,
            ConnectLogin {
                username: None,
                password: None,
                key_path: None,
                save_session: false,
                save_password: false,
            },
        )
        .unwrap();
        assert!(saved.session_password.is_none());
        assert_eq!(saved.profile.id, profile.id);
        assert!(saved.profile.has_password);
        let _ = fs::remove_dir_all(dir);
    }
}
