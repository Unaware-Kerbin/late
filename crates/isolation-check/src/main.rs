use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let root = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let sidecar = root.join("apps/agent-sidecar");
    if !sidecar.exists() {
        println!("isolation-check: apps/agent-sidecar not present; OK");
        return ExitCode::SUCCESS;
    }
    let mut hits = Vec::new();
    walk(&sidecar, &mut hits);
    if hits.is_empty() {
        println!("isolation-check: OK");
        ExitCode::SUCCESS
    } else {
        eprintln!("FAIL: agent-sidecar must not reference keyring/russh/secret");
        for h in hits {
            eprintln!("  {h}");
        }
        ExitCode::FAILURE
    }
}

fn walk(dir: &Path, hits: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if name == "node_modules" || name == "dist" || name == ".git" {
            continue;
        }
        if path.is_dir() {
            walk(&path, hits);
            continue;
        }
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        if !matches!(ext, "ts" | "js" | "mjs" | "cjs" | "json" | "tsx") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let is_tools = name == "tools.ts" || name == "tools.js";
        for (i, line) in text.lines().enumerate() {
            let lower = line.to_ascii_lowercase();
            if lower.contains("keyring") || lower.contains("russh") || lower.contains("secret") {
                hits.push(format!("{}:{}:{}", path.display(), i + 1, line.trim()));
            }
            if is_tools
                && (lower.contains("provider-keys")
                    || lower.contains("sidecar.token")
                    || lower.contains("providers.set")
                    || lower.contains("providers.status"))
            {
                hits.push(format!(
                    "{}:{}: agent tools must not touch provider credentials",
                    path.display(),
                    i + 1
                ));
            }
        }
    }
}
