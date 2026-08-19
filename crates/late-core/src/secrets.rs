use crate::config::LatePaths;
use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use zeroize::Zeroize;

#[derive(Debug, Default, Serialize, Deserialize)]
struct FileVault {
    secrets: HashMap<String, String>,
}

/// File-backed secret store (mode 0600). UI never reads values back after write.
#[derive(Clone)]
pub struct SecretStore {
    paths: LatePaths,
}

impl SecretStore {
    pub fn new(paths: LatePaths) -> Self {
        Self { paths }
    }

    pub fn set(&self, profile_id: &str, secret: &str) -> Result<()> {
        let mut vault = self.load_vault()?;
        vault
            .secrets
            .insert(profile_id.to_string(), secret.to_string());
        self.save_vault(&vault)
    }

    pub fn get(&self, profile_id: &str) -> Result<Option<String>> {
        Ok(self.load_vault()?.secrets.get(profile_id).cloned())
    }

    pub fn delete(&self, profile_id: &str) -> Result<()> {
        let mut vault = self.load_vault()?;
        if let Some(mut v) = vault.secrets.remove(profile_id) {
            v.zeroize();
        }
        self.save_vault(&vault)
    }

    fn load_vault(&self) -> Result<FileVault> {
        let path = self.paths.secrets_file();
        if !path.exists() {
            return Ok(FileVault::default());
        }
        let raw = fs::read_to_string(path)?;
        Ok(serde_json::from_str(&raw).unwrap_or_default())
    }

    fn save_vault(&self, vault: &FileVault) -> Result<()> {
        let path = self.paths.secrets_file();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, serde_json::to_string_pretty(vault)?)?;
        let mut perms = fs::metadata(&path)?.permissions();
        perms.set_mode(0o600);
        fs::set_permissions(&path, perms)?;
        Ok(())
    }
}

