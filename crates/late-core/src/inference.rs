//! Local vLLM lifecycle. Operator-initiated only — never an agent tool.
//! Docker compose runs on a background thread so serial/SSH stay live.

use crate::error::{LateError, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

const DEFAULT_IMAGE: &str = "intel/llm-scaler-vllm:0.21.0-b3";
#[allow(dead_code)]
const DEFAULT_SERVE: &str = "Qwen/Qwen3-8B";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalModel {
    pub id: String,
    pub complete: bool,
    pub size_bytes: u64,
    pub note: String,
    #[serde(default)]
    pub recommended: bool,
    #[serde(default)]
    pub newest: bool,
    #[serde(default)]
    pub tp: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceStatus {
    pub running: bool,
    pub starting: bool,
    pub downloading: bool,
    pub models: Vec<String>,
    pub local_models: Vec<LocalModel>,
    pub container: Option<String>,
    pub download_id: Option<String>,
    /// Hugging Face id last requested by Start (not the served API name `local`).
    pub serve_model: Option<String>,
    pub gpu: crate::hardware::GpuProfile,
    pub detail: String,
    /// True when Start/Download may run the optional Intel XPU compose (discrete Intel or LATE_VLLM_FORCE=1).
    #[serde(default)]
    pub allow_intel_compose: bool,
    /// True when Late started this process (llama-server child / verified pid).
    #[serde(default)]
    pub late_owned: bool,
    #[serde(default)]
    pub gpu_launch: Option<crate::hardware::GpuLaunchPlan>,
    /// Docker CLI + engine reachable (needed for vLLM Start / Download).
    #[serde(default)]
    pub docker_available: bool,
}

#[derive(Default)]
struct Job {
    starting: bool,
    stopping: bool,
    downloading: bool,
    download_id: Option<String>,
    serve_model: Option<String>,
    last_error: Option<String>,
}

fn job() -> &'static Mutex<Job> {
    static JOB: OnceLock<Mutex<Job>> = OnceLock::new();
    JOB.get_or_init(|| Mutex::new(Job::default()))
}

fn compose_candidate(root: &Path) -> bool {
    root.join("docker/compose.yml").is_file()
}

fn bundled_resources_root() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("LATE_ROOT") {
        let p = PathBuf::from(p);
        if compose_candidate(&p) {
            return Some(p);
        }
    }
    if let Ok(p) = std::env::var("LATE_BUNDLE_BIN") {
        let p = PathBuf::from(p);
        if let Some(parent) = p.parent() {
            if compose_candidate(parent) {
                return Some(parent.to_path_buf());
            }
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(bin_dir) = exe.parent() {
            if let Some(res) = bin_dir.parent() {
                if compose_candidate(res) {
                    return Some(res.to_path_buf());
                }
            }
        }
    }
    None
}

fn repo_root() -> Result<PathBuf> {
    if let Some(p) = bundled_resources_root() {
        return Ok(p);
    }
    let here = std::env::current_dir()?;
    if compose_candidate(&here) {
        return Ok(here);
    }
    Err(LateError::Message(
        "cannot find docker/compose.yml; Late packages it in resources/docker, or set LATE_ROOT".into(),
    ))
}

/// Docker CLI is on PATH and the engine answers `docker info`.
pub fn docker_available() -> bool {
    match Command::new("docker")
        .args(["info", "--format", "{{.ServerVersion}}"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
    {
        Ok(out) => {
            out.status.success() && !String::from_utf8_lossy(&out.stdout).trim().is_empty()
        }
        Err(_) => false,
    }
}

/// vLLM Start/Download: Intel compose path and Docker both required.
pub fn vllm_start_allowed(allow_intel_compose: bool, docker_ok: bool) -> bool {
    allow_intel_compose && docker_ok
}

fn compose_file() -> Result<PathBuf> {
    let root = repo_root()?.canonicalize()?;
    let file = root.join("docker/compose.yml").canonicalize()?;
    if !file.starts_with(&root) {
        return Err(LateError::Message("compose path escaped repo root".into()));
    }
    Ok(file)
}

fn docker_dir() -> Result<PathBuf> {
    Ok(compose_file()?
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf())
}

fn read_env_key(path: &Path, key: &str) -> Option<String> {
    let raw = fs::read_to_string(path).ok()?;
    for line in raw.lines() {
        if let Some(v) = line.strip_prefix(&format!("{key}=")) {
            let v = v.trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

fn hf_home() -> PathBuf {
    if let Ok(p) = std::env::var("HF_HOME") {
        return PathBuf::from(p);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cache/huggingface")
}

pub fn validate_model(model: &str) -> Result<()> {
    if model.is_empty() || model.len() > 200 {
        return Err(LateError::Message("invalid model id".into()));
    }
    if !model
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-:".contains(c))
    {
        return Err(LateError::Message(
            "model id contains illegal characters".into(),
        ));
    }
    Ok(())
}

pub fn status() -> InferenceStatus {
    status_with(true)
}

pub fn status_with(use_all_gpus: bool) -> InferenceStatus {
    let (starting, stopping, downloading, download_id, serve_model, last_error) = {
        let j = job().lock().unwrap_or_else(|e| e.into_inner());
        (
            j.starting,
            j.stopping,
            j.downloading,
            j.download_id.clone(),
            j.serve_model.clone(),
            j.last_error.clone(),
        )
    };
    let serve_model =
        serve_model.or_else(|| read_env_key(&docker_dir().ok()?.join(".env"), "MODEL_PATH"));
    let models = probe_models();
    let gpu = crate::hardware::probe();
    let gpu_launch = crate::hardware::launch_plan(&gpu, use_all_gpus);
    let local_models = list_local_models_with(&gpu, use_all_gpus);
    let container = docker_status();
    let serving = !models.is_empty();
    let running = serving || container.as_deref() == Some("running") || starting;
    let detail = if let Some(err) = last_error {
        err
    } else if downloading {
        let id = download_id.clone().unwrap_or_else(|| "model".into());
        format!(
            "downloading {id} into the Hugging Face cache — {}",
            download_log_tail().unwrap_or_else(|| "starting hf download…".into())
        )
    } else if starting && !serving {
        match serve_model.as_deref() {
            Some(id) => format!("starting {id} — SSH/serial stay connected"),
            None => "starting GPU container in the background — SSH/serial stay connected".into(),
        }
    } else if stopping {
        "stopping container".into()
    } else if serving {
        format!("serving {}", models.join(", "))
    } else if container.as_deref() == Some("running") {
        "container is up; model still loading (can take several minutes)".into()
    } else if matches!(container.as_deref(), Some("restarting") | Some("exited")) {
        format!(
            "container {}: {}",
            container.as_deref().unwrap_or("?"),
            docker_log_tail().unwrap_or_else(|| "no logs yet".into())
        )
    } else {
        container
            .as_deref()
            .map(|s| format!("container: {s}"))
            .unwrap_or_else(|| idle_inference_detail(&gpu))
    };
    let allow_intel_compose =
        force_intel_compose() || crate::hardware::allow_intel_xpu_compose(&gpu.vendor);
    let docker_ok = docker_available();
    InferenceStatus {
        running,
        starting,
        downloading,
        models,
        local_models,
        container,
        download_id,
        serve_model,
        gpu,
        detail,
        allow_intel_compose,
        late_owned: false,
        gpu_launch: Some(gpu_launch),
        docker_available: docker_ok,
    }
}

fn probe_models() -> Vec<String> {
    let addr: SocketAddr = "127.0.0.1:8000".parse().unwrap();
    if TcpStream::connect_timeout(&addr, Duration::from_millis(150)).is_err() {
        return vec![];
    }
    let body = Command::new("curl")
        .args(["-sf", "-m", "1", "http://127.0.0.1:8000/v1/models"])
        .output()
        .ok();
    let Some(body) = body else {
        return vec![];
    };
    if !body.status.success() {
        return vec![];
    }
    let v: serde_json::Value =
        serde_json::from_slice(&body.stdout).unwrap_or(serde_json::Value::Null);
    v.get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.get("id").and_then(|i| i.as_str()).map(|s| s.to_string()))
                .filter(|id| !id.contains(":cloud"))
                .collect()
        })
        .unwrap_or_default()
}

fn write_gid_overlay(dir: &Path) -> Result<PathBuf> {
    let video = host_gid("video").unwrap_or_else(|| "44".into());
    let render = host_gid("render").unwrap_or_else(|| video.clone());
    let mut yaml = String::from(
        "# Generated — unique host GIDs only (Docker rejects duplicates)\nservices:\n  vllm:\n    group_add:\n",
    );
    yaml.push_str(&format!("      - \"{video}\"\n"));
    if render != video {
        yaml.push_str(&format!("      - \"{render}\"\n"));
    }
    let path = dir.join("compose.gids.yml");
    fs::write(&path, yaml)?;
    Ok(path)
}

fn host_gid(group: &str) -> Option<String> {
    let out = Command::new("getent")
        .args(["group", group])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .split(':')
        .nth(2)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

fn docker_status() -> Option<String> {
    let out = Command::new("docker")
        .args(["inspect", "-f", "{{.State.Status}}", "late-vllm"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn docker_log_tail() -> Option<String> {
    let out = Command::new("docker")
        .args(["logs", "--tail", "80", "late-vllm"])
        .output()
        .ok()?;
    let mut text = String::new();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    text.push_str(&String::from_utf8_lossy(&out.stdout));
    text.lines()
        .rev()
        .find(|l| {
            let s = l.to_ascii_lowercase();
            s.contains("runtimeerror")
                || s.contains("oneccl")
                || s.contains("ze_fd_manager")
                || s.contains("opendir")
                || s.contains("device index")
                || s.contains("out of range")
                || s.contains("no such file")
                || s.contains("safetensors")
        })
        .map(|l| l.trim().chars().take(240).collect())
        .or_else(|| last_useful_line(&text))
}

fn download_log_path() -> Option<PathBuf> {
    docker_dir().ok().map(|d| d.join("hf-download.log"))
}

fn download_log_tail() -> Option<String> {
    let path = download_log_path()?;
    let raw = fs::read_to_string(path).ok()?;
    last_useful_line(&raw)
}

fn last_useful_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .last()
        .map(|l| l.chars().take(240).collect())
}

fn hub_dir_for(model: &str) -> PathBuf {
    hf_home()
        .join("hub")
        .join(format!("models--{}", model.replace('/', "--")))
}

fn snapshot_complete(model_dir: &Path) -> bool {
    let snaps = model_dir.join("snapshots");
    let Ok(entries) = fs::read_dir(&snaps) else {
        return false;
    };
    for snap in entries.flatten() {
        let p = snap.path();
        if !p.is_dir() {
            continue;
        }
        let has_config = p.join("config.json").exists();
        let has_weights = fs::read_dir(&p).ok().is_some_and(|rd| {
            rd.flatten().any(|f| {
                let n = f.file_name();
                let n = n.to_string_lossy();
                n.ends_with(".safetensors") || n.ends_with(".bin") || n.ends_with(".gguf")
            })
        });
        if has_config && has_weights {
            return true;
        }
    }
    false
}

/// Refuse tokenizer-only / incomplete Hub snapshots.
fn require_complete_model(model: &str) -> Result<()> {
    let hub = hub_dir_for(model);
    if hub.is_dir() && !snapshot_complete(&hub) {
        return Err(LateError::Message(format!(
            "{model} is not fully downloaded (incomplete download (config/tokenizer only)). Click Download first, or serve Qwen/Qwen3-8B"
        )));
    }
    if let Some(local) = list_local_models().iter().find(|m| m.id == model) {
        if !local.complete {
            return Err(LateError::Message(format!(
                "{model} is not fully downloaded ({}). Click Download first, or serve Qwen/Qwen3-8B",
                local.note
            )));
        }
    }
    Ok(())
}

fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    for e in walkdir::WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .flatten()
    {
        if e.file_type().is_file() {
            if let Ok(m) = e.metadata() {
                total = total.saturating_add(m.len());
            }
        }
    }
    total
}

fn scan_hf_cache() -> Vec<LocalModel> {
    let hub = hf_home().join("hub");
    let Ok(rd) = fs::read_dir(&hub) else {
        return vec![];
    };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("models--") {
            continue;
        }
        let id = name.trim_start_matches("models--").replace("--", "/");
        let path = e.path();
        let complete = snapshot_complete(&path);
        let size_bytes = dir_size(&path);
        let note = if complete {
            "ready in Hugging Face cache".into()
        } else {
            "incomplete download (config/tokenizer only)".into()
        };
            out.push(LocalModel {
                id,
                complete,
                size_bytes,
                note,
                recommended: false,
                newest: false,
                tp: 1,
            });
    }
    out.sort_by(|a, b| b.complete.cmp(&a.complete).then(a.id.cmp(&b.id)));
    out
}

pub fn list_local_models() -> Vec<LocalModel> {
    list_local_models_with(&crate::hardware::probe(), true)
}

fn list_local_models_with(gpu: &crate::hardware::GpuProfile, use_all: bool) -> Vec<LocalModel> {
    let gpu = crate::hardware::serving_profile(gpu, use_all);
    let recs = crate::hardware::recommend(&gpu);
    let mut out = scan_hf_cache();
    for m in &mut out {
        if let Err(why) = crate::hardware::fits(&m.id, &gpu) {
            m.recommended = false;
            m.note = why;
        }
    }
    for r in &recs {
        if let Some(m) = out.iter_mut().find(|m| m.id == r.id) {
            m.recommended = r.recommended;
            m.newest = r.newest;
            m.tp = r.tp;
            m.note = if m.complete {
                format!("{} · cached", r.reason)
            } else {
                r.reason.clone()
            };
        } else {
            out.push(LocalModel {
                id: r.id.clone(),
                complete: false,
                size_bytes: 0,
                note: r.reason.clone(),
                recommended: r.recommended,
                newest: r.newest,
                tp: r.tp,
            });
        }
    }
    out.sort_by(|a, b| {
        b.recommended
            .cmp(&a.recommended)
            .then(b.newest.cmp(&a.newest))
            .then(b.complete.cmp(&a.complete))
            .then(a.id.cmp(&b.id))
    });
    out
}

pub fn pick_default_model() -> String {
    let gpu = crate::hardware::probe();
    let recs = crate::hardware::recommend(&gpu);
    let local = scan_hf_cache();
    if let Some(r) = recs
        .iter()
        .filter(|r| r.recommended && r.newest && local.iter().any(|m| m.id == r.id && m.complete))
        .max_by_key(|r| r.weight_gb)
    {
        return r.id.clone();
    }
    crate::hardware::default_model(&gpu)
}

fn upsert_dotenv(path: &Path, updates: &[(&str, String)]) -> Result<()> {
    let mut lines: Vec<String> = if path.is_file() {
        fs::read_to_string(path)?
            .lines()
            .map(|l| l.to_string())
            .collect()
    } else {
        vec![]
    };
    for (k, v) in updates {
        let prefix = format!("{k}=");
        if let Some(line) = lines
            .iter_mut()
            .find(|l| l.starts_with(&prefix) && !l.trim_start().starts_with('#'))
        {
            *line = format!("{k}={v}");
        } else {
            lines.push(format!("{k}={v}"));
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut body = lines.join("\n");
    if !body.ends_with('\n') {
        body.push('\n');
    }
    fs::write(path, body)?;
    Ok(())
}

fn force_intel_compose() -> bool {
    std::env::var("LATE_VLLM_FORCE").ok().as_deref() == Some("1")
}

fn require_intel_compose(gpu: &crate::hardware::GpuProfile) -> Result<()> {
    if force_intel_compose() || crate::hardware::allow_intel_xpu_compose(&gpu.vendor) {
        return Ok(());
    }
    Err(LateError::Message(
        crate::hardware::intel_xpu_compose_refuse(&gpu.vendor),
    ))
}

fn idle_inference_detail(gpu: &crate::hardware::GpuProfile) -> String {
    if !docker_available() {
        return "Install Docker to use vLLM on this computer. Ollama and llama.cpp are included and do not need Docker.".into();
    }
    if crate::hardware::allow_intel_xpu_compose(&gpu.vendor) {
        return "optional Intel XPU compose is not running on 127.0.0.1:8000. Point local vLLM at that URL, or Start below if this machine has a discrete Intel GPU.".into();
    }
    crate::hardware::intel_xpu_compose_refuse(&gpu.vendor)
}

fn vllm_image() -> String {
    std::env::var("VLLM_IMAGE").unwrap_or_else(|_| DEFAULT_IMAGE.into())
}

pub fn start(model: &str) -> Result<InferenceStatus> {
    start_with(model, true)
}

pub fn start_with(model: &str, use_all_gpus: bool) -> Result<InferenceStatus> {
    let model = if model.is_empty() || model == "local" {
        pick_default_model()
    } else {
        model.to_string()
    };
    validate_model(&model)?;
    if !docker_available() {
        return Err(LateError::Message(
            "Install Docker to use vLLM on this computer. Ollama and llama.cpp are included and do not need Docker.".into(),
        ));
    }
    let gpu = crate::hardware::probe();
    require_intel_compose(&gpu)?;
    let serving = crate::hardware::serving_profile(&gpu, use_all_gpus);
    if let Err(why) = crate::hardware::fits(&model, &serving) {
        return Err(LateError::Message(why));
    }
    let tp = crate::hardware::launch_plan(&gpu, use_all_gpus)
        .tensor_parallel
        .max(1);
    require_complete_model(&model)?;
    let file = compose_file()?;
    {
        let mut j = job().lock().unwrap_or_else(|e| e.into_inner());
        if j.starting {
            return Ok(status_with(use_all_gpus));
        }
        j.starting = true;
        j.stopping = false;
        j.serve_model = Some(model.clone());
        j.last_error = None;
    }
    std::thread::Builder::new()
        .name("late-vllm-up".into())
        .spawn(move || {
            let dir = file.parent().unwrap_or(Path::new(".")).to_path_buf();
            let _ = Command::new("bash")
                .arg(dir.join("detect-gpus.sh"))
                .arg(dir.join(".env"))
                .status();
            let video_gid = host_gid("video").unwrap_or_else(|| "44".into());
            let render_gid = host_gid("render").unwrap_or_else(|| video_gid.clone());
            let env_file = dir.join(".env");
            let dtype = if model.to_ascii_uppercase().contains("FP8") {
                "auto"
            } else {
                "float16"
            };
            let tp_s = tp.to_string();
            let _ = upsert_dotenv(
                &env_file,
                &[
                    ("MODEL_PATH", model.clone()),
                    ("SERVED_NAME", "local".into()),
                    ("DTYPE", dtype.into()),
                    ("TP", tp_s.clone()),
                    ("VLLM_IMAGE", vllm_image()),
                    ("HF_HOME", hf_home().to_string_lossy().into_owned()),
                ],
            );
            // Later --env-file wins over docker/.env so detect-gpus cannot clobber the Hub id.
            let model_env = dir.join("model.env");
            let _ = fs::write(
                &model_env,
                format!("MODEL_PATH={model}\nSERVED_NAME=local\nDTYPE={dtype}\nTP={tp_s}\n"),
            );
            let gids_file = write_gid_overlay(&dir);
            let mut args = vec![
                "compose".into(),
                "-f".into(),
                file.to_string_lossy().into_owned(),
            ];
            if let Ok(ref gids) = gids_file {
                args.push("-f".into());
                args.push(gids.to_string_lossy().into_owned());
            }
            if env_file.is_file() {
                args.push("--env-file".into());
                args.push(env_file.to_string_lossy().into_owned());
            }
            if model_env.is_file() {
                args.push("--env-file".into());
                args.push(model_env.to_string_lossy().into_owned());
            }
            args.extend([
                "up".into(),
                "-d".into(),
                "--pull".into(),
                "missing".into(),
                "--force-recreate".into(),
            ]);
            let out = Command::new("docker")
                .current_dir(&dir)
                .env("MODEL_PATH", &model)
                .env("SERVED_NAME", "local")
                .env("DTYPE", dtype)
                .env("TP", tp.to_string())
                .env("HF_HOME", hf_home())
                .env("VIDEO_GID", video_gid)
                .env("RENDER_GID", render_gid)
                // Host mask skips the iGPU hole (e.g. 0,2). The image only lists
                // discrete XPUs as 0,1 — never inject ZE_AFFINITY_MASK.
                .env_remove("ZE_AFFINITY_MASK")
                .env_remove("DOCKER_ZE_AFFINITY_MASK")
                .args(&args)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output();
            let mut j = job().lock().unwrap_or_else(|e| e.into_inner());
            j.starting = false;
            match out {
                Ok(status) if status.status.success() => {
                    // compose -d returns before vLLM finishes init; catch fast GPU/mask crashes.
                    std::thread::sleep(std::time::Duration::from_secs(8));
                    let st = docker_status();
                    match st.as_deref() {
                        Some("exited") | Some("dead") | Some("restarting") => {
                            j.last_error = Some(format!(
                                "container {}: {}",
                                st.as_deref().unwrap_or("stopped"),
                                docker_log_tail().unwrap_or_else(|| "no logs".into())
                            ));
                        }
                        _ => {
                            j.last_error = None;
                        }
                    }
                }
                Ok(status) => {
                    let err = String::from_utf8_lossy(&status.stderr);
                    j.last_error = Some(format!(
                        "docker compose up failed: {}",
                        err.chars().take(400).collect::<String>()
                    ));
                }
                Err(e) => j.last_error = Some(format!("docker compose: {e}")),
            }
        })
        .map_err(|e| LateError::Message(format!("could not spawn GPU start thread: {e}")))?;
    Ok(status_with(use_all_gpus))
}

pub fn stop() -> Result<InferenceStatus> {
    let file = compose_file()?;
    {
        let mut j = job().lock().unwrap_or_else(|e| e.into_inner());
        j.stopping = true;
        j.starting = false;
        j.last_error = None;
    }
    std::thread::Builder::new()
        .name("late-vllm-down".into())
        .spawn(move || {
            let dir = file.parent().unwrap_or(Path::new(".")).to_path_buf();
            let _ = Command::new("docker")
                .current_dir(&dir)
                .args([
                    "compose",
                    "-f",
                    file.to_str().unwrap_or("compose.yml"),
                    "down",
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let mut j = job().lock().unwrap_or_else(|e| e.into_inner());
            j.stopping = false;
        })
        .map_err(|e| LateError::Message(format!("could not spawn GPU stop thread: {e}")))?;
    Ok(status())
}

/// Download a Hugging Face id into the host HF cache (same volume vLLM mounts).
/// The cache directory is often root-owned, so this uses the vLLM image as root.
pub fn download(model: &str) -> Result<InferenceStatus> {
    validate_model(model)?;
    if !docker_available() {
        return Err(LateError::Message(
            "Install Docker to use vLLM on this computer. Ollama and llama.cpp are included and do not need Docker.".into(),
        ));
    }
    require_intel_compose(&crate::hardware::probe())?;
    {
        let mut j = job().lock().unwrap_or_else(|e| e.into_inner());
        if j.downloading {
            return Ok(status());
        }
        j.downloading = true;
        j.download_id = Some(model.to_string());
        j.last_error = None;
    }
    let model = model.to_string();
    std::thread::Builder::new()
        .name("late-hf-dl".into())
        .spawn(move || {
            let log_path =
                download_log_path().unwrap_or_else(|| PathBuf::from("/tmp/late-hf-download.log"));
            let _ = fs::write(
                &log_path,
                format!("downloading {model} into {}\n", hf_home().display()),
            );
            let image = vllm_image();
            let hf = hf_home();
            let volume = format!("{}:/root/.cache/huggingface", hf.display());
            let mut args = vec![
                "run".into(),
                "--rm".into(),
                "--name".into(),
                "late-hf-dl".into(),
                "--entrypoint".into(),
                "/opt/venv/bin/hf".into(),
                "-v".into(),
                volume,
            ];
            if let Ok(token) =
                std::env::var("HF_TOKEN").or_else(|_| std::env::var("HUGGING_FACE_HUB_TOKEN"))
            {
                args.push("-e".into());
                args.push(format!("HF_TOKEN={token}"));
            }
            args.push(image);
            args.push("download".into());
            args.push(model.clone());
            let mut cmd = Command::new("docker");
            cmd.args(&args);
            let result = match fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
            {
                Ok(file) => {
                    let stderr = file.try_clone().ok();
                    cmd.stdin(Stdio::null());
                    cmd.stdout(Stdio::from(file));
                    if let Some(err) = stderr {
                        cmd.stderr(Stdio::from(err));
                    } else {
                        cmd.stderr(Stdio::piped());
                    }
                    cmd.status()
                }
                Err(e) => Err(e),
            };
            let mut j = job().lock().unwrap_or_else(|e| e.into_inner());
            j.downloading = false;
            match result {
                Ok(st) if st.success() => {
                    j.last_error = None;
                    j.download_id = None;
                    let _ = append_log(&log_path, &format!("finished {model}\n"));
                }
                Ok(st) => {
                    j.last_error = Some(format!(
                        "hf download exited {} — {}",
                        st.code().unwrap_or(-1),
                        download_log_tail().unwrap_or_else(|| "see docker/hf-download.log".into())
                    ));
                }
                Err(e) => {
                    j.last_error = Some(format!("hf download failed to start: {e}"));
                }
            }
        })
        .map_err(|e| LateError::Message(format!("could not spawn download thread: {e}")))?;
    Ok(status())
}

fn append_log(path: &Path, line: &str) -> std::io::Result<()> {
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    f.write_all(line.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_complete_requires_config_and_weights() {
        let tmp = tempfile::tempdir().unwrap();
        let snap = tmp.path().join("snapshots/abc");
        fs::create_dir_all(&snap).unwrap();
        assert!(!snapshot_complete(tmp.path()));
        fs::write(snap.join("config.json"), "{}").unwrap();
        assert!(!snapshot_complete(tmp.path()));
        fs::write(snap.join("tokenizer.json"), "{}").unwrap();
        assert!(!snapshot_complete(tmp.path()));
        fs::write(snap.join("model.safetensors"), b"x").unwrap();
        assert!(snapshot_complete(tmp.path()));
    }

    #[test]
    fn vllm_start_hidden_without_docker() {
        assert!(!vllm_start_allowed(true, false));
        assert!(!vllm_start_allowed(false, true));
        assert!(vllm_start_allowed(true, true));
    }
}
