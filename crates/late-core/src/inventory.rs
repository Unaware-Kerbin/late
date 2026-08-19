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

fn normalize_folder(raw: &str) -> String {
    raw.split('/')
        .map(str::trim)
        .filter(|p| !p.is_empty() && *p != "." && *p != "..")
        .collect::<Vec<_>>()
        .join("/")
}

/// True when `path` is `ancestor` or a descendant (`ancestor/...`).
/// Uses a trailing-slash prefix so "NY" does not match "NYC".
fn is_under(path: &str, ancestor: &str) -> bool {
    path == ancestor || path.starts_with(&format!("{ancestor}/"))
}

fn register_folder(inv: &mut Inventory, path: Option<&str>) {
    let Some(path) = path.map(normalize_folder).filter(|s| !s.is_empty()) else {
        return;
    };
    let mut acc = String::new();
    for part in path.split('/') {
        if !acc.is_empty() {
            acc.push('/');
        }
        acc.push_str(part);
        if !inv.folders.iter().any(|f| f == &acc) {
            inv.folders.push(acc.clone());
        }
    }
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
        let mut inv: Inventory = toml::from_str(&raw)?;
        // Additive: keep empty folders, fill in ancestors, and list every device folder.
        let listed: Vec<String> = inv.folders.clone();
        for folder in listed {
            register_folder(&mut inv, Some(&folder));
        }
        let device_folders: Vec<Option<String>> =
            inv.devices.iter().map(|d| d.folder.clone()).collect();
        for folder in device_folders {
            register_folder(&mut inv, folder.as_deref());
        }
        Ok(inv)
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
        device.folder = device
            .folder
            .take()
            .map(|f| normalize_folder(&f))
            .filter(|f| !f.is_empty());
        let mut inv = self.load()?;
        if let Some(existing) = inv.devices.iter_mut().find(|d| d.id == device.id) {
            *existing = device.clone();
        } else {
            inv.devices.push(device.clone());
        }
        register_folder(&mut inv, device.folder.as_deref());
        self.save(&inv)?;
        Ok(device)
    }

    pub fn upsert_folder(&self, path: &str) -> Result<Inventory> {
        if normalize_folder(path).is_empty() {
            return Err(LateError::Config("folder name is empty".into()));
        }
        let mut inv = self.load()?;
        register_folder(&mut inv, Some(path));
        self.save(&inv)?;
        Ok(inv)
    }

    pub fn rename_folder(&self, from: &str, to: &str) -> Result<Inventory> {
        let from = normalize_folder(from);
        let to = normalize_folder(to);
        if from.is_empty() {
            return Err(LateError::Config("cannot rename the root group".into()));
        }
        if to.is_empty() {
            return Err(LateError::Config("new folder name is empty".into()));
        }
        if to == from {
            return self.load();
        }
        if is_under(&to, &from) {
            return Err(LateError::Config(
                "cannot rename a folder into itself".into(),
            ));
        }
        let mut inv = self.load()?;
        let conflict = inv
            .folders
            .iter()
            .map(|s| s.as_str())
            .chain(inv.devices.iter().filter_map(|d| d.folder.as_deref()))
            .any(|p| is_under(p, &to) && !is_under(p, &from));
        if conflict {
            return Err(LateError::Config(format!("folder {to} already exists")));
        }
        let prefix = format!("{from}/");
        for d in &mut inv.devices {
            let Some(cur) = d.folder.as_deref() else { continue };
            if cur == from {
                d.folder = Some(to.clone());
            } else if let Some(rest) = cur.strip_prefix(&prefix) {
                d.folder = Some(format!("{to}/{rest}"));
            }
        }
        inv.folders = inv
            .folders
            .into_iter()
            .filter_map(|f| {
                if f == from {
                    Some(to.clone())
                } else if let Some(rest) = f.strip_prefix(&prefix) {
                    Some(format!("{to}/{rest}"))
                } else {
                    Some(f)
                }
            })
            .collect();
        register_folder(&mut inv, Some(&to));
        inv.folders.sort();
        inv.folders.dedup();
        self.save(&inv)?;
        Ok(inv)
    }

    pub fn delete_folder(&self, path: &str) -> Result<Inventory> {
        let path = normalize_folder(path);
        if path.is_empty() {
            return Err(LateError::Config("cannot delete the root group".into()));
        }
        let parent = path.rsplit_once('/').map(|(p, _)| p.to_string());
        let mut inv = self.load()?;
        for d in &mut inv.devices {
            let Some(cur) = d.folder.as_deref() else { continue };
            if is_under(cur, &path) {
                d.folder = parent.clone();
            }
        }
        inv.folders.retain(|f| !is_under(f, &path));
        self.save(&inv)?;
        Ok(inv)
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
    use crate::types::{DeviceKind, Vendor};
    use std::ops::Deref;

    struct IsolatedStore {
        store: InventoryStore,
        dir: std::path::PathBuf,
    }

    impl IsolatedStore {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("late-inv-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&dir).unwrap();
            Self {
                store: InventoryStore::new(crate::config::LatePaths {
                    config: dir.clone(),
                    data: dir.clone(),
                }),
                dir,
            }
        }
    }

    impl Drop for IsolatedStore {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    impl Deref for IsolatedStore {
        type Target = InventoryStore;
        fn deref(&self) -> &Self::Target {
            &self.store
        }
    }

    fn ssh(name: &str, folder: &str) -> Device {
        let mut d = Device::new_ssh(name, "192.0.2.10", Vendor::AosCx);
        d.folder = Some(folder.into());
        d
    }

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

    #[test]
    fn nested_site_folders_roundtrip() {
        let store = IsolatedStore::new();
        store.upsert_device(ssh("edge-sw1", " Sites / NYC / Core ")).unwrap();
        store.upsert_folder("Sites/NYC/Access").unwrap();
        let inv = store.load().unwrap();
        assert!(inv.folders.iter().any(|f| f == "Sites"));
        assert!(inv.folders.iter().any(|f| f == "Sites/NYC"));
        assert!(inv.folders.iter().any(|f| f == "Sites/NYC/Core"));
        assert!(inv.folders.iter().any(|f| f == "Sites/NYC/Access"));
        assert_eq!(inv.devices[0].folder.as_deref(), Some("Sites/NYC/Core"));

        store.rename_folder("Sites/NYC", "Sites/Boston").unwrap();
        let inv = store.load().unwrap();
        assert_eq!(inv.devices[0].folder.as_deref(), Some("Sites/Boston/Core"));
        assert!(inv.folders.iter().any(|f| f == "Sites/Boston/Access"));

        store.delete_folder("Sites/Boston/Core").unwrap();
        let inv = store.load().unwrap();
        assert_eq!(inv.devices[0].folder.as_deref(), Some("Sites/Boston"));
        assert!(!inv.folders.iter().any(|f| f == "Sites/Boston/Core"));
    }

    #[test]
    fn normalize_drops_empty_dot_and_dotdot_segments() {
        assert_eq!(normalize_folder(""), "");
        assert_eq!(normalize_folder("/"), "");
        assert_eq!(normalize_folder("//"), "");
        assert_eq!(normalize_folder("  "), "");
        assert_eq!(normalize_folder("./."), "");
        assert_eq!(normalize_folder("Sites//NYC/"), "Sites/NYC");
        assert_eq!(normalize_folder("/Sites/NYC/"), "Sites/NYC");
        assert_eq!(normalize_folder(" Sites / NYC / Core "), "Sites/NYC/Core");
        assert_eq!(normalize_folder("Sites/./NYC/../Core"), "Sites/NYC/Core");
    }

    #[test]
    fn empty_folder_names_are_rejected() {
        let store = IsolatedStore::new();
        for raw in ["", "/", "//", "  ", "./.", ".."] {
            assert!(store.upsert_folder(raw).is_err(), "upsert {raw:?}");
            assert!(store.rename_folder("Sites", raw).is_err(), "rename to {raw:?}");
            assert!(store.delete_folder(raw).is_err(), "delete {raw:?}");
        }
        assert!(store.rename_folder("/", "Sites").is_err());
    }

    #[test]
    fn upsert_folder_then_list_shows_empty_tree_nodes() {
        let store = IsolatedStore::new();
        let inv = store.upsert_folder("Sites/NYC/Core").unwrap();
        assert!(inv.devices.is_empty());
        assert_eq!(
            inv.folders,
            vec![
                "Sites".to_string(),
                "Sites/NYC".to_string(),
                "Sites/NYC/Core".to_string()
            ]
        );
        let listed = store.load().unwrap();
        assert_eq!(listed.folders, inv.folders);
        assert!(listed.devices.is_empty());
    }

    #[test]
    fn upsert_device_does_not_wipe_empty_folders() {
        let store = IsolatedStore::new();
        store.upsert_folder("Sites/NYC/Access").unwrap();
        store.upsert_device(ssh("core-sw", "Sites/NYC/Core")).unwrap();
        let inv = store.load().unwrap();
        assert!(inv.folders.iter().any(|f| f == "Sites/NYC/Access"));
        assert!(inv.folders.iter().any(|f| f == "Sites/NYC/Core"));
    }

    #[test]
    fn deleting_last_device_keeps_empty_folder() {
        let store = IsolatedStore::new();
        let saved = store.upsert_device(ssh("only", "Sites/NYC")).unwrap();
        store.delete_device(&saved.id).unwrap();
        let inv = store.load().unwrap();
        assert!(inv.devices.is_empty());
        assert!(inv.folders.iter().any(|f| f == "Sites"));
        assert!(inv.folders.iter().any(|f| f == "Sites/NYC"));
    }

    #[test]
    fn prefix_ny_does_not_match_nyc() {
        let store = IsolatedStore::new();
        store.upsert_folder("NY").unwrap();
        store.upsert_device(ssh("nyc-sw", "NYC")).unwrap();
        store.rename_folder("NY", "Boston").unwrap();
        let inv = store.load().unwrap();
        assert!(inv.folders.iter().any(|f| f == "Boston"));
        assert!(inv.folders.iter().any(|f| f == "NYC"));
        assert!(!inv.folders.iter().any(|f| f == "NY"));
        assert_eq!(inv.devices[0].folder.as_deref(), Some("NYC"));

        store.delete_folder("Boston").unwrap();
        let inv = store.load().unwrap();
        assert!(inv.folders.iter().any(|f| f == "NYC"));
        assert_eq!(inv.devices[0].folder.as_deref(), Some("NYC"));
    }

    #[test]
    fn rename_rejects_collision_with_existing_folder() {
        let store = IsolatedStore::new();
        store.upsert_folder("Sites/NYC").unwrap();
        store.upsert_folder("Sites/Boston").unwrap();
        let err = store.rename_folder("Sites/NYC", "Sites/Boston").unwrap_err();
        assert!(
            err.to_string().contains("already exists"),
            "unexpected error: {err}"
        );
        let inv = store.load().unwrap();
        assert!(inv.folders.iter().any(|f| f == "Sites/NYC"));
        assert!(inv.folders.iter().any(|f| f == "Sites/Boston"));
    }

    #[test]
    fn rename_rejects_move_into_descendant() {
        let store = IsolatedStore::new();
        store.upsert_folder("Sites/NYC/Core").unwrap();
        let err = store.rename_folder("Sites", "Sites/NYC").unwrap_err();
        assert!(
            err.to_string().contains("into itself"),
            "unexpected error: {err}"
        );
        let inv = store.load().unwrap();
        assert!(inv.folders.iter().any(|f| f == "Sites/NYC/Core"));
    }

    #[test]
    fn load_hydrates_folders_from_devices_without_dropping_empties() {
        let store = IsolatedStore::new();
        let toml = r#"
folders = ["Empty/Site"]

[[devices]]
id = "d1"
name = "sw"
kind = "ssh"
vendor = "aos-cx"
host = "192.0.2.10"
folder = "Sites/NYC"
"#;
        fs::write(store.dir.join("inventory.toml"), toml).unwrap();
        let inv = store.load().unwrap();
        assert!(inv.folders.iter().any(|f| f == "Empty/Site"));
        assert!(inv.folders.iter().any(|f| f == "Empty"));
        assert!(inv.folders.iter().any(|f| f == "Sites"));
        assert!(inv.folders.iter().any(|f| f == "Sites/NYC"));
    }
}
