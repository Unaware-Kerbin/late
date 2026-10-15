use crate::config::LatePaths;
use crate::error::Result;
use crate::types::CaptureRecord;
use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};
use std::fs;

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
        crate::fsutil::write_private(
            &paths.data.join("captures.json"),
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

/// Cap diff payload so capture.diff cannot push multi‑MiB JSON into the renderer.
const DIFF_MAX_LINES: usize = 4_000;
const DIFF_MAX_LINE_CHARS: usize = 2_000;

pub fn diff_outputs(a: &str, b: &str) -> Vec<DiffLine> {
    let diff = TextDiff::from_lines(a, b);
    let mut lines = Vec::new();
    for change in diff.iter_all_changes() {
        if lines.len() >= DIFF_MAX_LINES {
            lines.push(DiffLine {
                tag: "eq".into(),
                text: format!("…[diff truncated after {DIFF_MAX_LINES} lines]"),
            });
            break;
        }
        let tag = match change.tag() {
            ChangeTag::Delete => "del",
            ChangeTag::Insert => "ins",
            ChangeTag::Equal => "eq",
        };
        let mut text = change.to_string();
        if text.len() > DIFF_MAX_LINE_CHARS {
            let omitted = text.len() - DIFF_MAX_LINE_CHARS;
            let mut end = DIFF_MAX_LINE_CHARS;
            while end > 0 && !text.is_char_boundary(end) {
                end -= 1;
            }
            text = format!("{}…[truncated {omitted} chars]", &text[..end]);
        }
        lines.push(DiffLine {
            tag: tag.into(),
            text,
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
    let stem = crate::confine::safe_export_stem(name)?;
    let dir = paths.data.join("exports");
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{stem}.log"));
    crate::confine::confine_under_roots(&path, &[dir.clone()], false)?;
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
        crate::fsutil::write_private(&dest, out)?;
        Ok(dest)
    } else {
        crate::fsutil::write_private(&path, body)?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_outputs_caps_line_count() {
        let a: String = (0..6_000).map(|i| format!("line-{i}-aaaa\n")).collect();
        let b: String = (0..6_000).map(|i| format!("line-{i}-bbbb\n")).collect();
        let lines = diff_outputs(&a, &b);
        assert!(lines.len() <= DIFF_MAX_LINES + 1);
        assert!(
            lines
                .last()
                .map(|l| l.text.contains("truncated"))
                .unwrap_or(false),
            "expected truncation marker"
        );
    }

    #[test]
    fn diff_outputs_caps_line_chars() {
        let long = "x".repeat(DIFF_MAX_LINE_CHARS + 500);
        let lines = diff_outputs(&long, "y");
        assert!(lines.iter().all(|l| l.text.len() < DIFF_MAX_LINE_CHARS + 80));
    }
}
