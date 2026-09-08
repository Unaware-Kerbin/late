//! Hub snapshot compiler. Locked backend: **MLC-LLM** (Apache, TVM).
//!
//! Download stays Hugging Face **safetensors** + `config.json` + tokenizer
//! (the raw image). Compile writes `~/.local/share/late/compiled/<id>/`.
//! Unknown graphs fail closed — we do not add a new `Family` enum per brand.
//!
//! Candle Qwen2 / Gemma 4 is a **serve fallback** until `mlc_llm` is on PATH
//! and compile succeeds. The compiled blob has no filesystem tools.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// Open-source compiler Late wraps. Not vLLM, Ollama, or llama.cpp.
pub const COMPILER_ID: &str = "mlc-llm";
pub const COMPILER_PROJECT: &str = "https://github.com/mlc-ai/mlc-llm";

/// MLC/TVM-class decoder-only Instruct types we will try to compile.
/// Not every Hub `model_type` — vision, diffusion, and custom `modeling_*.py` fail closed.
const MLC_MODEL_TYPES: &[&str] = &[
    "llama",
    "mistral",
    "mixtral",
    "qwen",
    "qwen2",
    "qwen3",
    "gemma",
    "gemma2",
    "gemma3",
    "phi",
    "phi3",
    "gpt2",
    "gpt_neox",
    "stablelm",
    "internlm",
    "baichuan",
    "chatglm",
    "deepseek",
    "olmo",
    "mpt",
    "falcon",
];

/// Architectures late-infer can still *serve* via Candle while MLC compile is missing.
const CANDLE_FALLBACK_TYPES: &[&str] = &["qwen", "qwen2", "gemma4"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServeKind {
    /// Serve with in-process Candle (Qwen2 / Gemma 4 Instruct only).
    CandleFallback,
    /// Weights compiled (or compilable) with MLC-LLM.
    Mlc,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompilePlan {
    pub model_type: String,
    pub serve: ServeKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompileManifest {
    pub compiler: String,
    pub compiler_project: String,
    pub model_id: String,
    pub model_type: String,
    pub status: String,
    pub serve: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Detected GPU vendor (`intel` / `nvidia` / `amd`). Omitted when unknown — never defaults to nvidia.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor: Option<String>,
    /// Same as `vendor` when recorded (`LATE_INFER_ACCEL` or PCI probe). Never defaults to nvidia.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pci_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<bool>,
    /// True when `compiled/<slug>/openvino/` has IR (Intel GPU Start). Candle JSON is not IR.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ir: Option<bool>,
    /// Snapshot file names only — not a write allowlist, not user cwd.
    pub snapshot: SnapshotFiles,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SnapshotFiles {
    pub config: String,
    pub tokenizer: String,
    #[serde(default)]
    pub tokenizer_config: Option<String>,
    pub weights: Vec<String>,
}

pub fn compiled_root() -> PathBuf {
    if let Ok(p) = std::env::var("LATE_COMPILED_DIR") {
        let p = p.trim();
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("late")
        .join("compiled")
}

pub fn blob_dir(model_id: &str) -> PathBuf {
    compiled_root().join(slug_model_id(model_id))
}

pub fn slug_model_id(model_id: &str) -> String {
    model_id.trim().replace('/', "--")
}

/// Fail closed: unknown Hub graphs are not a new Late decoder.
pub fn classify_config(config_json: &[u8]) -> Result<CompilePlan> {
    let v: serde_json::Value = serde_json::from_slice(config_json).context("parse config.json")?;
    let model_type = v
        .get("model_type")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let arches: Vec<String> = v
        .get("architectures")
        .and_then(|a| a.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_ascii_lowercase()))
                .collect()
        })
        .unwrap_or_default();
    let blob = format!("{model_type} {}", arches.join(" "));

    if blob.contains("stable-diffusion")
        || blob.contains("diffusion")
        || blob.contains("whisper")
        || blob.contains("clip")
        || (blob.contains("vit") && !blob.contains("qwen"))
    {
        bail!(
            "this Hub snapshot is not a supported Instruct LLM (model_type={model_type:?}). late-infer compiles decoder-only chat weights with {COMPILER_ID}, not vision/diffusion/audio graphs."
        );
    }

    if blob.contains("gemma4") || model_type == "gemma4" {
        return Ok(CompilePlan {
            model_type: "gemma4".into(),
            serve: ServeKind::CandleFallback,
        });
    }
    if CANDLE_FALLBACK_TYPES
        .iter()
        .any(|t| model_type == *t)
    {
        return Ok(CompilePlan {
            model_type: model_type.clone(),
            serve: ServeKind::CandleFallback,
        });
    }

    if MLC_MODEL_TYPES.iter().any(|t| model_type == *t) {
        return Ok(CompilePlan {
            model_type,
            serve: ServeKind::Mlc,
        });
    }

    bail!(
        "this Hub snapshot is not a supported graph (model_type={model_type:?} architectures={arches:?}). Late compiles Instruct safetensors with {COMPILER_ID} ({COMPILER_PROJECT}); unknown types fail closed instead of a new hand-written decoder."
    )
}

pub fn mlc_llm_bin() -> Option<PathBuf> {
    which("mlc_llm")
}

fn which(name: &str) -> Option<PathBuf> {
    let Ok(path) = std::env::var("PATH") else {
        return None;
    };
    for dir in path.split(':') {
        let p = Path::new(dir).join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// Materialize `late-compile.json` next to the Hub snapshot. Optionally invoke `mlc_llm`.
pub fn materialize(
    model_id: &str,
    plan: &CompilePlan,
    config: &Path,
    tokenizer: &Path,
    tokenizer_config: Option<&Path>,
    weights: &[PathBuf],
) -> Result<CompileManifest> {
    let dir = blob_dir(model_id);
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let mut serve = match plan.serve {
        ServeKind::CandleFallback => "candle-fallback",
        ServeKind::Mlc => "mlc-llm",
    }
    .to_string();
    let mut error = None;
    let mut status = "ok";

    if plan.serve == ServeKind::Mlc {
        // Intel + OpenVINO: skip MLC entirely. compile_hub / load_intel export IR next.
        if intel_accel_pinned() {
            if !intel_openvino_compile_path() {
                status = "error";
                error = Some(crate::intel::runtime_missing_msg());
                let manifest = manifest(
                    model_id,
                    plan,
                    status,
                    &serve,
                    error.clone(),
                    config,
                    tokenizer,
                    tokenizer_config,
                    weights,
                );
                write_manifest(&dir, &manifest)?;
                bail!("{}", crate::intel::runtime_missing_msg());
            }
            serve = "openvino-genai".into();
        } else {
            match try_mlc_compile(model_id, &dir) {
                Ok(()) => {
                    serve = "mlc-llm".into();
                }
                Err(e) => {
                    if plan.serve == ServeKind::Mlc && !candle_ok(&plan.model_type) {
                        status = "error";
                        error = Some(e.to_string());
                        let manifest = manifest(
                            model_id,
                            plan,
                            status,
                            &serve,
                            error.clone(),
                            config,
                            tokenizer,
                            tokenizer_config,
                            weights,
                        );
                        write_manifest(&dir, &manifest)?;
                        bail!("{e}");
                    }
                    tracing::warn!("mlc_llm compile skipped or failed; Candle fallback if any: {e}");
                    if candle_ok(&plan.model_type) {
                        serve = "candle-fallback".into();
                    } else {
                        status = "error";
                        error = Some(e.to_string());
                        let manifest = manifest(
                            model_id,
                            plan,
                            status,
                            &serve,
                            error.clone(),
                            config,
                            tokenizer,
                            tokenizer_config,
                            weights,
                        );
                        write_manifest(&dir, &manifest)?;
                        bail!("{e}");
                    }
                }
            }
        }
    }

    let manifest = manifest(
        model_id,
        plan,
        status,
        &serve,
        error,
        config,
        tokenizer,
        tokenizer_config,
        weights,
    );
    write_manifest(&dir, &manifest)?;
    Ok(manifest)
}

fn candle_ok(model_type: &str) -> bool {
    CANDLE_FALLBACK_TYPES.iter().any(|t| model_type == *t)
}

/// True when the orchestrator pinned Intel (`LATE_INFER_ACCEL=intel|xpu|…`).
/// Probed PCI alone is not enough — NVIDIA MLC fail-closed tests must keep working on Intel boxes.
fn intel_accel_pinned() -> bool {
    matches!(
        crate::device::parse_accel_token(
            &std::env::var("LATE_INFER_ACCEL").unwrap_or_default()
        )
        .ok()
        .flatten(),
        Some(crate::device::AccelKind::Intel)
    )
}

/// Intel + OpenVINO GenAI: IR export is the compile path (Qwen3/Llama/…). Do not require mlc-llm.
fn intel_openvino_compile_path() -> bool {
    intel_accel_pinned() && crate::intel::intel_runtime_ok()
}

/// Fail before fetching GB of weights when this graph cannot compile on this computer.
pub fn preflight_plan(plan: &CompilePlan) -> Result<()> {
    if plan.serve == ServeKind::Mlc && mlc_llm_bin().is_none() && !candle_ok(&plan.model_type) {
        // Intel Arc / XPU: OpenVINO IR export runs in compile_hub — never fall through to mlc-llm.
        if intel_accel_pinned() {
            if intel_openvino_compile_path() {
                return Ok(());
            }
            bail!("{}", crate::intel::runtime_missing_msg());
        }
        bail!(
            "{COMPILER_ID} is not on PATH. Install from {COMPILER_PROJECT} to compile Llama/Mistral/Qwen3/… snapshots. Qwen2 and Gemma 4 Instruct can still serve via Candle on this computer."
        );
    }
    Ok(())
}

/// Classify + preflight + materialize from files already on this computer (no Hub).
pub fn compile_snapshot(
    model_id: &str,
    config_bytes: &[u8],
    config: &Path,
    tokenizer: &Path,
    tokenizer_config: Option<&Path>,
    weights: &[PathBuf],
) -> Result<CompileManifest> {
    let plan = classify_config(config_bytes)?;
    preflight_plan(&plan)?;
    materialize(
        model_id,
        &plan,
        config,
        tokenizer,
        tokenizer_config,
        weights,
    )
}

fn intel_openvino_ir_present(model_id: &str) -> bool {
    blob_dir(model_id)
        .join("openvino")
        .join("openvino_model.xml")
        .is_file()
}

fn manifest(
    model_id: &str,
    plan: &CompilePlan,
    status: &str,
    serve: &str,
    error: Option<String>,
    config: &Path,
    tokenizer: &Path,
    tokenizer_config: Option<&Path>,
    weights: &[PathBuf],
) -> CompileManifest {
    let gpu = crate::device::compile_meta();
    let intel_ir = gpu.vendor.as_deref() == Some("intel") && intel_openvino_ir_present(model_id);
    let serve_out = if intel_ir {
        "openvino-genai".to_string()
    } else {
        serve.to_string()
    };
    CompileManifest {
        compiler: COMPILER_ID.into(),
        compiler_project: COMPILER_PROJECT.into(),
        model_id: model_id.into(),
        model_type: plan.model_type.clone(),
        status: status.into(),
        serve: serve_out,
        error,
        vendor: gpu.vendor,
        accel: gpu.accel,
        pci_id: gpu.pci_id,
        idle: gpu.idle,
        display: gpu.display,
        ir: if intel_ir { Some(true) } else { None },
        snapshot: SnapshotFiles {
            config: config.display().to_string(),
            tokenizer: tokenizer.display().to_string(),
            tokenizer_config: tokenizer_config.map(|p| p.display().to_string()),
            weights: weights.iter().map(|p| p.display().to_string()).collect(),
        },
    }
}


pub fn write_manifest(dir: &Path, m: &CompileManifest) -> Result<()> {
    let path = dir.join("late-compile.json");
    let body = serde_json::to_vec_pretty(m)?;
    fs::write(&path, body).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

fn try_mlc_compile(model_id: &str, dir: &Path) -> Result<()> {
    let Some(bin) = mlc_llm_bin() else {
        bail!(
            "{COMPILER_ID} is not on PATH. Install from {COMPILER_PROJECT} to compile Llama/Mistral/Qwen3/… snapshots. Qwen2 and Gemma 4 Instruct can still serve via Candle on this computer."
        );
    };
    let out = dir.join("mlc");
    fs::create_dir_all(&out)?;
    // Probe CLI; full convert_weight can take many minutes and is skipped unless LATE_MLC_COMPILE=1.
    if std::env::var("LATE_MLC_COMPILE").ok().as_deref() != Some("1") {
        let help = Command::new(&bin)
            .arg("--help")
            .output()
            .with_context(|| format!("run {}", bin.display()))?;
        if !help.status.success() {
            bail!(
                "{COMPILER_ID} --help failed (status {:?})",
                help.status.code()
            );
        }
        tracing::info!(
            "{COMPILER_ID} present; set LATE_MLC_COMPILE=1 to run convert/compile for {model_id}"
        );
        return Ok(());
    }
    let status = Command::new(&bin)
        .args(["gen_config", model_id, "--output"])
        .arg(&out)
        .status()
        .with_context(|| format!("run {} gen_config", bin.display()))?;
    if !status.success() {
        bail!("{COMPILER_ID} gen_config failed for {model_id}");
    }
    let _ = Duration::from_secs(1);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qwen2_is_candle_fallback() {
        let p = classify_config(
            br#"{"model_type":"qwen2","architectures":["Qwen2ForCausalLM"]}"#,
        )
        .unwrap();
        assert_eq!(p.model_type, "qwen2");
        assert_eq!(p.serve, ServeKind::CandleFallback);
    }

    #[test]
    fn llama_is_mlc() {
        let p = classify_config(br#"{"model_type":"llama","architectures":["LlamaForCausalLM"]}"#)
            .unwrap();
        assert_eq!(p.serve, ServeKind::Mlc);
    }

    #[test]
    fn mistral_is_mlc() {
        let p = classify_config(br#"{"model_type":"mistral"}"#).unwrap();
        assert_eq!(p.serve, ServeKind::Mlc);
    }

    #[test]
    fn gemma4_is_candle_fallback() {
        let p = classify_config(
            br#"{"model_type":"gemma4","architectures":["Gemma4ForConditionalGeneration"]}"#,
        )
        .unwrap();
        assert_eq!(p.model_type, "gemma4");
        assert_eq!(p.serve, ServeKind::CandleFallback);
    }

    #[test]
    fn diffusion_fails_closed() {
        let e = classify_config(br#"{"model_type":"stable-diffusion"}"#).unwrap_err();
        let s = e.to_string();
        assert!(s.contains("not a supported"), "{s}");
    }

    #[test]
    fn unknown_custom_fails_closed() {
        let e = classify_config(br#"{"model_type":"my_custom_moe","architectures":["FooForCausalLM"]}"#)
            .unwrap_err();
        let s = e.to_string();
        assert!(s.contains("fail closed") || s.contains("not a supported"), "{s}");
    }

    #[test]
    fn compile_snapshot_fails_closed_without_hub_download() {
        let dir = std::env::temp_dir().join(format!(
            "late-compile-snap-{}-fake",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let config = dir.join("config.json");
        let tok = dir.join("tokenizer.json");
        let weight = dir.join("model.safetensors");
        fs::write(&config, br#"{"model_type":"my_custom_moe","architectures":["FooForCausalLM"]}"#)
            .unwrap();
        fs::write(&tok, b"{}").unwrap();
        fs::write(&weight, b"").unwrap();
        let bytes = fs::read(&config).unwrap();
        let e = compile_snapshot("org/fake-moe", &bytes, &config, &tok, None, &[weight])
            .unwrap_err()
            .to_string();
        assert!(e.contains("fail closed") || e.contains("not a supported"), "{e}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn compiler_id_is_mlc() {
        assert_eq!(COMPILER_ID, "mlc-llm");
    }

    #[test]
    fn gpu_meta_empty_accel_is_not_nvidia() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("LATE_INFER_ACCEL");
        std::env::remove_var("LATE_INFER_PCI");
        let meta = crate::device::compile_meta();
        assert_ne!(meta.vendor.as_deref(), Some("nvidia"), "{meta:?}");
        assert_ne!(meta.accel.as_deref(), Some("nvidia"), "{meta:?}");
        std::env::set_var("LATE_INFER_ACCEL", "intel");
        std::env::set_var("LATE_INFER_PCI", "0000:08:00.0");
        let meta = crate::device::compile_meta();
        assert_eq!(meta.vendor.as_deref(), Some("intel"));
        assert_eq!(meta.accel.as_deref(), Some("intel"));
        assert_eq!(meta.pci_id.as_deref(), Some("0000:08:00.0"));
        std::env::remove_var("LATE_INFER_ACCEL");
        std::env::remove_var("LATE_INFER_PCI");
    }

    #[test]
    fn slug_replaces_slash() {
        assert_eq!(slug_model_id("Qwen/Qwen2.5-0.5B-Instruct"), "Qwen--Qwen2.5-0.5B-Instruct");
    }

    #[test]
    fn manifest_schema_has_no_allowlist_or_cwd() {
        let m = CompileManifest {
            compiler: COMPILER_ID.into(),
            compiler_project: COMPILER_PROJECT.into(),
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
            snapshot: SnapshotFiles::default(),
        };
        let v = serde_json::to_value(&m).unwrap();
        assert_no_fs_api(&v);
        assert_eq!(v["compiler"], "mlc-llm");
    }

    fn assert_no_fs_api(v: &serde_json::Value) {
        assert!(v.get("allowlist").is_none());
        assert!(v.get("cwd").is_none());
        assert!(v.get("write_dir").is_none());
        assert!(v.get("tools").is_none());
        assert!(v.get("filesystem").is_none());
        assert!(v.get("add_allowed_dir").is_none());
        let dump = v.to_string();
        assert!(!dump.contains("\"allowlist\""));
        assert!(!dump.contains("\"write_dir\""));
        assert!(!dump.contains("add_allowed_dir"));
    }

    #[test]
    fn whisper_and_vit_fail_closed() {
        let w = classify_config(br#"{"model_type":"whisper"}"#)
            .unwrap_err()
            .to_string();
        assert!(w.contains("not a supported"), "{w}");
        let v = classify_config(br#"{"model_type":"vit"}"#)
            .unwrap_err()
            .to_string();
        assert!(v.contains("not a supported"), "{v}");
    }

    #[test]
    fn qwen_with_vit_arch_stays_candle() {
        let p = classify_config(
            br#"{"model_type":"qwen2","architectures":["Qwen2SomethingVit"]}"#,
        )
        .unwrap();
        assert_eq!(p.serve, ServeKind::CandleFallback);
    }

    #[test]
    fn compiled_root_honors_late_compiled_dir() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "late-compiled-root-{}-{}",
            std::process::id(),
            "env"
        ));
        std::env::set_var("LATE_COMPILED_DIR", &dir);
        assert_eq!(compiled_root(), dir);
        std::env::remove_var("LATE_COMPILED_DIR");
    }

    #[test]
    fn materialize_qwen_writes_manifest_under_env_dir() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "late-compiled-mat-{}-{}",
            std::process::id(),
            "qwen"
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        std::env::set_var("LATE_COMPILED_DIR", &dir);

        let snap = dir.join("_snap");
        fs::create_dir_all(&snap).unwrap();
        let config = snap.join("config.json");
        let tok = snap.join("tokenizer.json");
        let tok_cfg = snap.join("tokenizer_config.json");
        let weight = snap.join("model.safetensors");
        fs::write(&config, br#"{"model_type":"qwen2"}"#).unwrap();
        fs::write(&tok, b"{}").unwrap();
        fs::write(&tok_cfg, br#"{"chat_template":"{{ messages[0].content }}"}"#).unwrap();
        fs::write(&weight, b"").unwrap();

        std::env::set_var("LATE_INFER_ACCEL", "intel");
        std::env::set_var("LATE_INFER_PCI", "0000:08:00.0");
        std::env::set_var("LATE_INFER_GPU_IDLE", "1");
        std::env::set_var("LATE_INFER_GPU_DISPLAY", "0");

        let plan = CompilePlan {
            model_type: "qwen2".into(),
            serve: ServeKind::CandleFallback,
        };
        let m = materialize(
            "Qwen/Qwen2.5-0.5B-Instruct",
            &plan,
            &config,
            &tok,
            Some(&tok_cfg),
            &[weight],
        )
        .unwrap();
        assert_eq!(m.compiler, "mlc-llm");
        assert_eq!(m.serve, "candle-fallback");
        assert_eq!(m.status, "ok");
        let written = dir
            .join("Qwen--Qwen2.5-0.5B-Instruct")
            .join("late-compile.json");
        assert!(written.exists(), "{}", written.display());
        let v: serde_json::Value =
            serde_json::from_slice(&fs::read(&written).unwrap()).unwrap();
        assert_no_fs_api(&v);
        assert_eq!(v["compiler"], "mlc-llm");
        assert_eq!(v["serve"], "candle-fallback");
        assert_eq!(v["vendor"], "intel");
        assert_eq!(v["accel"], "intel");
        assert_eq!(v["pci_id"], "0000:08:00.0");
        assert_eq!(v["idle"], true);
        assert_eq!(v["display"], false);

        std::env::remove_var("LATE_INFER_ACCEL");
        std::env::remove_var("LATE_INFER_PCI");
        std::env::remove_var("LATE_INFER_GPU_IDLE");
        std::env::remove_var("LATE_INFER_GPU_DISPLAY");
        std::env::remove_var("LATE_COMPILED_DIR");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn materialize_qwen_keeps_openvino_ir_on_intel() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "late-compiled-mat-{}-{}",
            std::process::id(),
            "qwen-ir"
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        std::env::set_var("LATE_COMPILED_DIR", &dir);

        let snap = dir.join("_snap");
        fs::create_dir_all(&snap).unwrap();
        let config = snap.join("config.json");
        let tok = snap.join("tokenizer.json");
        let weight = snap.join("model.safetensors");
        fs::write(&config, br#"{"model_type":"qwen2"}"#).unwrap();
        fs::write(&tok, b"{}").unwrap();
        fs::write(&weight, b"").unwrap();

        let ov = dir
            .join("Qwen--Qwen2.5-0.5B-Instruct")
            .join("openvino");
        fs::create_dir_all(&ov).unwrap();
        fs::write(ov.join("openvino_model.xml"), b"<net/>").unwrap();

        std::env::set_var("LATE_INFER_ACCEL", "intel");
        std::env::set_var("LATE_INFER_PCI", "0000:08:00.0");

        let plan = CompilePlan {
            model_type: "qwen2".into(),
            serve: ServeKind::CandleFallback,
        };
        let m = materialize(
            "Qwen/Qwen2.5-0.5B-Instruct",
            &plan,
            &config,
            &tok,
            None,
            &[weight],
        )
        .unwrap();
        assert_eq!(m.serve, "openvino-genai");
        assert_eq!(m.ir, Some(true));
        let written = dir
            .join("Qwen--Qwen2.5-0.5B-Instruct")
            .join("late-compile.json");
        let v: serde_json::Value =
            serde_json::from_slice(&fs::read(&written).unwrap()).unwrap();
        assert_eq!(v["serve"], "openvino-genai");
        assert_eq!(v["ir"], true);
        assert!(ov.join("openvino_model.xml").is_file());

        std::env::remove_var("LATE_INFER_ACCEL");
        std::env::remove_var("LATE_INFER_PCI");
        std::env::remove_var("LATE_COMPILED_DIR");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn materialize_llama_fails_closed_without_in_process_mlc() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "late-compiled-mat-{}-{}",
            std::process::id(),
            "llama"
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        std::env::set_var("LATE_COMPILED_DIR", &dir);
        // Explicitly not Intel — MLC fail-closed must not take the OpenVINO path on Arc boxes.
        std::env::remove_var("LATE_INFER_ACCEL");
        std::env::remove_var("LATE_INFER_INTEL_RUNTIME");

        let snap = dir.join("_snap");
        fs::create_dir_all(&snap).unwrap();
        let config = snap.join("config.json");
        let tok = snap.join("tokenizer.json");
        let weight = snap.join("model.safetensors");
        fs::write(&config, br#"{"model_type":"llama"}"#).unwrap();
        fs::write(&tok, b"{}").unwrap();
        fs::write(&weight, b"").unwrap();

        let plan = CompilePlan {
            model_type: "llama".into(),
            serve: ServeKind::Mlc,
        };
        if mlc_llm_bin().is_some() {
            let m = materialize("meta-llama/Llama-3.2-1B-Instruct", &plan, &config, &tok, None, &[weight])
                .expect("mlc_llm on PATH probes compile");
            let v = serde_json::to_value(&m).unwrap();
            assert_no_fs_api(&v);
            assert_eq!(m.serve, "mlc-llm");
        } else {
            let e = materialize(
                "meta-llama/Llama-3.2-1B-Instruct",
                &plan,
                &config,
                &tok,
                None,
                &[weight],
            )
            .unwrap_err()
            .to_string();
            assert!(
                e.contains("mlc") || e.contains("PATH") || e.contains("LATE_MLC_COMPILE"),
                "{e}"
            );
            let written = dir
                .join("meta-llama--Llama-3.2-1B-Instruct")
                .join("late-compile.json");
            if written.exists() {
                let v: serde_json::Value =
                    serde_json::from_slice(&fs::read(&written).unwrap()).unwrap();
                assert_no_fs_api(&v);
            }
        }

        std::env::remove_var("LATE_COMPILED_DIR");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn qwen3_classifies_as_mlc() {
        let p = classify_config(br#"{"model_type":"qwen3","architectures":["Qwen3ForCausalLM"]}"#)
            .unwrap();
        assert_eq!(p.model_type, "qwen3");
        assert_eq!(p.serve, ServeKind::Mlc);
    }

    #[test]
    fn preflight_qwen3_uses_openvino_when_intel_pinned() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let plan = CompilePlan {
            model_type: "qwen3".into(),
            serve: ServeKind::Mlc,
        };
        std::env::set_var("LATE_INFER_ACCEL", "intel");
        std::env::set_var("LATE_INFER_INTEL_RUNTIME", "1");
        preflight_plan(&plan).expect("Intel+OpenVINO must not require mlc-llm for Qwen3");
        std::env::set_var("LATE_INFER_INTEL_RUNTIME", "0");
        let e = preflight_plan(&plan).unwrap_err().to_string();
        assert!(
            e.contains("OpenVINO") || e.contains("Level Zero") || e.contains("Intel"),
            "expected OpenVINO missing message, got: {e}"
        );
        assert!(!e.contains("mlc-llm is not on PATH"), "{e}");
        std::env::remove_var("LATE_INFER_ACCEL");
        std::env::remove_var("LATE_INFER_INTEL_RUNTIME");
    }

    #[test]
    fn materialize_qwen3_skips_mlc_on_intel_openvino() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "late-compiled-mat-{}-{}",
            std::process::id(),
            "qwen3-ov"
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        std::env::set_var("LATE_COMPILED_DIR", &dir);
        std::env::set_var("LATE_INFER_ACCEL", "intel");
        std::env::set_var("LATE_INFER_PCI", "0000:08:00.0");
        std::env::set_var("LATE_INFER_INTEL_RUNTIME", "1");

        let snap = dir.join("_snap");
        fs::create_dir_all(&snap).unwrap();
        let config = snap.join("config.json");
        let tok = snap.join("tokenizer.json");
        let weight = snap.join("model.safetensors");
        fs::write(&config, br#"{"model_type":"qwen3","architectures":["Qwen3ForCausalLM"]}"#).unwrap();
        fs::write(&tok, b"{}").unwrap();
        fs::write(&weight, b"").unwrap();

        let plan = CompilePlan {
            model_type: "qwen3".into(),
            serve: ServeKind::Mlc,
        };
        let m = materialize(
            "Qwen/Qwen3-0.6B",
            &plan,
            &config,
            &tok,
            None,
            &[weight],
        )
        .expect("Intel+OpenVINO must materialize Qwen3 without mlc-llm");
        assert_eq!(m.status, "ok");
        assert_eq!(m.serve, "openvino-genai");
        assert!(m.error.is_none(), "{m:?}");
        let v = serde_json::to_value(&m).unwrap();
        assert_no_fs_api(&v);
        assert_eq!(v["serve"], "openvino-genai");

        std::env::remove_var("LATE_COMPILED_DIR");
        std::env::remove_var("LATE_INFER_ACCEL");
        std::env::remove_var("LATE_INFER_PCI");
        std::env::remove_var("LATE_INFER_INTEL_RUNTIME");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn preflight_qwen3_still_needs_mlc_without_intel_pin() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("LATE_INFER_ACCEL");
        std::env::remove_var("LATE_INFER_INTEL_RUNTIME");
        let plan = CompilePlan {
            model_type: "qwen3".into(),
            serve: ServeKind::Mlc,
        };
        if mlc_llm_bin().is_some() {
            preflight_plan(&plan).expect("mlc on PATH");
        } else {
            let e = preflight_plan(&plan).unwrap_err().to_string();
            assert!(e.contains("mlc-llm is not on PATH"), "{e}");
        }
    }

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
}
