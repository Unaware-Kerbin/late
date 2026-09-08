//! Late-owned OpenAI-compatible `/v1` for Local Agent chat.
//!
//! Loads Hugging Face **safetensors** Instruct weights (default
//! `Qwen/Qwen2.5-0.5B-Instruct`) and listens on **loopback only**.
//! Qwen2.5 uses ChatML / Hermes `<tool_call>`. Gemma 4 Instruct uses
//! `<|turn>` / `<|tool_call>call:name{…}` (not the same as Qwen).

mod bind;
mod compiler;
mod device;
mod engine;
mod gemma4;
mod host_ram;
mod intel;
mod openai;
mod prompt;
mod template;

use anyhow::Context;
use axum::extract::Request;
use axum::extract::State;
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bind::parse_loopback_bind;
use clap::Parser;
use compiler::{CompileManifest, COMPILER_ID};
use engine::{Engine, Family, DEFAULT_MODEL};
use openai::{
    error_json, ChatRequest, ChatResponse, Choice, ModelCard, ModelsResponse, ResponseMessage,
    Usage,
};
use prompt::{
    apply_gemma_chat_template, apply_gemma_chat_template_with_tools, apply_qwen_chat_template,
    apply_qwen_chat_template_with_tools, parse_gemma_tool_calls, parse_hermes_tool_calls,
    text_before_gemma_tool_calls, text_before_tool_calls,
};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(
    name = "late-infer",
    about = "Late-owned Hugging Face safetensors server (OpenAI /v1, loopback only)",
    after_help = "\
Environment:
  LATE_INFER_ACCEL=intel|nvidia|amd
      Pin the GPU vendor on your computer. Empty is not NVIDIA — late-infer
      probes PCI/DRM. Orchestrator sets this from idle-first detect (not a
      hardcoded intel default): nvidia = CUDA, intel = OpenVINO GenAI /
      Level Zero, amd = HIP/ROCm or fail closed. CPU is not the discrete card.
  LATE_INFER_PCI
      PCI slot from detect (example 0000:08:00.0). Recorded on late-compile.json.
  LATE_INFER_INTEL_PYTHON
      Python with openvino_genai (default ~/.local/share/late/intel-ov).
  ONEAPI_DEVICE_SELECTOR / ZE_AFFINITY_MASK / ZE_FLAT_DEVICE_HIERARCHY
      Intel toolkit. ZE_AFFINITY_MASK is a Level Zero index among discrete
      XPUs (iGPU may enumerate separately). Idle-primary sets this; do not
      assume 0 is the card you want.
  CUDA_VISIBLE_DEVICES
      NVIDIA only. Ignored / mismatch if LATE_INFER_ACCEL=intel or PCI is Intel-only.
  HIP_VISIBLE_DEVICES / ROCR_VISIBLE_DEVICES
      AMD only.

Serve binds loopback 127.0.0.1 only. --compile-only fetches a Hub
snapshot on loopback and exits; it does not listen."
)]
struct Args {
    /// Bind address. Loopback only (127.0.0.1 / ::1).
    #[arg(long, default_value = "127.0.0.1:8010")]
    bind: String,

    /// Hugging Face Hub id (safetensors Instruct).
    #[arg(long, default_value = DEFAULT_MODEL)]
    model: String,

    #[arg(long, default_value = "main")]
    revision: String,

    /// Skip CUDA/Metal. Does not turn an Intel/AMD card into CPU; those still fail closed.
    #[arg(long)]
    cpu: bool,

    /// Fetch Hub snapshot, classify, materialize the compiled blob, then exit.
    /// Does not bind a server (nothing on :8010).
    #[arg(long)]
    compile_only: bool,
}

struct AppState {
    engine: Mutex<Engine>,
    model_id: String,
    device_label: String,
    family: Family,
    chat_template: Option<String>,
    compiler: String,
    serve: String,
    model_type: String,
    weights_in_host_ram: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let args = Args::parse();
    let plan = device::resolve_from_env().map_err(|e| anyhow::anyhow!("{e}"))?;
    if args.compile_only {
        device::compile_gate(&plan).map_err(|e| anyhow::anyhow!("{e}"))?;
        let model_id = args.model.clone();
        let revision = args.revision.clone();
        let manifest = tokio::task::spawn_blocking(move || engine::compile_hub(&model_id, &revision))
            .await
            .context("join compile")??;
        println!("{}", compile_only_json(&manifest));
        return Ok(());
    }
    device::serve_gate(&plan).map_err(|e| anyhow::anyhow!("{e}"))?;

    let bind = parse_loopback_bind(&args.bind)?;
    let intel_serve = plan.accel == Some(device::AccelKind::Intel);
    let model_id = args.model.clone();
    let revision = args.revision.clone();

    let engine = if intel_serve {
        if args.cpu {
            anyhow::bail!(
                "LATE_INFER_ACCEL=intel on your computer needs the Intel GPU (OpenVINO GenAI). --cpu is not that card."
            );
        }
        let (_, device_label) = device::pick(false, &plan).map_err(|e| anyhow::anyhow!("{e}"))?;
        tracing::info!("device: {device_label}");
        tokio::task::spawn_blocking(move || Engine::load_intel(&model_id, &revision, device_label))
            .await
            .context("join load")??
    } else {
        let (dev, device_label) = device::pick(args.cpu, &plan).map_err(|e| anyhow::anyhow!("{e}"))?;
        tracing::info!("device: {device_label}");
        tokio::task::spawn_blocking(move || Engine::load(&model_id, &revision, dev, device_label))
            .await
            .context("join load")??
    };

    let served = engine.model_id.clone();
    let device_label = engine.device_label.clone();
    let family = engine.family;
    let chat_template = engine.chat_template.clone();
    let compiler = engine.compile_manifest.compiler.clone();
    let serve = engine.compile_manifest.serve.clone();
    let model_type = engine.compile_manifest.model_type.clone();
    let weights_in_host_ram = engine.weights_in_host_ram();
    tracing::info!(
        "loaded {served} on {device_label} family={family:?} compiler={compiler} serve={serve}"
    );
    let state = Arc::new(AppState {
        engine: Mutex::new(engine),
        model_id: served.clone(),
        device_label,
        family,
        chat_template,
        compiler,
        serve,
        model_type,
        weights_in_host_ram,
    });

    let router = Router::new()
        .route("/health", get(health))
        .route("/v1/models", get(list_models))
        .route("/models", get(list_models))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/chat/completions", post(chat_completions))
        .layer(middleware::from_fn(loopback_cors))
        .with_state(state);

    tracing::info!("late-infer listening on http://{bind}/v1  model={served} compiler={COMPILER_ID}");
    let listener = tokio::net::TcpListener::bind(bind).await?;
    axum::serve(listener, router).await?;
    Ok(())
}

async fn health(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    Json(health_body(
        &st.model_id,
        &st.device_label,
        &st.compiler,
        &st.serve,
        &st.model_type,
        st.weights_in_host_ram,
    ))
}

fn health_device_meta(device: &str, serve: &str) -> (String, String) {
    let d = device.to_ascii_lowercase();
    let s = serve.to_ascii_lowercase();
    if s.contains("openvino") || d.contains("intel-xpu") || d.contains("openvino") || d.contains("xpu") {
        return ("intel".into(), "intel-xpu".into());
    }
    if d.contains("cuda") {
        return ("nvidia".into(), "cuda".into());
    }
    if d.contains("hip") || d.contains("rocm") {
        return ("amd".into(), "hip".into());
    }
    if d.contains("cpu") {
        return ("cpu".into(), "cpu".into());
    }
    (String::new(), String::new())
}

fn health_body(
    model: &str,
    device: &str,
    compiler: &str,
    serve: &str,
    model_type: &str,
    weights_in_host_ram: bool,
) -> serde_json::Value {
    let (accel, device_kind) = health_device_meta(device, serve);
    serde_json::json!({
        "ok": true,
        "name": "late-infer",
        "model": model,
        "device": device,
        "compiler": compiler,
        "serve": serve,
        "model_type": model_type,
        "accel": accel,
        "device_kind": device_kind,
        "weights_in_host_ram": weights_in_host_ram,
    })
}

fn is_loopback_origin(raw: &str) -> bool {
    let rest = raw
        .trim()
        .strip_prefix("http://")
        .or_else(|| raw.trim().strip_prefix("https://"));
    let Some(rest) = rest else { return false };
    let host = rest.split('/').next().unwrap_or("").split(':').next().unwrap_or("");
    matches!(host, "127.0.0.1" | "localhost" | "[::1]" | "::1")
}

async fn loopback_cors(req: Request, next: Next) -> Response {
    let origin = req.headers().get(header::ORIGIN).cloned();
    if req.method() == Method::OPTIONS {
        let mut res = Response::new(Default::default());
        *res.status_mut() = StatusCode::NO_CONTENT;
        apply_loopback_cors(&mut res, origin.as_ref());
        return res;
    }
    let mut res = next.run(req).await;
    apply_loopback_cors(&mut res, origin.as_ref());
    res
}

fn apply_loopback_cors(res: &mut Response, origin: Option<&HeaderValue>) {
    let Some(origin) = origin else { return };
    let Ok(raw) = origin.to_str() else { return };
    if !is_loopback_origin(raw) {
        return;
    }
    if let Ok(value) = HeaderValue::from_str(raw) {
        res.headers_mut().insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, value);
    }
    res.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, OPTIONS"),
    );
    res.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("accept, authorization, content-type"),
    );
}

/// Public compile-only stdout: identity only, no snapshot paths / cwd / allowlist.
fn compile_only_json(manifest: &CompileManifest) -> String {
    serde_json::json!({
        "ok": manifest.status == "ok",
        "compiler": manifest.compiler,
        "compiler_project": manifest.compiler_project,
        "model": manifest.model_id,
        "model_type": manifest.model_type,
        "status": manifest.status,
        "serve": manifest.serve,
        "vendor": manifest.vendor,
        "accel": manifest.accel,
        "pci_id": manifest.pci_id,
        "idle": manifest.idle,
        "display": manifest.display,
        "ir": manifest.ir.unwrap_or(false),
    })
    .to_string()
}

fn family_prompt(
    family: Family,
    messages: &[openai::ChatMessage],
    tools: Option<&[serde_json::Value]>,
    required: Option<&str>,
) -> String {
    let tools_on = tools.is_some_and(|t| !t.is_empty());
    match family {
        Family::Qwen2 => {
            if tools_on {
                apply_qwen_chat_template_with_tools(messages, tools, required)
            } else {
                apply_qwen_chat_template(messages)
            }
        }
        Family::Gemma4 => {
            if tools_on {
                apply_gemma_chat_template_with_tools(messages, tools, required)
            } else {
                apply_gemma_chat_template(messages)
            }
        }
    }
}

async fn list_models(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    Json(ModelsResponse {
        object: "list",
        data: vec![ModelCard {
            id: st.model_id.clone(),
            object: "model",
            owned_by: "late",
        }],
    })
}

async fn chat_completions(
    State(st): State<Arc<AppState>>,
    Json(req): Json<ChatRequest>,
) -> impl IntoResponse {
    if let Some((message, kind)) = req.reject_reason() {
        return (
            StatusCode::BAD_REQUEST,
            Json(error_json(message, kind)),
        )
            .into_response();
    }

    let tools_on = req.tools_active();
    let required = req
        .tool_choice
        .as_ref()
        .and_then(|c| c.required_name());
    let tools = if tools_on {
        req.tools.as_deref()
    } else {
        None
    };
    let prompt = st
        .chat_template
        .as_deref()
        .and_then(|tmpl| template::apply_hub_chat_template(tmpl, &req.messages, tools, true))
        .unwrap_or_else(|| family_prompt(st.family, &req.messages, tools, required.as_deref()));
    let max_tokens = req.max_tokens.unwrap_or(512).clamp(1, 2048) as usize;
    let temperature = req.temperature;
    let top_p = req.top_p;
    let model_name = if req.model.is_empty() {
        st.model_id.clone()
    } else {
        req.model.clone()
    };

    let family = st.family;
    let result = tokio::task::spawn_blocking(move || {
        let mut eng = st.engine.blocking_lock();
        eng.generate(&prompt, max_tokens, temperature, top_p)
    })
    .await;

    match result {
        Ok(Ok(out)) => {
            let created = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let calls = match family {
                Family::Qwen2 => parse_hermes_tool_calls(&out.text),
                Family::Gemma4 => parse_gemma_tool_calls(&out.text),
            };
            let (content, tool_calls, finish) = if calls.is_empty() {
                (Some(out.text), None, out.finish_reason)
            } else {
                let preamble = match family {
                    Family::Qwen2 => text_before_tool_calls(&out.text),
                    Family::Gemma4 => text_before_gemma_tool_calls(&out.text),
                };
                (preamble, Some(calls), "tool_calls")
            };
            Json(ChatResponse {
                id: format!("late-infer-{created}"),
                object: "chat.completion",
                created,
                model: model_name,
                choices: vec![Choice {
                    index: 0,
                    message: ResponseMessage {
                        role: "assistant",
                        content,
                        tool_calls,
                    },
                    finish_reason: finish,
                }],
                usage: Usage {
                    prompt_tokens: out.prompt_tokens,
                    completion_tokens: out.completion_tokens,
                    total_tokens: out.prompt_tokens + out.completion_tokens,
                },
            })
            .into_response()
        }
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(error_json(e.to_string(), "server_error")),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(error_json(format!("join: {e}"), "server_error")),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::{compile_only_json, family_prompt, health_body, Args};
    use crate::compiler::CompileManifest;
    use crate::engine::Family;
    use crate::openai::{ChatMessage, Content};
    use clap::{CommandFactory, Parser};

    fn msg(role: &str, text: &str) -> ChatMessage {
        ChatMessage {
            role: role.into(),
            content: Content::Text(text.into()),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        }
    }

    #[test]
    fn health_includes_compiler_serve_model_type() {
        let v = health_body(
            "Qwen/Qwen2.5-0.5B-Instruct",
            "cpu",
            crate::compiler::COMPILER_ID,
            "candle-fallback",
            "qwen2",
            true,
        );
        assert_eq!(v["compiler"], "mlc-llm");
        assert_eq!(v["serve"], "candle-fallback");
        assert_eq!(v["model_type"], "qwen2");
        assert_eq!(v["ok"], true);
        assert_eq!(v["device_kind"], "cpu");
        assert_eq!(v["weights_in_host_ram"], true);
    }

    #[test]
    fn health_intel_xpu_is_gpu_not_host_ram() {
        let v = health_body(
            "Qwen/Qwen2.5-0.5B-Instruct",
            "intel-xpu:openvino-genai · 0000:08:00.0",
            crate::compiler::COMPILER_ID,
            "openvino-genai",
            "qwen2",
            false,
        );
        assert_eq!(v["device_kind"], "intel-xpu");
        assert_eq!(v["accel"], "intel");
        assert_eq!(v["weights_in_host_ram"], false);
    }

    #[test]
    fn health_cuda_is_gpu_not_host_ram() {
        let v = health_body(
            "Qwen/Qwen2.5-0.5B-Instruct",
            "cuda:0",
            crate::compiler::COMPILER_ID,
            "candle-cuda",
            "qwen2",
            false,
        );
        assert_eq!(v["device_kind"], "cuda");
        assert_eq!(v["weights_in_host_ram"], false);
    }

    #[test]
    fn family_prompt_falls_back_to_hermes_when_no_hub_template() {
        let s = family_prompt(Family::Qwen2, &[msg("user", "hi")], None, None);
        assert!(s.contains("<|im_start|>user\nhi"));
    }

    #[test]
    fn compile_only_flag_parses_without_bind() {
        let a = Args::parse_from([
            "late-infer",
            "--compile-only",
            "--model",
            "Qwen/Qwen2.5-0.5B-Instruct",
        ]);
        assert!(a.compile_only);
        assert_eq!(a.model, "Qwen/Qwen2.5-0.5B-Instruct");
    }

    #[test]
    fn help_documents_late_infer_accel() {
        let mut cmd = Args::command();
        let mut buf = Vec::new();
        cmd.write_long_help(&mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("LATE_INFER_ACCEL=intel|nvidia|amd"), "{s}");
        assert!(s.contains("intel"), "{s}");
        assert!(s.contains("nvidia"), "{s}");
        assert!(s.contains("amd"), "{s}");
        assert!(s.contains("OpenVINO"), "{s}");
        assert!(s.contains("ZE_AFFINITY_MASK"), "{s}");
        assert!(s.contains("ONEAPI_DEVICE_SELECTOR"), "{s}");
        assert!(s.contains("iGPU"), "{s}");
        assert!(s.contains("127.0.0.1"), "{s}");
        assert!(s.contains("--cpu"), "{s}");
        assert!(!s.contains("0.0.0.0"), "{s}");
    }

    #[test]
    fn compile_only_json_has_no_filesystem_fields() {
        let m = CompileManifest {
            compiler: crate::compiler::COMPILER_ID.into(),
            compiler_project: crate::compiler::COMPILER_PROJECT.into(),
            model_id: "Qwen/Qwen2.5-0.5B-Instruct".into(),
            model_type: "qwen2".into(),
            status: "ok".into(),
            serve: "candle-fallback".into(),
            error: None,
            vendor: None,
            accel: None,
            pci_id: None,
            idle: None,
            display: None,
            ir: None,
            snapshot: crate::compiler::SnapshotFiles::default(),
        };
        let raw = compile_only_json(&m);
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["compiler"], "mlc-llm");
        assert_eq!(v["serve"], "candle-fallback");
        assert_eq!(v["ir"], false);
        assert_eq!(v["model_type"], "qwen2");
        assert!(v.get("accel").is_some());
        assert!(v.get("vendor").is_some());
        assert!(v.get("snapshot").is_none());
        assert!(v.get("cwd").is_none());
        assert!(v.get("allowlist").is_none());
        assert!(v.get("dest").is_none());
        assert!(v.get("write_dir").is_none());
        assert!(v.get("filesystem").is_none());
        let dump = raw.to_lowercase();
        assert!(!dump.contains("allowlist"));
        assert!(!dump.contains("write_dir"));
        assert!(!dump.contains("\"cwd\""));
    }
}
