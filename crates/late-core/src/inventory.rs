use crate::config::LatePaths;
use crate::error::{LateError, Result};
use crate::types::{AuthProfile, Device, Inventory};
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct AuthFile {
    #[serde(default)]
    profiles: Vec<AuthProfile>,
}

#[derive(Clone)]
pub struct InventoryStore {
    paths: LatePaths,
}

impl InventoryStore {
    pub fn new(paths: LatePaths) -> Self {
        Self { paths }
    }

    pub fn load(&self) -> Result<Inventory> {
        let path = self.paths.inventory();
        if !path.exists() {
            let inv = Inventory::default();
            self.save(&inv)?;
            return Ok(inv);
        }
        let raw = fs::read_to_string(path)?;
        Ok(toml::from_str(&raw)?)
    }

    pub fn save(&self, inv: &Inventory) -> Result<()> {
        fs::write(
            self.paths.inventory(),
            toml::to_string_pretty(inv).map_err(|e| LateError::Config(e.to_string()))?,
        )?;
        Ok(())
    }

    pub fn upsert_device(&self, mut device: Device) -> Result<Device> {
        if device.id.is_empty() {
            device.id = uuid::Uuid::new_v4().to_string();
        }
        let mut inv = self.load()?;
        if let Some(existing) = inv.devices.iter_mut().find(|d| d.id == device.id) {
            *existing = device.clone();
        } else {
            inv.devices.push(device.clone());
        }
        if let Some(folder) = &device.folder {
            if !inv.folders.contains(folder) {
                inv.folders.push(folder.clone());
            }
        }
        self.save(&inv)?;
        Ok(device)
    }

    pub fn delete_device(&self, id: &str) -> Result<()> {
        let mut inv = self.load()?;
        inv.devices.retain(|d| d.id != id);
        self.save(&inv)
    }

    pub fn get(&self, id: &str) -> Result<Device> {
        self.load()?
            .devices
            .into_iter()
            .find(|d| d.id == id)
            .ok_or_else(|| LateError::NotFound(format!("device {id}")))
    }

    pub fn load_auth(&self) -> Result<Vec<AuthProfile>> {
        let path = self.paths.auth_profiles();
        if !path.exists() {
            return Ok(vec![]);
        }
        let raw = fs::read_to_string(path)?;
        let file: AuthFile = toml::from_str(&raw)?;
        Ok(file.profiles)
    }

    pub fn save_auth(&self, profiles: &[AuthProfile]) -> Result<()> {
        let file = AuthFile {
            profiles: profiles.to_vec(),
        };
        fs::write(
            self.paths.auth_profiles(),
            toml::to_string_pretty(&file).map_err(|e| LateError::Config(e.to_string()))?,
        )?;
        Ok(())
    }

    pub fn upsert_auth(&self, mut profile: AuthProfile) -> Result<AuthProfile> {
        if profile.id.is_empty() {
            profile.id = uuid::Uuid::new_v4().to_string();
        }
        let mut profiles = self.load_auth()?;
        if let Some(existing) = profiles.iter_mut().find(|p| p.id == profile.id) {
            *existing = profile.clone();
        } else {
            profiles.push(profile.clone());
        }
        self.save_auth(&profiles)?;
        Ok(profile)
    }

    pub fn delete_auth(&self, id: &str) -> Result<()> {
        let mut profiles = self.load_auth()?;
        profiles.retain(|p| p.id != id);
        self.save_auth(&profiles)
    }

    pub fn get_auth(&self, id: &str) -> Result<AuthProfile> {
        self.load_auth()?
            .into_iter()
            .find(|p| p.id == id)
            .ok_or_else(|| LateError::NotFound(format!("auth profile {id}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DeviceKind;

    #[test]
    fn user_config_ssh_device_resolves_auth_if_present() {
        let paths = crate::config::LatePaths::discover();
        if !paths.inventory().exists() {
            return;
        }
        let store = InventoryStore::new(paths.clone());
        let inv = store.load().expect("inventory.toml must deserialize");
        let ssh = inv.devices.iter().find(|d| d.kind == DeviceKind::Ssh);
        let Some(ssh) = ssh else { return };
        let pid = ssh
            .auth_profile_id
            .as_deref()
            .expect("saved SSH device must keep auth_profile_id");
        store
            .get_auth(pid)
            .expect("pcap.remote.start must find the auth profile on disk");
        let serial = inv.devices.iter().find(|d| d.kind == DeviceKind::Serial);
        if let Some(serial) = serial {
            assert!(
                serial.serial_path.as_deref().unwrap_or("").starts_with("/dev/"),
                "serial device must keep its TTY path"
            );
        }
    }
}
