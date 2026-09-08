//! Intel XPU serve via OpenVINO GenAI (Level Zero GPU).
//!
//! Candle cannot use XPU. This worker is the Intel path behind the same
//! `late-infer` binary (loopback `/v1`). CPU is not the discrete Intel card.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};

const WORKER_PY: &str = include_str!("../python/ov_worker.py");

#[derive(Debug, Deserialize)]
struct WorkerMsg {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    device: Option<String>,
    #[serde(default)]
    device_name: Option<String>,
    #[serde(default)]
    pci: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    finish_reason: Option<String>,
}

pub struct OvSession {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    pub device_label: String,
}

impl Drop for OvSession {
    fn drop(&mut self) {
        let _ = self.stdin.write_all(b"{\"op\":\"quit\"}\n");
        let _ = self.stdin.flush();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn env_trim(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn late_data_dir() -> PathBuf {
    if let Some(p) = env_trim("XDG_DATA_HOME") {
        return PathBuf::from(p).join("late");
    }
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("late")
}

/// Python that can `import openvino_genai`. Empty / missing is not NVIDIA.
pub fn find_intel_python() -> Option<PathBuf> {
    let mut cands: Vec<PathBuf> = Vec::new();
    if let Some(p) = env_trim("LATE_INFER_INTEL_PYTHON") {
        cands.push(PathBuf::from(p));
    }
    let venv = late_data_dir().join("intel-ov").join("bin");
    cands.push(venv.join("python"));
    cands.push(venv.join("python3"));
    cands.push(venv.join("python.exe"));
    for name in ["python3.12", "python3", "python"] {
        if let Ok(path) = which(name) {
            cands.push(path);
        }
    }
    for p in cands {
        if python_has_openvino_genai(&p) {
            return Some(p);
        }
    }
    None
}

fn which(name: &str) -> Result<PathBuf, ()> {
    let path = std::env::var("PATH").map_err(|_| ())?;
    let sep = if cfg!(windows) { ';' } else { ':' };
    for dir in path.split(sep) {
        let p = Path::new(dir).join(name);
        if p.is_file() {
            return Ok(p);
        }
        if cfg!(windows) {
            let exe = Path::new(dir).join(format!("{name}.exe"));
            if exe.is_file() {
                return Ok(exe);
            }
        }
    }
    Err(())
}

fn python_has_openvino_genai(py: &Path) -> bool {
    if !py.is_file() {
        return false;
    }
    let out = Command::new(py)
        .args([
            "-c",
            "import openvino_genai, openvino; print('openvino-genai')",
        ])
        .output();
    match out {
        Ok(o) if o.status.success() => {
            String::from_utf8_lossy(&o.stdout).contains("openvino-genai")
        }
        _ => false,
    }
}

/// Live probe: OpenVINO GenAI importable. `LATE_INFER_INTEL_RUNTIME=0|1` overrides for tests.
pub fn intel_runtime_ok() -> bool {
    match env_trim("LATE_INFER_INTEL_RUNTIME")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "0" | "false" | "no" => false,
        "1" | "true" | "yes" => true,
        _ => find_intel_python().is_some(),
    }
}

pub fn runtime_missing_msg() -> String {
    "late-infer cannot use the Intel GPU on your computer (this engine needs OpenVINO GenAI / Level Zero, not CUDA). Start will not run on CPU and call it that card.".into()
}

fn worker_script_path() -> Result<PathBuf> {
    let dir = late_data_dir().join("intel-ov");
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let path = dir.join("ov_worker.py");
    std::fs::write(&path, WORKER_PY).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

impl OvSession {
    pub fn start_and_load(model_id: &str, model_dir: &Path, ov_dir: &Path) -> Result<Self> {
        if !intel_runtime_ok() {
            bail!("{}", runtime_missing_msg());
        }
        let py = find_intel_python().ok_or_else(|| anyhow::anyhow!(runtime_missing_msg()))?;
        let script = worker_script_path()?;
        let mut child = Command::new(&py)
            .arg(&script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| format!("spawn OpenVINO worker {}", py.display()))?;
        let stdin = child.stdin.take().context("worker stdin")?;
        let stdout = child.stdout.take().context("worker stdout")?;
        let mut session = Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            device_label: "intel-xpu:openvino-genai".into(),
        };
        let pci = env_trim("LATE_INFER_PCI");
        let probe = session.roundtrip(&serde_json::json!({
            "op": "probe",
            "pci": pci,
        }))?;
        if !probe.ok {
            bail!(
                "{}",
                probe.error.unwrap_or_else(|| crate::host_ram::HOST_RAM_VS_VRAM.into())
            );
        }
        let req = serde_json::json!({
            "op": "load",
            "model_id": model_id,
            "model_dir": model_dir,
            "ov_dir": ov_dir,
            "pci": pci,
        });
        let msg = session.roundtrip(&req)?;
        if !msg.ok {
            bail!(
                "{}",
                msg.error.unwrap_or_else(|| "OpenVINO load failed".into())
            );
        }
        let mut label = format!(
            "intel-xpu:openvino-genai ({})",
            msg.device_name.as_deref().or(msg.device.as_deref()).unwrap_or("GPU")
        );
        if let Some(pci) = msg.pci.as_deref().filter(|s| !s.is_empty()) {
            label.push_str(" · ");
            label.push_str(pci);
        }
        if msg.kind.as_deref() != Some("openvino-genai") && msg.kind.is_some() {
            tracing::warn!("intel worker kind={:?}", msg.kind);
        }
        session.device_label = label;
        Ok(session)
    }

    fn roundtrip(&mut self, req: &serde_json::Value) -> Result<WorkerMsg> {
        let line = serde_json::to_string(req)?;
        self.stdin
            .write_all(line.as_bytes())
            .context("write worker")?;
        self.stdin.write_all(b"\n").context("write worker nl")?;
        self.stdin.flush().context("flush worker")?;
        let mut buf = String::new();
        let n = self.stdout.read_line(&mut buf).context("read worker")?;
        if n == 0 {
            bail!("OpenVINO worker closed stdout (Intel GPU serve failed; not falling back to CPU)");
        }
        serde_json::from_str(buf.trim()).with_context(|| format!("parse worker line {buf:?}"))
    }

    /// Compile-only / Convert: write OpenVINO IR under `ov_dir`. Does not bind :8010.
    pub fn export_ir(model_id: &str, model_dir: &Path, ov_dir: &Path) -> Result<()> {
        if !intel_runtime_ok() {
            bail!("{}", runtime_missing_msg());
        }
        let py = find_intel_python().ok_or_else(|| anyhow::anyhow!(runtime_missing_msg()))?;
        let script = worker_script_path()?;
        let mut child = Command::new(&py)
            .arg(&script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| format!("spawn OpenVINO worker {}", py.display()))?;
        let stdin = child.stdin.take().context("worker stdin")?;
        let stdout = child.stdout.take().context("worker stdout")?;
        let mut session = Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            device_label: "intel-xpu:openvino-genai".into(),
        };
        let pci = env_trim("LATE_INFER_PCI");
        let probe = session.roundtrip(&serde_json::json!({
            "op": "probe",
            "pci": pci,
        }))?;
        if !probe.ok {
            bail!(
                "{}",
                probe.error.unwrap_or_else(|| crate::host_ram::HOST_RAM_VS_VRAM.into())
            );
        }
        let msg = session.roundtrip(&serde_json::json!({
            "op": "convert",
            "model_id": model_id,
            "model_dir": model_dir,
            "ov_dir": ov_dir,
            "pci": pci,
        }))?;
        if !msg.ok {
            bail!(
                "{}",
                msg.error.unwrap_or_else(|| "OpenVINO IR export failed".into())
            );
        }
        let _ = session.stdin.write_all(b"{\"op\":\"quit\"}\n");
        let _ = session.stdin.flush();
        let _ = session.child.wait();
        if !ov_dir.join("openvino_model.xml").is_file()
            && !ov_dir
                .read_dir()
                .map(|rd| {
                    rd.filter_map(|e| e.ok())
                        .any(|e| e.path().extension().and_then(|s| s.to_str()) == Some("xml"))
                })
                .unwrap_or(false)
        {
            bail!("OpenVINO IR is missing after convert. late-infer will not load Candle CPU as that Intel GPU.");
        }
        Ok(())
    }

    pub fn generate(
        &mut self,
        prompt: &str,
        max_tokens: usize,
        temperature: Option<f64>,
        top_p: Option<f64>,
    ) -> Result<(String, String)> {
        let req = serde_json::json!({
            "op": "generate",
            "prompt": prompt,
            "max_tokens": max_tokens,
            "temperature": temperature,
            "top_p": top_p,
        });
        let msg = self.roundtrip(&req)?;
        if !msg.ok {
            bail!(
                "{}",
                msg.error.unwrap_or_else(|| "OpenVINO generate failed".into())
            );
        }
        Ok((
            msg.text.unwrap_or_default(),
            msg.finish_reason.unwrap_or_else(|| "stop".into()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_env_override_false() {
        let prev = std::env::var("LATE_INFER_INTEL_RUNTIME").ok();
        std::env::set_var("LATE_INFER_INTEL_RUNTIME", "0");
        assert!(!intel_runtime_ok());
        match prev {
            Some(v) => std::env::set_var("LATE_INFER_INTEL_RUNTIME", v),
            None => std::env::remove_var("LATE_INFER_INTEL_RUNTIME"),
        }
    }

    #[test]
    fn runtime_env_override_true() {
        let prev = std::env::var("LATE_INFER_INTEL_RUNTIME").ok();
        std::env::set_var("LATE_INFER_INTEL_RUNTIME", "1");
        assert!(intel_runtime_ok());
        match prev {
            Some(v) => std::env::set_var("LATE_INFER_INTEL_RUNTIME", v),
            None => std::env::remove_var("LATE_INFER_INTEL_RUNTIME"),
        }
    }

    #[test]
    fn missing_msg_is_not_cuda() {
        let s = runtime_missing_msg();
        assert!(s.contains("Intel"), "{s}");
        assert!(!s.to_ascii_lowercase().contains("cuda:0"), "{s}");
        assert!(!s.contains("NVIDIA GPU 0"), "{s}");
    }
}
