use crate::config::LatePaths;
use crate::error::Result;
use crate::types::CaptureRecord;
use serde::{Deserialize, Serialize};
use std::fs;
use similar::{ChangeTag, TextDiff};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CaptureStore {
    pub captures: Vec<CaptureRecord>,
}

impl CaptureStore {
    pub fn load(paths: &LatePaths) -> Result<Self> {
        let path = paths.data.join("captures.json");
        if !path.exists() {
            return Ok(Self::default());
        }
        Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
    }

    pub fn save(&self, paths: &LatePaths) -> Result<()> {
        fs::create_dir_all(&paths.data)?;
        fs::write(
            paths.data.join("captures.json"),
            serde_json::to_string_pretty(self)?,
        )?;
        Ok(())
    }

    pub fn add(&mut self, rec: CaptureRecord) {
        self.captures.insert(0, rec);
        self.captures.truncate(200);
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DiffLine {
    pub tag: String,
    pub text: String,
}

pub fn diff_outputs(a: &str, b: &str) -> Vec<DiffLine> {
    let diff = TextDiff::from_lines(a, b);
    let mut lines = Vec::new();
    for change in diff.iter_all_changes() {
        let tag = match change.tag() {
            ChangeTag::Delete => "del",
            ChangeTag::Insert => "ins",
            ChangeTag::Equal => "eq",
        };
        lines.push(DiffLine {
            tag: tag.into(),
            text: change.to_string(),
        });
    }
    lines
}

pub fn export_session(
    paths: &LatePaths,
    name: &str,
    body: &str,
    encrypt_passphrase: Option<&str>,
) -> Result<std::path::PathBuf> {
    fs::create_dir_all(paths.data.join("exports"))?;
    let path = paths.data.join("exports").join(format!("{name}.log"));
    if let Some(pass) = encrypt_passphrase {
        // age was omitted: passphrase export is XOR obfuscation with SHA-256(pass),
        // not real encryption. Use `age` CLI on the plaintext `.log` for secrecy.
        use sha2::{Digest, Sha256};
        let key = Sha256::digest(pass.as_bytes());
        let mut out = b"LATEXOR1".to_vec();
        for (i, b) in body.as_bytes().iter().enumerate() {
            out.push(b ^ key[i % 32]);
        }
        let dest = path.with_extension("log.xor");
        fs::write(&dest, out)?;
        Ok(dest)
    } else {
        fs::write(&path, body)?;
        Ok(path)
    }
}
