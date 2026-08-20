use crate::config::LatePaths;
use crate::error::{LateError, Result};
use crate::types::{CollectionsFile, CommandCollection};
use std::fs;

pub fn load(paths: &LatePaths) -> Result<CollectionsFile> {
    let path = paths.collections();
    if !path.exists() {
        return Ok(CollectionsFile::default());
    }
    let raw = fs::read_to_string(path)?;
    Ok(toml::from_str(&raw)?)
}

pub fn save(paths: &LatePaths, file: &CollectionsFile) -> Result<()> {
    crate::fsutil::write_private(
        &paths.collections(),
        toml::to_string_pretty(file).map_err(|e| LateError::Config(e.to_string()))?,
    )?;
    Ok(())
}

pub fn upsert(paths: &LatePaths, col: CommandCollection) -> Result<CommandCollection> {
    let mut file = load(paths)?;
    if let Some(existing) = file.collections.iter_mut().find(|c| c.id == col.id) {
        *existing = col.clone();
    } else {
        file.collections.push(col.clone());
    }
    save(paths, &file)?;
    Ok(col)
}

pub fn delete(paths: &LatePaths, id: &str) -> Result<()> {
    let mut file = load(paths)?;
    file.collections.retain(|c| c.id != id);
    save(paths, &file)
}
