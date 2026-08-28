//! Operator-initiated weight fetch for llama.cpp (Hugging Face GGUF) and Ollama
//! (`ollama pull`, including `hf.co/…` Hub ids). Never an agent tool.
//!
//! vLLM still uses `inference::download` (Intel compose + host HF cache).

use crate::config::{load_settings, AppSettings, LatePaths};
use crate::confine::confine_under_roots;
use crate::error::{LateError, Result};
use crate::fsutil::{mkdir_private, write_private};
use crate::hardware::GpuProfile;
use crate::inference::{validate_model, InferenceStatus, LocalModel};
use reqwest::redirect::{Attempt, Policy};
use serde::Deserialize;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// Approximate Q4_K_M (or official Q4_0 QAT) GGUF size (GB).
/// SFW instruct only. Gated Hub ids need HF_TOKEN.
const GGUF_LIBRARY: &[(&str, u32, &str)] = &[
    ("Qwen/Qwen3-0.6B-GGUF", 1, "Tiny Qwen3 GGUF"),
    ("Qwen/Qwen3-1.7B-GGUF", 2, "Tiny GGUF"),
    ("Qwen/Qwen3-4B-GGUF", 3, "Small GGUF"),
    ("Qwen/Qwen3-8B-GGUF", 6, "Default 8B GGUF"),
    ("Qwen/Qwen3-14B-GGUF", 10, "14B GGUF"),
    ("Qwen/Qwen3-32B-GGUF", 20, "Qwen3 32B GGUF"),
    ("unsloth/Qwen3.5-4B-GGUF", 3, "Qwen3.5 4B instruct GGUF"),
    ("unsloth/Qwen3.5-9B-GGUF", 6, "Qwen3.5 9B instruct GGUF"),
    ("unsloth/Qwen3.8-27B-GGUF", 16, "Newest Qwen dense 27B GGUF"),
    ("Qwen/Qwen2.5-7B-Instruct-GGUF", 5, "Qwen2.5 instruct GGUF (previous)"),
    (
        "google/gemma-4-E2B-it-qat-q4_0-gguf",
        3,
        "Official Gemma 4 E2B IT QAT GGUF",
    ),
    (
        "google/gemma-4-E4B-it-qat-q4_0-gguf",
        5,
        "Official Gemma 4 E4B IT QAT GGUF",
    ),
    (
        "google/gemma-4-12B-it-qat-q4_0-gguf",
        8,
        "Official Gemma 4 12B IT QAT GGUF",
    ),
    (
        "google/gemma-4-26B-A4B-it-qat-q4_0-gguf",
        16,
        "Official Gemma 4 26B-A4B IT QAT GGUF",
    ),
    (
        "google/gemma-4-31B-it-qat-q4_0-gguf",
        18,
        "Official Gemma 4 31B IT QAT GGUF",
    ),
    (
        "google/gemma-3-1b-it-qat-q4_0-gguf",
        1,
        "Official Gemma 3 1B IT QAT GGUF (gated, previous)",
    ),
    (
        "google/gemma-3-4b-it-qat-q4_0-gguf",
        3,
        "Official Gemma 3 4B IT QAT GGUF (gated, previous)",
    ),
    (
        "google/gemma-3-12b-it-qat-q4_0-gguf",
        8,
        "Official Gemma 3 12B IT QAT GGUF (gated, previous)",
    ),
    (
        "google/gemma-3-27b-it-qat-q4_0-gguf",
        16,
        "Official Gemma 3 27B IT QAT GGUF (gated, previous)",
    ),
    (
        "unsloth/gemma-3-4b-it-GGUF",
        3,
        "Gemma 3 4B IT GGUF, more quants (gated base, previous)",
    ),
    (
        "unsloth/Llama-4-Scout-17B-16E-Instruct-GGUF",
        60,
        "Llama 4 Scout instruct GGUF (gated base)",
    ),
    (
        "unsloth/Llama-3.3-70B-Instruct-GGUF",
        40,
        "Llama 3.3 70B instruct GGUF (gated base)",
    ),
    (
        "unsloth/Llama-3.2-1B-Instruct-GGUF",
        1,
        "Llama 3.2 1B instruct GGUF (gated base)",
    ),
    (
        "unsloth/Llama-3.2-3B-Instruct-GGUF",
        3,
        "Llama 3.2 3B instruct GGUF (gated base)",
    ),
    (
        "unsloth/Llama-3.1-8B-Instruct-GGUF",
        5,
        "Llama 3.1 8B instruct GGUF (gated base, previous)",
    ),
    (
        "unsloth/Phi-4-mini-instruct-GGUF",
        3,
        "Phi-4 mini instruct GGUF",
    ),
    ("unsloth/phi-4-GGUF", 9, "Phi-4 GGUF"),
    (
        "bartowski/Mistral-7B-Instruct-v0.3-GGUF",
        5,
        "Mistral 7B instruct GGUF",
    ),
];

const OLLAMA_LIBRARY: &[(&str, u32, &str)] = &[
    ("llama4:scout", 67, "Ollama library, Llama 4 Scout"),
    ("llama3.3", 40, "Ollama library, Llama 3.3 70B"),
    ("llama3.2", 2, "Ollama library, Llama 3.2 3B"),
    ("llama3.2:1b", 1, "Ollama library, Llama 3.2 1B"),
    ("llama3.1:8b", 5, "Ollama library, Llama 8B (previous)"),
    ("gemma4:e2b", 4, "Ollama library, Gemma 4 E2B"),
    ("gemma4:e4b", 6, "Ollama library, Gemma 4 E4B"),
    ("gemma4:12b", 8, "Ollama library, Gemma 4 12B"),
    ("gemma4:26b", 18, "Ollama library, Gemma 4 26B"),
    ("gemma4:31b", 20, "Ollama library, Gemma 4 31B"),
    ("gemma3:1b", 1, "Ollama library, Gemma 3 tiny (previous)"),
    ("gemma3:4b", 4, "Ollama library, Gemma 3 (previous)"),
    ("gemma3:12b", 8, "Ollama library, Gemma 3 12B (previous)"),
    ("gemma3:27b", 16, "Ollama library, Gemma 3 27B (previous)"),
    ("gemma2:9b", 6, "Ollama library, Gemma 2 (previous)"),
    ("qwen3.8:27b", 18, "Ollama library, Qwen3.8 27B"),
    ("qwen3.5:0.8b", 1, "Ollama library, Qwen3.5 tiny"),
    ("qwen3.5:2b", 2, "Ollama library, Qwen3.5 2B"),
    ("qwen3.5:4b", 3, "Ollama library, Qwen3.5 4B"),
    ("qwen3.5:9b", 6, "Ollama library, Qwen3.5 9B"),
    ("qwen3.5:27b", 17, "Ollama library, Qwen3.5 27B"),
    ("qwen3:0.6b", 1, "Ollama library, Qwen3 tiny (previous)"),
    ("qwen3:1.7b", 2, "Ollama library, Qwen3 1.7B (previous)"),
    ("qwen3:4b", 3, "Ollama library, Qwen3 4B (previous)"),
    ("qwen3:8b", 5, "Ollama library, Qwen3 8B"),
    ("qwen3:14b", 9, "Ollama library, Qwen3 14B"),
    ("qwen3:32b", 20, "Ollama library, Qwen3 32B (previous vs 3.8)"),
    ("qwen2.5:7b", 5, "Ollama library, Qwen 2.5 (previous)"),
    ("qwen2.5:14b", 9, "Ollama library, Qwen 2.5 larger (previous)"),
    ("mistral-small3.2", 15, "Ollama library, Mistral Small 3.2"),
    ("mistral", 5, "Ollama library, Mistral (previous)"),
    ("phi4", 9, "Ollama library, Phi-4"),
    ("phi4-mini", 3, "Ollama library, Phi-4 mini"),
    ("granite4.2:8b", 6, "Ollama library, Granite 4.2"),
    ("granite4.2:3b", 3, "Ollama library, Granite 4.2 3B"),
    ("granite4.2:30b", 18, "Ollama library, Granite 4.2 30B"),
    ("granite3.3:8b", 5, "Ollama library, Granite 3.3 (previous)"),
    (
        "hf.co/google/gemma-4-E4B-it-qat-q4_0-gguf",
        5,
        "Hugging Face Gemma 4 E4B via Ollama",
    ),
    (
        "hf.co/google/gemma-3-4b-it-qat-q4_0-gguf",
        3,
        "Hugging Face Gemma 3 4B via Ollama (gated, previous)",
    ),
    (
        "hf.co/Qwen/Qwen3-8B-GGUF",
        6,
        "Hugging Face Qwen3 8B GGUF via Ollama",
    ),
];

const QUANT_PREF: &[&str] = &[
    "q4_k_m", "q5_k_m", "q4_k_s", "q5_k_s", "q6_k", "q8_0", "q4_0", "q5_0",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    Vllm,
    LlamaCpp,
    Ollama,
}

pub fn parse_engine(s: &str) -> Result<Engine> {
    match s.trim().to_ascii_lowercase().as_str() {
        "" | "vllm" | "local" | "local-vllm" | "local_vllm" => Ok(Engine::Vllm),
        "llamacpp" | "llama.cpp" | "llama-cpp" | "llama_cpp" => Ok(Engine::LlamaCpp),
        "ollama" => Ok(Engine::Ollama),
        other => Err(LateError::Message(format!(
            "unknown inference engine {other:?} (use vllm, llamacpp, or ollama)"
        ))),
    }
}

#[derive(Default)]
struct Job {
    downloading: bool,
    download_id: Option<String>,
    starting: bool,
    last_error: Option<String>,
    progress: Option<String>,
    serve_model: Option<String>,
    child: Option<Child>,
}

fn llama_job() -> &'static Mutex<Job> {
    static JOB: OnceLock<Mutex<Job>> = OnceLock::new();
    JOB.get_or_init(|| Mutex::new(Job::default()))
}

fn ollama_job() -> &'static Mutex<Job> {
    static JOB: OnceLock<Mutex<Job>> = OnceLock::new();
    JOB.get_or_init(|| Mutex::new(Job::default()))
}

fn paths() -> LatePaths {
    LatePaths::discover()
}

fn gguf_root() -> Result<PathBuf> {
    let root = paths().data.join("models").join("gguf");
    mkdir_private(&paths().data)?;
    mkdir_private(&paths().data.join("models"))?;
    mkdir_private(&root)?;
    Ok(root)
}

fn gguf_log() -> PathBuf {
    paths().data.join("logs").join("gguf-download.log")
}

fn ollama_log() -> PathBuf {
    paths().data.join("logs").join("ollama-pull.log")
}

fn llama_pid_file() -> PathBuf {
    paths()
        .data
        .join("models")
        .join("gguf")
        .join("llama-server.pid")
}

fn llama_server_log() -> PathBuf {
    paths().data.join("logs").join("llama-server.log")
}

pub fn status_for(engine: Engine, settings: &AppSettings) -> InferenceStatus {
    match engine {
        Engine::Vllm => crate::inference::status_with(settings.use_all_gpus),
        Engine::LlamaCpp => llama_status(settings),
        Engine::Ollama => ollama_status(settings),
    }
}

pub fn download(engine: Engine, model: &str, settings: &AppSettings) -> Result<InferenceStatus> {
    match engine {
        Engine::Vllm => crate::inference::download(model),
        Engine::LlamaCpp => start_gguf_download(model),
        Engine::Ollama => start_ollama_pull(model, settings),
    }
}

pub fn start(engine: Engine, model: &str, settings: &AppSettings) -> Result<InferenceStatus> {
    match engine {
        Engine::Vllm => crate::inference::start_with(model, settings.use_all_gpus),
        Engine::LlamaCpp => start_llama_server(model, settings),
        Engine::Ollama => Err(LateError::Message(
            "Ollama is already the server. Use Pull to fetch a model, then pick it in the model list."
                .into(),
        )),
    }
}

pub fn stop(engine: Engine) -> Result<InferenceStatus> {
    match engine {
        Engine::Vllm => crate::inference::stop(),
        Engine::LlamaCpp => stop_llama_server(),
        Engine::Ollama => Err(LateError::Message(
            "Stop does not shut down Ollama. Quit the Ollama app if you want it off.".into(),
        )),
    }
}

fn llama_status(settings: &AppSettings) -> InferenceStatus {
    let (starting, downloading, download_id, serve_model, last_error, progress, child_live) = {
        let mut j = llama_job().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(child) = j.child.as_mut() {
            if let Ok(Some(st)) = child.try_wait() {
                j.child = None;
                let _ = fs::remove_file(llama_pid_file());
                if !st.success() {
                    j.last_error = Some(format!(
                        "llama-server exited {} — {}",
                        st.code().unwrap_or(-1),
                        tail_log(&llama_server_log())
                            .unwrap_or_else(|| "see logs/llama-server.log".into())
                    ));
                }
            }
        }
        (
            j.starting,
            j.downloading,
            j.download_id.clone(),
            j.serve_model.clone(),
            j.last_error.clone(),
            j.progress.clone(),
            j.child.is_some(),
        )
    };
    let gpu = crate::hardware::probe();
    let gpu_launch = crate::hardware::launch_plan(&gpu, settings.use_all_gpus);
    let local_models = list_gguf_models(&gpu);
    let bind = loopback_bind(&settings.llama_cpp_base_url).ok();
    let models = bind
        .as_ref()
        .map(|b| probe_openai_models(&format!("http://{b}/v1/models")))
        .unwrap_or_default();
    let serving = !models.is_empty();
    if serving {
        let mut j = llama_job().lock().unwrap_or_else(|e| e.into_inner());
        j.starting = false;
    }
    let owned = owned_llama_running();
    let running = serving || owned || starting;
    let bin = find_in_path("llama-server").or_else(|| find_in_path("llama-cpp-server"));
    let detail = if let Some(err) = last_error {
        err
    } else if downloading {
        let id = download_id.clone().unwrap_or_else(|| "GGUF".into());
        progress.unwrap_or_else(|| {
            format!(
                "downloading {id} from Hugging Face — {}",
                tail_log(&gguf_log()).unwrap_or_else(|| "starting…".into())
            )
        })
    } else if starting && !serving {
        match serve_model.as_deref() {
            Some(id) => format!("starting llama-server with {id}"),
            None => "starting llama-server".into(),
        }
    } else if serving {
        if owned || child_live {
            format!("serving {}", models.join(", "))
        } else {
            format!(
                "serving {} — Stop only applies to llama-server Late started",
                models.join(", ")
            )
        }
    } else if owned {
        "llama-server is up; model still loading".into()
    } else if bin.is_none() {
        "Download GGUF here, then install llama.cpp so `llama-server` is on PATH, or run it yourself on 127.0.0.1:8080.".into()
    } else {
        "llama.cpp idle. Download a GGUF from Hugging Face, then Start (needs llama-server on PATH).".into()
    };
    InferenceStatus {
        running,
        starting,
        downloading,
        models,
        local_models,
        container: None,
        download_id,
        serve_model,
        gpu,
        detail,
        allow_intel_compose: false,
        late_owned: owned || starting || child_live,
        gpu_launch: Some(gpu_launch),
    }
}

fn ollama_status(settings: &AppSettings) -> InferenceStatus {
    let (downloading, download_id, last_error, progress) = {
        let j = ollama_job().lock().unwrap_or_else(|e| e.into_inner());
        (
            j.downloading,
            j.download_id.clone(),
            j.last_error.clone(),
            j.progress.clone(),
        )
    };
    let gpu = crate::hardware::probe();
    let gpu_launch = crate::hardware::launch_plan(&gpu, settings.use_all_gpus);
    let native = match ollama_native_root(&settings.ollama_base_url) {
        Ok(u) => u,
        Err(e) => {
            return InferenceStatus {
                running: false,
                starting: false,
                downloading,
                models: vec![],
                local_models: list_ollama_models(&gpu, &[]),
                container: None,
                download_id,
                serve_model: None,
                gpu,
                detail: e.to_string(),
                allow_intel_compose: false,
                late_owned: false,
                gpu_launch: Some(gpu_launch),
            };
        }
    };
    let pulled = probe_ollama_tags(&native);
    let models = pulled.clone();
    let local_models = list_ollama_models(&gpu, &pulled);
    let reachable = ollama_reachable(&native);
    let detail = if let Some(err) = last_error {
        err
    } else if downloading {
        let id = download_id.clone().unwrap_or_else(|| "model".into());
        progress.unwrap_or_else(|| {
            format!(
                "pulling {id} via Ollama — {}",
                tail_log(&ollama_log()).unwrap_or_else(|| "starting…".into())
            )
        })
    } else if !reachable {
        format!(
            "Ollama is not running at {native}. Install from https://ollama.com and start it, then Pull. Late does not install Ollama."
        )
    } else if models.is_empty() {
        format!("Ollama is running at {native} with no models. Pull a library name (gemma4:e4b, qwen3:8b) or a Hugging Face id (google/gemma-4-E4B-it-qat-q4_0-gguf).")
    } else {
        format!("Ollama at {native} · {}", models.join(", "))
    };
    InferenceStatus {
        running: reachable,
        starting: false,
        downloading,
        models,
        local_models,
        container: None,
        download_id,
        serve_model: None,
        gpu,
        detail,
        allow_intel_compose: false,
        late_owned: false,
        gpu_launch: Some(gpu_launch),
    }
}

fn start_gguf_download(model: &str) -> Result<InferenceStatus> {
    let spec = parse_hub_spec(model)?;
    validate_model(&spec.repo)?;
    if let Some(file) = &spec.file {
        safe_gguf_name(file)?;
    }
    {
        let mut j = llama_job().lock().unwrap_or_else(|e| e.into_inner());
        if j.downloading {
            return Ok(llama_status(&load_settings_or_default()));
        }
        j.downloading = true;
        j.download_id = Some(model.to_string());
        j.last_error = None;
        j.progress = Some(format!("downloading {} from Hugging Face", spec.repo));
    }
    let spec = spec.clone();
    let label = model.to_string();
    std::thread::Builder::new()
        .name("late-gguf-dl".into())
        .spawn(move || {
            let result = block_on(download_gguf(spec.clone()));
            let mut j = llama_job().lock().unwrap_or_else(|e| e.into_inner());
            j.downloading = false;
            j.progress = None;
            match result {
                Ok(path) => {
                    j.last_error = None;
                    j.download_id = None;
                    j.serve_model = Some(label.clone());
                    let _ = append_log(
                        &gguf_log(),
                        &format!("finished {} -> {}\n", spec.repo, path.display()),
                    );
                }
                Err(e) => {
                    j.last_error = Some(e.to_string());
                    let _ = append_log(&gguf_log(), &format!("error: {e}\n"));
                }
            }
        })
        .map_err(|e| {
            let mut j = llama_job().lock().unwrap_or_else(|e| e.into_inner());
            j.downloading = false;
            LateError::Message(format!("could not spawn GGUF download thread: {e}"))
        })?;
    Ok(llama_status(&load_settings_or_default()))
}

fn start_ollama_pull(model: &str, settings: &AppSettings) -> Result<InferenceStatus> {
    let name = ollama_pull_name(model)?;
    let root = ollama_native_root(&settings.ollama_base_url)?;
    {
        let mut j = ollama_job().lock().unwrap_or_else(|e| e.into_inner());
        if j.downloading {
            return Ok(ollama_status(settings));
        }
        j.downloading = true;
        j.download_id = Some(name.clone());
        j.last_error = None;
        j.progress = Some(format!("pulling {name} via Ollama"));
    }
    std::thread::Builder::new()
        .name("late-ollama-pull".into())
        .spawn(move || {
            let result = block_on(ollama_pull(&root, &name));
            let mut j = ollama_job().lock().unwrap_or_else(|e| e.into_inner());
            j.downloading = false;
            j.progress = None;
            match result {
                Ok(()) => {
                    j.last_error = None;
                    j.download_id = None;
                    let _ = append_log(&ollama_log(), &format!("finished {name}\n"));
                }
                Err(e) => {
                    j.last_error = Some(e.to_string());
                    let _ = append_log(&ollama_log(), &format!("error: {e}\n"));
                }
            }
        })
        .map_err(|e| {
            let mut j = ollama_job().lock().unwrap_or_else(|e| e.into_inner());
            j.downloading = false;
            LateError::Message(format!("could not spawn Ollama pull thread: {e}"))
        })?;
    Ok(ollama_status(settings))
}

fn start_llama_server(model: &str, settings: &AppSettings) -> Result<InferenceStatus> {
    let bind = loopback_bind(&settings.llama_cpp_base_url)?;
    let bin = find_in_path("llama-server")
        .or_else(|| find_in_path("llama-cpp-server"))
        .ok_or_else(|| {
            LateError::Message(
                "llama-server is not on PATH. Late does not install llama.cpp. Download still saved the GGUF under ~/.local/share/late/models/gguf/ — run llama-server yourself, or install llama.cpp and click Start."
                    .into(),
            )
        })?;
    let weights = resolve_gguf_file(model)?;
    {
        let mut j = llama_job().lock().unwrap_or_else(|e| e.into_inner());
        if j.child.is_some() || owned_llama_running() {
            return Ok(llama_status(settings));
        }
        j.starting = true;
        j.serve_model = Some(model.to_string());
        j.last_error = None;
    }
    let log_path = llama_server_log();
    if let Some(parent) = log_path.parent() {
        let _ = mkdir_private(parent);
    }
    let log = OpenOptions::new().create(true).append(true).open(&log_path);
    let gpu = crate::hardware::probe();
    let mut cmd = Command::new(&bin);
    cmd.arg("-m")
        .arg(&weights)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(bind.port().to_string());
    for a in crate::hardware::llama_gpu_args(&gpu, settings.use_all_gpus) {
        cmd.arg(a);
    }
    for (k, v) in crate::hardware::llama_gpu_env(&gpu, settings.use_all_gpus) {
        cmd.env(k, v);
    }
    cmd.stdin(Stdio::null());
    match log {
        Ok(f) => {
            let err = f.try_clone().ok();
            cmd.stdout(Stdio::from(f));
            if let Some(e) = err {
                cmd.stderr(Stdio::from(e));
            } else {
                cmd.stderr(Stdio::piped());
            }
        }
        Err(_) => {
            cmd.stdout(Stdio::null());
            cmd.stderr(Stdio::null());
        }
    }
    match cmd.spawn() {
        Ok(child) => {
            let _ = write_owned_pid(child.id());
            let mut j = llama_job().lock().unwrap_or_else(|e| e.into_inner());
            j.child = Some(child);
            j.starting = true;
        }
        Err(e) => {
            let mut j = llama_job().lock().unwrap_or_else(|e| e.into_inner());
            j.starting = false;
            j.last_error = Some(format!("could not start llama-server: {e}"));
            return Err(LateError::Message(format!(
                "could not start llama-server: {e}"
            )));
        }
    }
    Ok(llama_status(settings))
}

fn stop_llama_server() -> Result<InferenceStatus> {
    let mut j = llama_job().lock().unwrap_or_else(|e| e.into_inner());
    j.starting = false;
    if let Some(mut child) = j.child.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    if let Some(pid) = read_owned_pid().and_then(|rec| {
        if rec.matches_live() {
            Some(rec.pid)
        } else {
            None
        }
    }) {
        let _ = Command::new("kill").arg(pid.to_string()).status();
    }
    let _ = fs::remove_file(llama_pid_file());
    drop(j);
    Ok(llama_status(&load_settings_or_default()))
}

fn load_settings_or_default() -> AppSettings {
    load_settings(&paths().settings()).unwrap_or_default()
}

fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    if let Ok(h) = tokio::runtime::Handle::try_current() {
        return h.block_on(fut);
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
        .block_on(fut)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HubSpec {
    pub repo: String,
    /// Exact filename, or a quant tag such as `Q4_K_M`.
    pub file: Option<String>,
}

pub(crate) fn parse_hub_spec(raw: &str) -> Result<HubSpec> {
    let mut s = raw.trim().to_string();
    if s.is_empty() {
        return Err(LateError::Message("missing Hugging Face id".into()));
    }
    for prefix in [
        "https://huggingface.co/",
        "http://huggingface.co/",
        "https://hf.co/",
        "http://hf.co/",
        "hf.co/",
        "huggingface.co/",
    ] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.trim_start_matches('/').to_string();
            break;
        }
    }
    s = s.split('?').next().unwrap_or(&s).to_string();
    s = s.trim_start_matches('/').to_string();
    for marker in ["/blob/", "/tree/", "/resolve/"] {
        if let Some(i) = s.find(marker) {
            s.truncate(i);
            break;
        }
    }
    if s.is_empty() || !s.contains('/') {
        return Err(LateError::Message(
            "need a Hugging Face id like Qwen/Qwen3-8B-GGUF (optionally :Q4_K_M or :file.gguf)"
                .into(),
        ));
    }
    validate_model(&s)?;
    let (repo, file) = match s.rsplit_once(':') {
        Some((repo, spec)) if repo.contains('/') && !spec.is_empty() && !spec.contains('/') => {
            (repo.to_string(), Some(spec.to_string()))
        }
        _ => (s.clone(), None),
    };
    let segs: Vec<&str> = repo.split('/').collect();
    if segs.len() != 2 || segs.iter().any(|p| p.is_empty()) {
        return Err(LateError::Message("invalid Hugging Face repo id".into()));
    }
    Ok(HubSpec { repo, file })
}

pub(crate) fn ollama_pull_name(raw: &str) -> Result<String> {
    let s = raw.trim();
    if s.is_empty() {
        return Err(LateError::Message("missing Ollama model name".into()));
    }
    validate_model(s)?;
    if s.starts_with("hf.co/") {
        return Ok(s.to_string());
    }
    if let Some(rest) = s.strip_prefix("huggingface.co/") {
        return Ok(format!("hf.co/{rest}"));
    }
    for prefix in [
        "https://huggingface.co/",
        "http://huggingface.co/",
        "https://hf.co/",
        "http://hf.co/",
    ] {
        if let Some(rest) = s.strip_prefix(prefix) {
            return Ok(format!("hf.co/{rest}"));
        }
    }
    if s.contains('/') {
        return Ok(format!("hf.co/{s}"));
    }
    Ok(s.to_string())
}

fn safe_gguf_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 200 {
        return Err(LateError::Message("invalid GGUF filename".into()));
    }
    if name.contains("..")
        || name.contains('\\')
        || name.contains('/')
        || name.starts_with('/')
        || name.starts_with('.')
    {
        return Err(LateError::Message("GGUF filename rejected".into()));
    }
    if name.chars().any(|c| c.is_control() || c == '\0') {
        return Err(LateError::Message("GGUF filename rejected".into()));
    }
    Ok(())
}

fn ensure_gguf_magic(path: &Path) -> Result<()> {
    let mut f = fs::File::open(path)?;
    let mut magic = [0u8; 4];
    f.read_exact(&mut magic)
        .map_err(|_| LateError::Message("downloaded file is too small to be a GGUF".into()))?;
    if &magic != b"GGUF" {
        return Err(LateError::Message(
            "downloaded file is not a GGUF (missing GGUF magic)".into(),
        ));
    }
    Ok(())
}

fn skip_gguf(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("mmproj")
        || n.contains("projector")
        || n.contains("encoder")
        || n.contains("draft")
        || n.contains("speculative")
        || n.contains("-of-")
}

pub(crate) fn pick_gguf_file(files: &[String], want: Option<&str>) -> Result<String> {
    let ggufs: Vec<&String> = files
        .iter()
        .filter(|f| f.to_ascii_lowercase().ends_with(".gguf") && !skip_gguf(f))
        .collect();
    if ggufs.is_empty() {
        return Err(LateError::Message(
            "that Hugging Face repo has no single-file GGUF (need a *-GGUF repo, not safetensors)"
                .into(),
        ));
    }
    if let Some(want) = want {
        let w = want.to_ascii_lowercase();
        if w.ends_with(".gguf") {
            let leaf = Path::new(want)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(want);
            return ggufs
                .iter()
                .find(|f| {
                    let n = f.to_ascii_lowercase();
                    n == w
                        || Path::new(*f)
                            .file_name()
                            .and_then(|x| x.to_str())
                            .is_some_and(|x| x.eq_ignore_ascii_case(leaf))
                })
                .map(|s| (*s).clone())
                .ok_or_else(|| LateError::Message(format!("{want} is not in that repo")));
        }
        if let Some(f) = ggufs
            .iter()
            .find(|f| quant_token(f).is_some_and(|t| t == w))
        {
            return Ok((*f).clone());
        }
        return Err(LateError::Message(format!(
            "no GGUF matching {want} in that repo"
        )));
    }
    for q in QUANT_PREF {
        if let Some(f) = ggufs
            .iter()
            .find(|f| quant_token(f).is_some_and(|t| t == *q))
        {
            return Ok((*f).clone());
        }
    }
    Ok(ggufs[0].clone())
}

fn quant_token(filename: &str) -> Option<String> {
    let n = filename.to_ascii_lowercase();
    let stem = n.strip_suffix(".gguf")?;
    let tok = stem.rsplit('-').next()?;
    if tok.is_empty() {
        None
    } else {
        Some(tok.to_string())
    }
}

pub(crate) fn hf_download_host_ok(host: &str) -> bool {
    let h = host.trim().trim_end_matches('.').to_ascii_lowercase();
    matches!(
        h.as_str(),
        "huggingface.co" | "www.huggingface.co" | "hf.co"
    ) || h.ends_with(".huggingface.co")
        || h.ends_with(".xethub.hf.co")
}

fn hf_redirect(attempt: Attempt<'_>) -> reqwest::redirect::Action {
    if attempt.previous().len() > 16 {
        return attempt.error("too many redirects");
    }
    let host = attempt.url().host_str().map(str::to_string);
    match host.as_deref() {
        Some(h) if hf_download_host_ok(h) => attempt.follow(),
        Some(h) => attempt.error(format!("refusing redirect to {h}")),
        None => attempt.error("redirect with no host"),
    }
}

fn hf_token() -> Option<String> {
    std::env::var("HF_TOKEN")
        .or_else(|_| std::env::var("HUGGING_FACE_HUB_TOKEN"))
        .ok()
        .filter(|s| !s.is_empty())
}

fn hf_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .redirect(Policy::custom(hf_redirect))
        .build()
        .map_err(|e| LateError::Http(e.to_string()))
}

#[derive(Debug, Deserialize)]
struct HfModelInfo {
    #[serde(default)]
    siblings: Vec<HfSibling>,
}

#[derive(Debug, Deserialize)]
struct HfSibling {
    rfilename: String,
}

async fn download_gguf(spec: HubSpec) -> Result<PathBuf> {
    let _ = mkdir_private(&paths().data.join("logs"));
    let _ = append_log(
        &gguf_log(),
        &format!("listing {} on Hugging Face\n", spec.repo),
    );
    let client = hf_client()?;
    let api = format!("https://huggingface.co/api/models/{}", spec.repo);
    let mut req = client.get(&api);
    if let Some(t) = hf_token() {
        req = req.bearer_auth(t);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| LateError::Http(redact_secrets(&e.to_string())))?;
    let status = resp.status();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(LateError::Message(
            "Hugging Face returned 401/403. For gated repos set HF_TOKEN (or HUGGING_FACE_HUB_TOKEN) in the environment."
                .into(),
        ));
    }
    if status.as_u16() == 404 {
        return Err(LateError::Message(format!(
            "{} was not found. Use a GGUF repo such as Qwen/Qwen3-8B-GGUF, not the safetensors id.",
            spec.repo
        )));
    }
    if !status.is_success() {
        return Err(LateError::Message(format!(
            "Hugging Face API {} for {}",
            status.as_u16(),
            spec.repo
        )));
    }
    let info: HfModelInfo = resp
        .json()
        .await
        .map_err(|e| LateError::Http(format!("Hugging Face API JSON: {e}")))?;
    let files: Vec<String> = info.siblings.into_iter().map(|s| s.rfilename).collect();
    let listed = pick_gguf_file(&files, spec.file.as_deref())?;
    if listed.contains("..") || listed.starts_with('/') || listed.contains('\\') {
        return Err(LateError::Message("GGUF filename rejected".into()));
    }
    let filename = Path::new(&listed)
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| LateError::Message("invalid GGUF filename".into()))?
        .to_string();
    safe_gguf_name(&filename)?;
    let dest_dir = gguf_root()?.join(spec.repo.replace('/', "--"));
    mkdir_private(&dest_dir)?;
    let dest = confine_under_roots(&dest_dir.join(&filename), &[gguf_root()?], false)?;
    refuse_symlink(&dest)?;
    refuse_symlink(&dest_dir)?;
    if dest.exists() {
        let _ = append_log(
            &gguf_log(),
            &format!("already on disk {}\n", dest.display()),
        );
        ensure_gguf_magic(&dest)?;
        return Ok(dest);
    }
    let mut url = reqwest::Url::parse(&format!(
        "https://huggingface.co/{}/resolve/main/",
        spec.repo
    ))
    .map_err(|e| LateError::Http(e.to_string()))?;
    {
        let mut segs = url
            .path_segments_mut()
            .map_err(|_| LateError::Http("bad Hugging Face URL".into()))?;
        segs.pop_if_empty();
        for seg in listed.split('/') {
            if seg.is_empty() || seg == "." || seg == ".." {
                return Err(LateError::Message("GGUF filename rejected".into()));
            }
            segs.push(seg);
        }
    }
    let _ = append_log(&gguf_log(), &format!("GET {filename}\n"));
    let mut req = client.get(url);
    if let Some(t) = hf_token() {
        req = req.bearer_auth(t);
    }
    let mut resp = req
        .send()
        .await
        .map_err(|e| LateError::Http(redact_secrets(&e.to_string())))?;
    if !resp.status().is_success() {
        let code = resp.status().as_u16();
        if code == 401 || code == 403 {
            return Err(LateError::Message(
                "Hugging Face returned 401/403. For gated repos set HF_TOKEN (or HUGGING_FACE_HUB_TOKEN) in the environment."
                    .into(),
            ));
        }
        return Err(LateError::Message(format!(
            "download HTTP {code} for {filename}"
        )));
    }
    let total = resp.content_length();
    const MAX_GGUF_BYTES: u64 = 80_000_000_000;
    if total.is_some_and(|t| t > MAX_GGUF_BYTES) {
        return Err(LateError::Message(format!(
            "{filename} is larger than 80GB — refuse to download that much in one file"
        )));
    }
    let part = dest.with_extension("part");
    refuse_symlink(&part)?;
    {
        let mut opts = OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut file = opts.open(&part)?;
        let mut got = 0u64;
        let mut last_note = 0u64;
        loop {
            match resp.chunk().await {
                Ok(Some(chunk)) => {
                    file.write_all(&chunk)?;
                    got = got.saturating_add(chunk.len() as u64);
                    if got > MAX_GGUF_BYTES {
                        let _ = fs::remove_file(&part);
                        return Err(LateError::Message(
                            "GGUF download exceeded 80GB — stopping".into(),
                        ));
                    }
                    if got.saturating_sub(last_note) >= 32 * 1024 * 1024 {
                        last_note = got;
                        let msg = match total {
                            Some(t) if t > 0 => format!(
                                "downloading {filename} · {:.1}/{:.1} GB",
                                got as f64 / 1e9,
                                t as f64 / 1e9
                            ),
                            _ => format!("downloading {filename} · {:.1} GB", got as f64 / 1e9),
                        };
                        let mut j = llama_job().lock().unwrap_or_else(|e| e.into_inner());
                        j.progress = Some(msg.clone());
                        let _ = append_log(&gguf_log(), &format!("{msg}\n"));
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    let _ = fs::remove_file(&part);
                    return Err(LateError::Http(redact_secrets(&e.to_string())));
                }
            }
        }
        file.flush()?;
    }
    fs::rename(&part, &dest)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut p = fs::metadata(&dest)?.permissions();
        p.set_mode(0o600);
        fs::set_permissions(&dest, p)?;
    }
    if let Err(e) = ensure_gguf_magic(&dest) {
        let _ = fs::remove_file(&dest);
        return Err(e);
    }
    Ok(dest)
}

async fn ollama_pull(root: &str, name: &str) -> Result<()> {
    let _ = mkdir_private(&paths().data.join("logs"));
    let _ = append_log(&ollama_log(), &format!("POST {root}/api/pull {name}\n"));
    let url = format!("{root}/api/pull");
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .redirect(Policy::custom(|attempt| {
            if attempt.previous().len() > 4 {
                return attempt.error("too many redirects");
            }
            let host = attempt.url().host_str().unwrap_or("").to_string();
            if is_loopback_hostname(&host) {
                attempt.follow()
            } else {
                attempt.error(format!("refusing Ollama redirect to {host}"))
            }
        }))
        .build()
        .map_err(|e| LateError::Http(redact_secrets(&e.to_string())))?;
    let mut resp = client
        .post(&url)
        .json(&serde_json::json!({"model": name, "name": name, "stream": true}))
        .send()
        .await
        .map_err(|e| {
            LateError::Message(format!(
                "Ollama pull failed to start ({}). Is Ollama running at {root}?",
                redact_secrets(&e.to_string())
            ))
        })?;
    if !resp.status().is_success() {
        let code = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(LateError::Message(format!(
            "Ollama pull HTTP {code}: {}",
            redact_secrets(&body.chars().take(200).collect::<String>())
        )));
    }
    let mut buf = Vec::new();
    loop {
        let chunk = resp
            .chunk()
            .await
            .map_err(|e| LateError::Http(e.to_string()))?;
        let Some(chunk) = chunk else { break };
        buf.extend_from_slice(&chunk);
        while let Some(i) = buf.iter().position(|b| *b == b'\n') {
            let line = buf.drain(..=i).collect::<Vec<_>>();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let v: serde_json::Value =
                serde_json::from_str(line).unwrap_or(serde_json::Value::Null);
            if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
                return Err(LateError::Message(err.into()));
            }
            let status = v
                .get("status")
                .and_then(|s| s.as_str())
                .unwrap_or("pulling");
            let msg = match (
                v.get("completed").and_then(|c| c.as_u64()),
                v.get("total").and_then(|t| t.as_u64()),
            ) {
                (Some(c), Some(t)) if t > 0 => {
                    format!("{status} · {:.1}/{:.1} GB", c as f64 / 1e9, t as f64 / 1e9)
                }
                _ => status.to_string(),
            };
            {
                let mut j = ollama_job().lock().unwrap_or_else(|e| e.into_inner());
                j.progress = Some(format!("pulling {name} — {msg}"));
            }
            let _ = append_log(&ollama_log(), &format!("{msg}\n"));
        }
    }
    Ok(())
}

fn list_gguf_models(gpu: &GpuProfile) -> Vec<LocalModel> {
    let recs = crate::hardware::rank_quant_catalog(GGUF_LIBRARY, gpu);
    let mut out = scan_gguf_dir();
    out.extend(scan_hf_gguf_cache());
    for m in &mut out {
        if let Some(r) = recs.iter().find(|r| r.id == m.id || m.id.starts_with(&format!("{}:", r.id)))
        {
            m.recommended = r.recommended;
            m.newest = r.newest;
        }
    }
    for r in &recs {
        if !out
            .iter()
            .any(|m| m.id == r.id || m.id.starts_with(&format!("{}:", r.id)))
        {
            out.push(LocalModel {
                id: r.id.clone(),
                complete: false,
                size_bytes: 0,
                note: r.reason.clone(),
                recommended: r.recommended,
                newest: r.newest,
                tp: 1,
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

fn list_ollama_models(gpu: &GpuProfile, pulled: &[String]) -> Vec<LocalModel> {
    let recs = crate::hardware::rank_quant_catalog(OLLAMA_LIBRARY, gpu);
    let mut out = Vec::new();
    for name in pulled {
        let rec = recs.iter().find(|r| ollama_name_matches(&r.id, name));
        out.push(LocalModel {
            id: name.clone(),
            complete: true,
            size_bytes: 0,
            note: "pulled into Ollama".into(),
            recommended: rec.map(|r| r.recommended).unwrap_or(false),
            newest: rec.map(|r| r.newest).unwrap_or(false),
            tp: 1,
        });
    }
    for r in &recs {
        if !out.iter().any(|m| ollama_name_matches(&r.id, &m.id)) {
            out.push(LocalModel {
                id: r.id.clone(),
                complete: false,
                size_bytes: 0,
                note: r.reason.clone(),
                recommended: r.recommended,
                newest: r.newest,
                tp: 1,
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

fn ollama_name_matches(want: &str, have: &str) -> bool {
    let want = want.trim_start_matches("hf.co/");
    let have = have.trim_start_matches("hf.co/");
    have == want
        || have.starts_with(&format!("{want}:"))
        || have.starts_with(&format!("{want}/"))
        || want.starts_with(have)
}

fn scan_gguf_dir() -> Vec<LocalModel> {
    let Ok(root) = gguf_root() else {
        return vec![];
    };
    let Ok(repos) = fs::read_dir(&root) else {
        return vec![];
    };
    let mut out = Vec::new();
    for repo in repos.flatten() {
        if !repo.path().is_dir() {
            continue;
        }
        let id = repo.file_name().to_string_lossy().replace("--", "/");
        let Ok(files) = fs::read_dir(repo.path()) else {
            continue;
        };
        let mut ggufs: Vec<(String, u64)> = Vec::new();
        for f in files.flatten() {
            let name = f.file_name().to_string_lossy().to_string();
            if !name.to_ascii_lowercase().ends_with(".gguf") || skip_gguf(&name) {
                continue;
            }
            let size = f.metadata().map(|m| m.len()).unwrap_or(0);
            ggufs.push((name, size));
        }
        if ggufs.len() == 1 {
            out.push(LocalModel {
                id,
                complete: true,
                size_bytes: ggufs[0].1,
                note: format!("GGUF on disk ({})", ggufs[0].0),
                recommended: false,
                newest: false,
                tp: 1,
            });
        } else {
            for (name, size) in ggufs {
                out.push(LocalModel {
                    id: format!("{id}:{name}"),
                    complete: true,
                    size_bytes: size,
                    note: "GGUF on disk".into(),
                    recommended: false,
                    newest: false,
                    tp: 1,
                });
            }
        }
    }
    out
}

fn scan_hf_gguf_cache() -> Vec<LocalModel> {
    let hub = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cache/huggingface/hub");
    let Ok(rd) = fs::read_dir(&hub) else {
        return vec![];
    };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !name.starts_with("models--") {
            continue;
        }
        let id = name.trim_start_matches("models--").replace("--", "/");
        let snaps = e.path().join("snapshots");
        let Ok(snaps) = fs::read_dir(&snaps) else {
            continue;
        };
        for snap in snaps.flatten() {
            if !snap.path().is_dir() {
                continue;
            }
            let Ok(files) = fs::read_dir(snap.path()) else {
                continue;
            };
            for f in files.flatten() {
                let fname = f.file_name().to_string_lossy().to_string();
                if !fname.to_ascii_lowercase().ends_with(".gguf") || skip_gguf(&fname) {
                    continue;
                }
                let size = f.metadata().map(|m| m.len()).unwrap_or(0);
                out.push(LocalModel {
                    id: format!("{id}:{fname}"),
                    complete: true,
                    size_bytes: size,
                    note: "GGUF in Hugging Face cache".into(),
                    recommended: false,
                    newest: false,
                    tp: 1,
                });
            }
        }
    }
    out
}

fn collect_ggufs_in(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(dir) else {
        return vec![];
    };
    rd.flatten()
        .filter_map(|f| {
            let n = f.file_name().to_string_lossy().to_string();
            if n.to_ascii_lowercase().ends_with(".gguf") && !skip_gguf(&n) {
                Some(f.path())
            } else {
                None
            }
        })
        .collect()
}

fn resolve_gguf_file(model: &str) -> Result<PathBuf> {
    let spec = parse_hub_spec(model)?;
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(root) = gguf_root() {
        candidates.extend(collect_ggufs_in(&root.join(spec.repo.replace('/', "--"))));
    }
    if candidates.is_empty() {
        let hub = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".cache/huggingface/hub")
            .join(format!("models--{}", spec.repo.replace('/', "--")))
            .join("snapshots");
        if let Ok(snaps) = fs::read_dir(&hub) {
            for snap in snaps.flatten() {
                if snap.path().is_dir() {
                    candidates.extend(collect_ggufs_in(&snap.path()));
                }
            }
        }
    }
    if let Some(want) = &spec.file {
        let w = want.to_ascii_lowercase();
        if let Some(p) = candidates.iter().find(|p| {
            p.file_name()
                .map(|n| {
                    let n = n.to_string_lossy().to_ascii_lowercase();
                    n == w || n.contains(&w)
                })
                .unwrap_or(false)
        }) {
            return Ok(p.clone());
        }
    }
    if candidates.len() == 1 {
        return Ok(candidates.pop().unwrap());
    }
    if !candidates.is_empty() {
        let names: Vec<String> = candidates
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        let pick = pick_gguf_file(&names, spec.file.as_deref())?;
        if let Some(p) = candidates.iter().find(|c| {
            c.file_name()
                .map(|n| n.to_string_lossy() == pick)
                .unwrap_or(false)
        }) {
            return Ok(p.clone());
        }
    }
    Err(LateError::Message(format!(
        "{model} is not on disk yet. Click Download first (GGUF lands in {}).",
        gguf_root()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "~/.local/share/late/models/gguf".into())
    )))
}

fn ollama_native_root(base: &str) -> Result<String> {
    let trimmed = base.trim().trim_end_matches('/');
    let root = trimmed
        .strip_suffix("/v1")
        .or_else(|| trimmed.strip_suffix("/v1/"))
        .unwrap_or(trimmed);
    let url = reqwest::Url::parse(root)
        .map_err(|_| LateError::Message("invalid Ollama base URL".into()))?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(LateError::Message("Ollama URL must be http(s)".into()));
    }
    let host = url.host_str().unwrap_or("");
    if !is_loopback_hostname(host) {
        return Err(LateError::Message(
            "Ollama Pull only talks to loopback (127.0.0.1 / localhost / ::1)".into(),
        ));
    }
    Ok(root.trim_end_matches('/').to_string())
}

fn loopback_bind(base: &str) -> Result<std::net::SocketAddr> {
    let trimmed = base.trim().trim_end_matches('/');
    let root = trimmed.strip_suffix("/v1").unwrap_or(trimmed);
    let url = reqwest::Url::parse(root)
        .map_err(|_| LateError::Message("invalid llama.cpp base URL".into()))?;
    let host = url.host_str().unwrap_or("");
    if !is_loopback_hostname(host) {
        return Err(LateError::Message(
            "llama-server Start only binds loopback".into(),
        ));
    }
    let port = url.port_or_known_default().unwrap_or(8080);
    format!("127.0.0.1:{port}")
        .parse()
        .map_err(|_| LateError::Message("invalid llama.cpp port".into()))
}

fn probe_openai_models(url: &str) -> Vec<String> {
    let out = Command::new("curl")
        .args(["-sf", "-m", "1", url])
        .output()
        .ok();
    let Some(out) = out else {
        return vec![];
    };
    if !out.status.success() {
        return vec![];
    }
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).unwrap_or(serde_json::Value::Null);
    v.get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.get("id").and_then(|i| i.as_str()).map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

fn probe_ollama_tags(root: &str) -> Vec<String> {
    let url = format!("{root}/api/tags");
    let out = Command::new("curl")
        .args(["-sf", "-m", "1", &url])
        .output()
        .ok();
    let Some(out) = out else {
        return vec![];
    };
    if !out.status.success() {
        return vec![];
    }
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).unwrap_or(serde_json::Value::Null);
    v.get("models")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| {
                    x.get("name")
                        .or_else(|| x.get("model"))
                        .and_then(|i| i.as_str())
                        .map(|s| s.to_string())
                })
                .collect()
        })
        .unwrap_or_default()
}

fn ollama_reachable(root: &str) -> bool {
    let hostport = root
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .split('/')
        .next()
        .unwrap_or("127.0.0.1:11434");
    let addr: std::net::SocketAddr = match hostport.parse() {
        Ok(a) => a,
        Err(_) => format!("{hostport}:11434")
            .parse()
            .or_else(|_| "127.0.0.1:11434".parse())
            .unwrap_or_else(|_| std::net::SocketAddr::from(([127, 0, 0, 1], 11434))),
    };
    std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(150)).is_ok()
}

fn is_loopback_hostname(host: &str) -> bool {
    let h = host
        .trim()
        .trim_matches(|c| c == '[' || c == ']')
        .to_ascii_lowercase();
    h == "127.0.0.1" || h == "localhost" || h == "::1"
}

fn refuse_symlink(path: &Path) -> Result<()> {
    if path
        .symlink_metadata()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(LateError::Message(
            "refusing to write through a symbolic link".into(),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct OwnedPid {
    pid: u32,
    starttime: u64,
    comm: String,
}

impl OwnedPid {
    fn matches_live(&self) -> bool {
        if self.pid == 0 || self.starttime == 0 {
            return false;
        }
        proc_starttime(self.pid) == Some(self.starttime)
            && proc_comm(self.pid).is_some_and(|c| c == self.comm)
            && llama_comm_ok(&self.comm)
    }
}

fn llama_comm_ok(comm: &str) -> bool {
    comm == "llama-server" || comm == "llama-cpp-server" || comm == "llama-cpp-serve"
}

fn proc_comm(pid: u32) -> Option<String> {
    let raw = fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    let c = raw.trim();
    if c.is_empty() {
        None
    } else {
        Some(c.to_string())
    }
}

fn proc_starttime(pid: u32) -> Option<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = stat.rsplit_once(')')?.1;
    rest.split_whitespace().nth(19)?.parse().ok()
}

fn write_owned_pid(pid: u32) -> Result<()> {
    let comm = proc_comm(pid).unwrap_or_else(|| "llama-server".into());
    if !llama_comm_ok(&comm) {
        return Err(LateError::Message(
            "refusing to record a pid that is not llama-server".into(),
        ));
    }
    let starttime = proc_starttime(pid).unwrap_or(0);
    if starttime == 0 {
        return Err(LateError::Message(
            "could not read llama-server starttime; Stop will only kill the child we spawned"
                .into(),
        ));
    }
    write_private(
        &llama_pid_file(),
        format!("pid={pid}\nstarttime={starttime}\ncomm={comm}\n"),
    )
}

fn read_owned_pid() -> Option<OwnedPid> {
    let raw = fs::read_to_string(llama_pid_file()).ok()?;
    let mut pid = None;
    let mut starttime = None;
    let mut comm = None;
    for line in raw.lines() {
        if let Some(v) = line.strip_prefix("pid=") {
            pid = v.trim().parse().ok();
        } else if let Some(v) = line.strip_prefix("starttime=") {
            starttime = v.trim().parse().ok();
        } else if let Some(v) = line.strip_prefix("comm=") {
            comm = Some(v.trim().to_string());
        }
    }
    Some(OwnedPid {
        pid: pid?,
        starttime: starttime?,
        comm: comm?,
    })
}

fn owned_llama_running() -> bool {
    read_owned_pid().is_some_and(|rec| rec.matches_live())
}

fn redact_secrets(s: &str) -> String {
    let mut out = s.to_string();
    for key in ["HF_TOKEN", "HUGGING_FACE_HUB_TOKEN"] {
        if let Ok(val) = std::env::var(key) {
            if !val.is_empty() {
                out = out.replace(&val, "[redacted]");
            }
        }
    }
    out
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn tail_log(path: &Path) -> Option<String> {
    let raw = fs::read_to_string(path).ok()?;
    raw.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .next_back()
        .map(|l| l.chars().take(240).collect())
}

fn append_log(path: &Path, line: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        let _ = mkdir_private(parent);
    }
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    f.write_all(line.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_aliases() {
        assert_eq!(parse_engine("").unwrap(), Engine::Vllm);
        assert_eq!(parse_engine("local").unwrap(), Engine::Vllm);
        assert_eq!(parse_engine("llama.cpp").unwrap(), Engine::LlamaCpp);
        assert_eq!(parse_engine("llamacpp").unwrap(), Engine::LlamaCpp);
        assert_eq!(parse_engine("ollama").unwrap(), Engine::Ollama);
        assert!(parse_engine("chatgpt").is_err());
    }

    #[test]
    fn hub_spec_repo_and_quant() {
        let a = parse_hub_spec("Qwen/Qwen3-8B-GGUF").unwrap();
        assert_eq!(a.repo, "Qwen/Qwen3-8B-GGUF");
        assert_eq!(a.file, None);
        let b = parse_hub_spec("Qwen/Qwen3-8B-GGUF:Q4_K_M").unwrap();
        assert_eq!(b.file.as_deref(), Some("Q4_K_M"));
        let named =
            parse_hub_spec("https://huggingface.co/Qwen/Qwen3-8B-GGUF:Qwen3-8B-Q8_0.gguf").unwrap();
        assert_eq!(named.repo, "Qwen/Qwen3-8B-GGUF");
        assert_eq!(named.file.as_deref(), Some("Qwen3-8B-Q8_0.gguf"));
        let c = parse_hub_spec(
            "https://huggingface.co/Qwen/Qwen3-8B-GGUF/blob/main/Qwen3-8B-Q8_0.gguf",
        )
        .unwrap();
        assert_eq!(c.repo, "Qwen/Qwen3-8B-GGUF");
        assert_eq!(c.file, None);
        assert!(parse_hub_spec("llama3.2").is_err());
        assert!(parse_hub_spec("../etc/passwd").is_err());
    }

    #[test]
    fn ollama_maps_hub_ids_to_hf_co() {
        assert_eq!(ollama_pull_name("llama3.2").unwrap(), "llama3.2");
        assert_eq!(ollama_pull_name("qwen2.5:7b").unwrap(), "qwen2.5:7b");
        assert_eq!(
            ollama_pull_name("Qwen/Qwen3-8B-GGUF").unwrap(),
            "hf.co/Qwen/Qwen3-8B-GGUF"
        );
        assert_eq!(
            ollama_pull_name("hf.co/Qwen/Qwen3-8B-GGUF:Q8_0").unwrap(),
            "hf.co/Qwen/Qwen3-8B-GGUF:Q8_0"
        );
        assert_eq!(
            ollama_pull_name("huggingface.co/Qwen/Qwen3-8B-GGUF").unwrap(),
            "hf.co/Qwen/Qwen3-8B-GGUF"
        );
        assert_eq!(
            ollama_pull_name("https://huggingface.co/Qwen/Qwen3-8B-GGUF:Q8_0").unwrap(),
            "hf.co/Qwen/Qwen3-8B-GGUF:Q8_0"
        );
    }

    #[test]
    fn pick_prefers_q4_k_m() {
        let files = vec![
            "mmproj.gguf".into(),
            "Qwen3-8B-Q8_0.gguf".into(),
            "Qwen3-8B-Q4_K_M.gguf".into(),
        ];
        assert_eq!(
            pick_gguf_file(&files, None).unwrap(),
            "Qwen3-8B-Q4_K_M.gguf"
        );
        assert_eq!(
            pick_gguf_file(&files, Some("Q8_0")).unwrap(),
            "Qwen3-8B-Q8_0.gguf"
        );
        assert!(pick_gguf_file(&["config.json".into()], None).is_err());
        let bait = vec![
            "totally-innocent-q4_k_m-extra.gguf".into(),
            "Qwen3-8B-Q5_K_M.gguf".into(),
        ];
        assert_eq!(pick_gguf_file(&bait, None).unwrap(), "Qwen3-8B-Q5_K_M.gguf");
    }

    #[test]
    fn hf_hosts_allow_cdn_and_xet() {
        assert!(hf_download_host_ok("huggingface.co"));
        assert!(hf_download_host_ok("cdn-lfs.huggingface.co"));
        assert!(hf_download_host_ok("cas-bridge.xethub.hf.co"));
        assert!(!hf_download_host_ok("evil.example"));
        assert!(!hf_download_host_ok("huggingface.co.evil.com"));
        assert!(!hf_download_host_ok("evil.hf.co"));
        assert!(!hf_download_host_ok("hf.co.evil.com"));
    }

    #[test]
    fn gguf_leaf_name_rejects_slash() {
        assert!(safe_gguf_name("model.gguf").is_ok());
        assert!(safe_gguf_name("foo/bar.gguf").is_err());
        assert!(safe_gguf_name("../x.gguf").is_err());
    }

    #[test]
    fn gguf_magic_accepts_header() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("m.gguf");
        std::fs::write(&p, b"GGUFrest").unwrap();
        ensure_gguf_magic(&p).unwrap();
        std::fs::write(&p, b"PK\x03\x04").unwrap();
        assert!(ensure_gguf_magic(&p).is_err());
    }

    #[test]
    fn ollama_root_loopback_only() {
        assert!(ollama_native_root("http://127.0.0.1:11434/v1").is_ok());
        assert!(ollama_native_root("http://localhost:11434").is_ok());
        assert!(ollama_native_root("http://example.com:11434/v1").is_err());
        assert!(ollama_native_root("http://127.0.0.1.evil.com:11434/v1").is_err());
    }

    #[test]
    fn llama_bind_loopback_only() {
        assert_eq!(
            loopback_bind("http://127.0.0.1:8080/v1").unwrap().port(),
            8080
        );
        assert!(loopback_bind("http://10.0.0.5:8080/v1").is_err());
    }

    #[test]
    fn catalogs_include_gemma_and_stay_sfw() {
        let gguf: String = GGUF_LIBRARY
            .iter()
            .map(|(id, _, _)| *id)
            .collect::<Vec<_>>()
            .join(" ");
        let ollama: String = OLLAMA_LIBRARY
            .iter()
            .map(|(id, _, _)| *id)
            .collect::<Vec<_>>()
            .join(" ");
        assert!(gguf.contains("google/gemma-4-E4B-it-qat-q4_0-gguf"));
        assert!(gguf.contains("google/gemma-3-4b-it-qat-q4_0-gguf"));
        assert!(gguf.contains("Qwen/Qwen3-8B-GGUF"));
        assert!(gguf.contains("unsloth/Qwen3.8-27B-GGUF"));
        assert!(gguf.contains("Qwen/Qwen2.5-7B-Instruct-GGUF"));
        assert!(ollama.contains("gemma4:e4b"));
        assert!(ollama.contains("gemma3:4b"));
        assert!(ollama.contains("qwen3.8:27b"));
        assert!(ollama.contains("qwen3:8b"));
        assert!(ollama.contains("qwen2.5:7b"));
        assert!(ollama.contains("llama4:scout"));
        for blob in [gguf.as_str(), ollama.as_str()] {
            let lower = blob.to_ascii_lowercase();
            assert!(!lower.contains("uncensored"));
            assert!(!lower.contains("abliterat"));
            assert!(!lower.contains("dolphin"));
        }
    }

    fn gpu8() -> crate::hardware::GpuProfile {
        crate::hardware::GpuProfile {
            vendor: "nvidia".into(),
            discrete_count: 1,
            vram_gb: 8,
            vram_bytes_each: 8 << 30,
            tp_ok: false,
            summary: "test".into(),
            cards: vec![],
        }
    }

    #[test]
    fn gguf_and_ollama_list_full_catalog_newest_wins() {
        let gguf = crate::hardware::rank_quant_catalog(GGUF_LIBRARY, &gpu8());
        assert_eq!(gguf.len(), GGUF_LIBRARY.len());
        assert!(GGUF_LIBRARY.len() > 15, "GGUF catalog is not a 2-row slice");
        assert!(gguf.iter().any(|r| r.id == "Qwen/Qwen3-8B-GGUF" && r.newest && r.recommended));
        assert!(gguf
            .iter()
            .any(|r| r.id == "Qwen/Qwen2.5-7B-Instruct-GGUF" && !r.newest));
        assert!(gguf
            .iter()
            .any(|r| r.id == "google/gemma-4-E4B-it-qat-q4_0-gguf" && r.newest));
        assert!(gguf
            .iter()
            .any(|r| r.id == "google/gemma-3-4b-it-qat-q4_0-gguf" && !r.newest));
        let too_big = gguf
            .iter()
            .find(|r| r.id.contains("70B") || r.id.contains("32B-GGUF") || r.id.contains("Scout"));
        assert!(too_big.is_some());
        assert!(!too_big.unwrap().recommended);

        let ollama = crate::hardware::rank_quant_catalog(OLLAMA_LIBRARY, &gpu8());
        assert_eq!(ollama.len(), OLLAMA_LIBRARY.len());
        assert!(OLLAMA_LIBRARY.len() > 20, "Ollama catalog is not a 2-row slice");
        assert!(ollama.iter().any(|r| r.id == "qwen3:8b" && r.newest && r.recommended));
        assert!(ollama.iter().any(|r| r.id == "qwen2.5:7b" && !r.newest));
        assert!(ollama.iter().any(|r| r.id == "gemma4:e4b" && r.newest));
        assert!(ollama.iter().any(|r| r.id == "gemma3:4b" && !r.newest));
        assert!(ollama.iter().any(|r| r.id == "llama3.3" && !r.recommended));
        assert!(ollama.iter().any(|r| r.id == "qwen3.8:27b" && r.newest && !r.recommended));
    }
}
