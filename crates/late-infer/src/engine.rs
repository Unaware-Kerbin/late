use crate::compiler::{self, CompileManifest, ServeKind};
use crate::gemma4::{self, TextModel as Gemma4Text};
use crate::template;
use anyhow::{Context, Result};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::generation::LogitsProcessor;
use candle_transformers::models::qwen2::{Config, ModelForCausalLM};
use hf_hub::api::sync::ApiBuilder;
use hf_hub::api::Progress;
use hf_hub::{Cache, Repo, RepoType};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokenizers::Tokenizer;

/// Default Hub Instruct id (~1 GB safetensors). 0.5B is weak at tools; the
/// protocol still returns OpenAI `tool_calls` when the model emits Hermes JSON.
/// For better tool follow-through, Start with `Qwen/Qwen2.5-1.5B-Instruct`.
pub const DEFAULT_MODEL: &str = "Qwen/Qwen2.5-0.5B-Instruct";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Qwen2,
    Gemma4,
}

enum Inner {
    Qwen2(ModelForCausalLM),
    Gemma4(Gemma4Text),
    IntelOv(crate::intel::OvSession),
}

pub struct Engine {
    inner: Inner,
    tokenizer: Tokenizer,
    device: Device,
    pub model_id: String,
    pub device_label: String,
    pub family: Family,
    pub chat_template: Option<String>,
    pub compile_manifest: CompileManifest,
    eos: Vec<u32>,
}

#[derive(Debug, Deserialize)]
struct HubConfig {
    vocab_size: usize,
    hidden_size: usize,
    intermediate_size: usize,
    num_hidden_layers: usize,
    num_attention_heads: usize,
    num_key_value_heads: usize,
    max_position_embeddings: usize,
    #[serde(default)]
    sliding_window: Option<serde_json::Value>,
    #[serde(default)]
    max_window_layers: Option<usize>,
    #[serde(default)]
    tie_word_embeddings: bool,
    rope_theta: f64,
    rms_norm_eps: f64,
    #[serde(default)]
    use_sliding_window: bool,
    hidden_act: candle_nn::Activation,
}

impl HubConfig {
    fn into_candle(self) -> Config {
        let sliding_window = match self.sliding_window {
            Some(serde_json::Value::Number(n)) => n.as_u64().unwrap_or(32768) as usize,
            _ => self.max_position_embeddings,
        };
        Config {
            vocab_size: self.vocab_size,
            hidden_size: self.hidden_size,
            intermediate_size: self.intermediate_size,
            num_hidden_layers: self.num_hidden_layers,
            num_attention_heads: self.num_attention_heads,
            num_key_value_heads: self.num_key_value_heads,
            max_position_embeddings: self.max_position_embeddings,
            sliding_window,
            max_window_layers: self.max_window_layers.unwrap_or(self.num_hidden_layers),
            tie_word_embeddings: self.tie_word_embeddings,
            rope_theta: self.rope_theta,
            rms_norm_eps: self.rms_norm_eps,
            use_sliding_window: self.use_sliding_window,
            hidden_act: self.hidden_act,
        }
    }
}

pub fn detect_family(bytes: &[u8]) -> Result<Family> {
    let v: serde_json::Value = serde_json::from_slice(bytes).context("parse config.json")?;
    let model_type = v
        .get("model_type")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let arches: Vec<&str> = v
        .get("architectures")
        .and_then(|a| a.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
        .unwrap_or_default();
    let blob = format!("{model_type} {}", arches.join(" ")).to_ascii_lowercase();
    if blob.contains("gemma4") || blob.contains("gemma-4") {
        return Ok(Family::Gemma4);
    }
    if blob.contains("qwen") {
        return Ok(Family::Qwen2);
    }
    anyhow::bail!(
        "late-infer Candle serve does not load model_type={model_type:?} architectures={arches:?}. Qwen2 Instruct and Gemma 4 Instruct safetensors only (not GGUF, not Gemma 3). Other Instruct types need {} ({}) plus LATE_MLC_COMPILE=1.",
        compiler::COMPILER_ID,
        compiler::COMPILER_PROJECT,
    )
}

fn load_qwen_config(bytes: &[u8], path: &Path) -> Result<Config> {
    if let Ok(cfg) = serde_json::from_slice::<Config>(bytes) {
        return Ok(cfg);
    }
    let hub: HubConfig = serde_json::from_slice(bytes)
        .with_context(|| format!("parse Qwen2 config {}", path.display()))?;
    Ok(hub.into_candle())
}

/// Parseable `--compile-only` stderr. Orchestrator maps this to percent / ETA.
/// No snapshot paths, cwd, or allowlist.
pub fn format_hub_progress_line(phase: &str, bytes: u64, total: u64, eta_sec: Option<u64>) -> String {
    let percent = if total > 0 {
        ((bytes as f64 / total as f64) * 100.0).clamp(0.0, 100.0).round() as u64
    } else {
        0
    };
    match eta_sec {
        Some(eta) => format!(
            "late-infer: progress phase={phase} bytes={bytes} total={total} percent={percent} eta_sec={eta}"
        ),
        None => format!(
            "late-infer: progress phase={phase} bytes={bytes} total={total} percent={percent}"
        ),
    }
}

pub fn format_compile_progress_line() -> String {
    "late-infer: progress phase=compiling".to_string()
}

struct HubFetchState {
    started: Instant,
    last_emit: Instant,
    bytes: u64,
    total: u64,
}

impl HubFetchState {
    fn eta_sec(&self) -> Option<u64> {
        if self.total > 0 && self.bytes >= self.total {
            return Some(0);
        }
        if self.bytes == 0 || self.total <= self.bytes {
            return None;
        }
        let elapsed = self.started.elapsed().as_secs_f64();
        if elapsed < 0.4 {
            return None;
        }
        let rate = self.bytes as f64 / elapsed;
        if rate <= 0.0 {
            return None;
        }
        Some(((self.total - self.bytes) as f64 / rate).round() as u64)
    }

    fn emit(&mut self, force: bool) {
        if !force && self.last_emit.elapsed() < Duration::from_millis(250) {
            return;
        }
        self.last_emit = Instant::now();
        eprintln!(
            "{}",
            format_hub_progress_line("downloading", self.bytes, self.total, self.eta_sec())
        );
    }
}

#[derive(Clone)]
struct HubFetchProgress {
    inner: Arc<Mutex<HubFetchState>>,
}

impl HubFetchProgress {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            inner: Arc::new(Mutex::new(HubFetchState {
                started: now,
                last_emit: now.checked_sub(Duration::from_secs(1)).unwrap_or(now),
                bytes: 0,
                total: 0,
            })),
        }
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, HubFetchState> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn mark_complete(&self, size: u64) {
        let mut state = self.lock_state();
        state.total = state.total.saturating_add(size);
        state.bytes = state.bytes.saturating_add(size);
        state.emit(true);
    }
}

impl Progress for HubFetchProgress {
    fn init(&mut self, size: usize, _filename: &str) {
        let mut state = self.lock_state();
        state.total = state.total.saturating_add(size as u64);
        state.emit(true);
    }

    fn update(&mut self, size: usize) {
        let mut state = self.lock_state();
        state.bytes = state.bytes.saturating_add(size as u64);
        state.emit(false);
    }

    fn finish(&mut self) {
        self.lock_state().emit(true);
    }
}

fn hub_get(
    cache: &Cache,
    repo: &Repo,
    api_repo: &hf_hub::api::sync::ApiRepo,
    filename: &str,
    progress: &HubFetchProgress,
) -> Result<PathBuf> {
    if let Some(path) = cache.repo(repo.clone()).get(filename) {
        let len = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        progress.mark_complete(len);
        return Ok(path);
    }
    api_repo
        .download_with_progress(filename, progress.clone())
        .map_err(Into::into)
}

fn weight_files(
    cache: &Cache,
    repo: &Repo,
    api_repo: &hf_hub::api::sync::ApiRepo,
    progress: &HubFetchProgress,
) -> Result<Vec<PathBuf>> {
    if let Ok(one) = hub_get(cache, repo, api_repo, "model.safetensors", progress) {
        return Ok(vec![one]);
    }
    let index_path = hub_get(
        cache,
        repo,
        api_repo,
        "model.safetensors.index.json",
        progress,
    )
    .context("need model.safetensors or model.safetensors.index.json")?;
    let v: serde_json::Value = serde_json::from_slice(&fs::read(&index_path)?)?;
    let map = v
        .get("weight_map")
        .and_then(|m| m.as_object())
        .context("weight_map missing")?;
    let mut names = BTreeSet::new();
    for file in map.values() {
        if let Some(s) = file.as_str() {
            names.insert(s.to_string());
        }
    }
    names
        .into_iter()
        .map(|n| hub_get(cache, repo, api_repo, &n, progress))
        .collect()
}

fn status_line(msg: &str) {
    tracing::info!("{msg}");
    eprintln!("late-infer: {msg}");
}

pub struct HubSnapshot {
    pub plan: compiler::CompilePlan,
    pub config_path: PathBuf,
    pub config_bytes: Vec<u8>,
    pub tokenizer_path: PathBuf,
    pub tokenizer_config_path: Option<PathBuf>,
    pub weights: Vec<PathBuf>,
}

/// Fetch config, fail-closed classify, then tokenizer + safetensors into the Late Hub cache.
pub fn fetch_hub_snapshot(model_id: &str, revision: &str) -> Result<HubSnapshot> {
    status_line("downloading Hub snapshot…");
    let cache_dir = hub_cache_dir()?;
    tracing::info!("Hugging Face cache: {}", cache_dir.display());
    let disk = Cache::new(cache_dir.clone());
    let api = ApiBuilder::new()
        .with_cache_dir(cache_dir)
        .with_progress(false)
        .with_token(std::env::var("HF_TOKEN").ok())
        .build()
        .context("Hugging Face Hub client")?;
    let repo = Repo::with_revision(
        model_id.to_string(),
        RepoType::Model,
        revision.to_string(),
    );
    let api_repo = api.repo(repo.clone());
    let progress = HubFetchProgress::new();
    eprintln!("{}", format_hub_progress_line("downloading", 0, 0, None));
    let config_path = hub_get(&disk, &repo, &api_repo, "config.json", &progress).context("config.json")?;
    let config_bytes = fs::read(&config_path)
        .with_context(|| format!("read {}", config_path.display()))?;
    let plan = compiler::classify_config(&config_bytes)?;
    compiler::preflight_plan(&plan)?;
    tracing::info!(
        "compile plan: compiler={} model_type={} serve={:?}",
        compiler::COMPILER_ID,
        plan.model_type,
        plan.serve
    );
    let tokenizer_path =
        hub_get(&disk, &repo, &api_repo, "tokenizer.json", &progress).context("tokenizer.json")?;
    let tokenizer_config_path =
        hub_get(&disk, &repo, &api_repo, "tokenizer_config.json", &progress).ok();
    let weights = weight_files(&disk, &repo, &api_repo, &progress)?;
    tracing::info!("weights: {} file(s)", weights.len());
    Ok(HubSnapshot {
        plan,
        config_path,
        config_bytes,
        tokenizer_path,
        tokenizer_config_path,
        weights,
    })
}

/// Download + classify + materialize, then exit. Does not bind :8010 and does not load Candle.
/// Intel + OpenVINO: also export IR under `compiled/<slug>/openvino/` (RAM-capped).
pub fn compile_hub(model_id: &str, revision: &str) -> Result<CompileManifest> {
    let plan = crate::device::resolve_from_env().map_err(|e| anyhow::anyhow!("{e}"))?;
    crate::device::compile_gate(&plan).map_err(|e| anyhow::anyhow!("{e}"))?;
    let snap = fetch_hub_snapshot(model_id, revision)?;
    status_line("compiling on your computer…");
    eprintln!("{}", format_compile_progress_line());
    let mut manifest = compiler::compile_snapshot(
        model_id,
        &snap.config_bytes,
        &snap.config_path,
        &snap.tokenizer_path,
        snap.tokenizer_config_path.as_deref(),
        &snap.weights,
    )?;
    if plan.accel == Some(crate::device::AccelKind::Intel) {
        // Fail closed: Intel compile must produce OpenVINO IR (or reuse it). Soft-ok
        // without IR left Start stuck on "IR is missing" after a green Download.
        if !crate::intel::intel_runtime_ok() {
            anyhow::bail!("{}", crate::intel::runtime_missing_msg());
        }
        let model_dir = snap
            .config_path
            .parent()
            .map(Path::to_path_buf)
            .context("Hub snapshot has no directory")?;
        let ov_dir = compiler::blob_dir(model_id).join("openvino");
        let have_ir = ov_dir.join("openvino_model.xml").is_file();
        if have_ir {
            manifest.serve = "openvino-genai".into();
            manifest.ir = Some(true);
        } else {
            let weights = crate::host_ram::weight_bytes(&snap.weights);
            crate::host_ram::refuse_host_weight_store(
                weights,
                false,
                true,
                crate::host_ram::read_meminfo(),
            )?;
            status_line("exporting OpenVINO IR on your computer…");
            eprintln!("{}", format_compile_progress_line());
            crate::intel::OvSession::export_ir(model_id, &model_dir, &ov_dir)?;
            if !ov_dir.join("openvino_model.xml").is_file() {
                anyhow::bail!(
                    "OpenVINO IR is missing after convert. late-infer will not load Candle CPU as that Intel GPU."
                );
            }
            manifest.serve = "openvino-genai".into();
            manifest.ir = Some(true);
        }
        compiler::write_manifest(&compiler::blob_dir(model_id), &manifest)?;
    }
    Ok(manifest)
}

impl Engine {
    /// Intel XPU: OpenVINO GenAI on the idle GPU from detect. Never Candle CPU
    /// labeled as that card. Hub safetensors (not a CUDA compiled blob).
    pub fn load_intel(model_id: &str, revision: &str, device_label: String) -> Result<Self> {
        if !crate::intel::intel_runtime_ok() {
            anyhow::bail!("{}", crate::intel::runtime_missing_msg());
        }
        tracing::info!("loading {model_id}@{revision} on {device_label} (OpenVINO GenAI)");
        let snap = fetch_hub_snapshot(model_id, revision)?;
        let config_bytes = snap.config_bytes;
        let tokenizer_path = snap.tokenizer_path;
        let tokenizer_config_path = snap.tokenizer_config_path;
        let weights = snap.weights;
        let config_path = snap.config_path;

        let chat_template = tokenizer_config_path.as_ref().and_then(|p| {
            fs::read(p)
                .ok()
                .and_then(|b| template::load_chat_template(&b))
        });

        status_line("compiling on your computer…");
        eprintln!("{}", format_compile_progress_line());
        let mut compile_manifest = compiler::compile_snapshot(
            model_id,
            &config_bytes,
            &config_path,
            &tokenizer_path,
            tokenizer_config_path.as_deref(),
            &weights,
        )?;
        compile_manifest.serve = "openvino-genai".into();

        let family = detect_family(&config_bytes)?;
        let model_dir = config_path
            .parent()
            .map(Path::to_path_buf)
            .context("Hub snapshot has no directory")?;
        let ov_dir = compiler::blob_dir(model_id).join("openvino");
        let have_ir = ov_dir.join("openvino_model.xml").is_file();
        if !have_ir {
            anyhow::bail!(
                "OpenVINO IR is missing. Start refused to protect your computer. Download/compile must produce IR, or Convert only when MemAvailable is safe. {}",
                crate::host_ram::HOST_RAM_VS_VRAM
            );
        }
        crate::host_ram::refuse_host_weight_store(
            crate::host_ram::weight_bytes(&weights),
            true,
            true,
            crate::host_ram::read_meminfo(),
        )?;
        status_line("OpenVINO GenAI on the Intel GPU on your computer…");
        let session = crate::intel::OvSession::start_and_load(model_id, &model_dir, &ov_dir)?;
        compile_manifest.serve = "openvino-genai".into();
        compile_manifest.ir = Some(true);
        compiler::write_manifest(&compiler::blob_dir(model_id), &compile_manifest)?;
        let device_label = session.device_label.clone();
        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| anyhow::anyhow!("tokenizer: {e}"))?;
        let eos = eos_ids(&tokenizer, family);
        Ok(Self {
            inner: Inner::IntelOv(session),
            tokenizer,
            device: Device::Cpu,
            model_id: model_id.to_string(),
            device_label,
            family,
            chat_template,
            compile_manifest,
            eos,
        })
    }

    pub fn load(model_id: &str, revision: &str, device: Device, device_label: String) -> Result<Self> {
        tracing::info!("loading {model_id}@{revision} on {device_label}");
        let snap = fetch_hub_snapshot(model_id, revision)?;
        let plan = snap.plan.clone();
        let config_path = snap.config_path;
        let config_bytes = snap.config_bytes;
        let tokenizer_path = snap.tokenizer_path;
        let tokenizer_config_path = snap.tokenizer_config_path;
        let weights = snap.weights;

        let chat_template = tokenizer_config_path.as_ref().and_then(|p| {
            fs::read(p)
                .ok()
                .and_then(|b| template::load_chat_template(&b))
        });

        status_line("compiling on your computer…");
        eprintln!("{}", format_compile_progress_line());
        let compile_manifest = compiler::compile_snapshot(
            model_id,
            &config_bytes,
            &config_path,
            &tokenizer_path,
            tokenizer_config_path.as_deref(),
            &weights,
        )?;

        if plan.serve != ServeKind::CandleFallback || compile_manifest.serve != "candle-fallback" {
            anyhow::bail!(
                "this Hub snapshot (model_type={}) needs {} to serve. In-process MLC runtime is not in late-infer yet. Install mlc_llm from {} and set LATE_MLC_COMPILE=1 to compile. Qwen2 and Gemma 4 Instruct still serve via Candle on this computer.",
                plan.model_type,
                compiler::COMPILER_ID,
                compiler::COMPILER_PROJECT,
            );
        }

        let family = detect_family(&config_bytes)?;
        tracing::info!("architecture: {family:?} serve={}", compile_manifest.serve);

        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| anyhow::anyhow!("tokenizer: {e}"))?;
        let on_gpu = device.is_cuda() || device.is_metal();
        crate::host_ram::refuse_host_weight_store(
            crate::host_ram::weight_bytes(&weights),
            on_gpu,
            on_gpu,
            crate::host_ram::read_meminfo(),
        )?;
        let dtype = if on_gpu {
            DType::BF16
        } else {
            DType::F32
        };
        let vb = unsafe { VarBuilder::from_mmaped_safetensors(&weights, dtype, &device)? };
        let inner = match family {
            Family::Qwen2 => {
                let config = load_qwen_config(&config_bytes, &config_path)?;
                Inner::Qwen2(ModelForCausalLM::new(&config, vb)?)
            }
            Family::Gemma4 => {
                let cfg = gemma4::parse_text_config(&config_bytes)?;
                let vb = vb.pp("model").pp("language_model");
                let model = Gemma4Text::new(&cfg, vb).map_err(|e| {
                    anyhow::anyhow!(
                        "Gemma 4 text load failed ({e}). Candle 0.9 has no gemma4; 0.11's module still misses E2B PLE / KV-share (huggingface/candle#3448). late-infer vendors that text path."
                    )
                })?;
                Inner::Gemma4(model)
            }
        };
        let eos = eos_ids(&tokenizer, family);
        Ok(Self {
            inner,
            tokenizer,
            device,
            model_id: model_id.to_string(),
            device_label,
            family,
            chat_template,
            compile_manifest,
            eos,
        })
    }

    /// True when weights live in DRAM (Candle CPU). OpenVINO GenAI / CUDA / Metal are GPU VRAM.
    pub fn weights_in_host_ram(&self) -> bool {
        match self.inner {
            Inner::IntelOv(_) => false,
            _ => !self.device.is_cuda() && !self.device.is_metal(),
        }
    }

    pub fn generate(
        &mut self,
        prompt: &str,
        max_tokens: usize,
        temperature: Option<f64>,
        top_p: Option<f64>,
    ) -> Result<GenOut> {
        if let Inner::IntelOv(session) = &mut self.inner {
            let (text, finish) = session.generate(prompt, max_tokens, temperature, top_p)?;
            let finish_reason: &'static str = match finish.as_str() {
                "length" => "length",
                _ => "stop",
            };
            return Ok(GenOut {
                text,
                prompt_tokens: 0,
                completion_tokens: 0,
                finish_reason,
            });
        }
        match &mut self.inner {
            Inner::Qwen2(m) => m.clear_kv_cache(),
            Inner::Gemma4(m) => m.clear_kv_cache(),
            Inner::IntelOv(_) => unreachable!(),
        }
        let encoding = self
            .tokenizer
            .encode(prompt, true)
            .map_err(|e| anyhow::anyhow!("encode: {e}"))?;
        let mut tokens = encoding.get_ids().to_vec();
        let prompt_tokens = tokens.len();
        if tokens.is_empty() {
            anyhow::bail!("empty prompt");
        }

        let greedy = temperature.map(|t| t <= 0.0).unwrap_or(false);
        let temp = if greedy { None } else { temperature };
        let mut logits_processor = LogitsProcessor::new(299792458, temp, top_p);

        let mut finish = "length";
        for index in 0..max_tokens {
            let context_size = if index > 0 { 1 } else { tokens.len() };
            let start_pos = tokens.len().saturating_sub(context_size);
            let ctxt = &tokens[start_pos..];
            let input = Tensor::new(ctxt, &self.device)?.unsqueeze(0)?;
            let logits = match &mut self.inner {
                Inner::Qwen2(m) => m.forward(&input, start_pos)?,
                Inner::Gemma4(m) => m.forward(&input, start_pos)?,
                Inner::IntelOv(_) => unreachable!("OpenVINO generate does not use Candle"),
            };
            let logits = logits.squeeze(0)?.squeeze(0)?.to_dtype(DType::F32)?;
            let next = logits_processor.sample(&logits)?;
            tokens.push(next);
            if self.eos.contains(&next) {
                finish = "stop";
                break;
            }
        }

        let completion = &tokens[prompt_tokens..];
        let stripped: Vec<u32> = completion
            .iter()
            .copied()
            .filter(|t| !self.eos.contains(t))
            .collect();
        let text = self
            .tokenizer
            .decode(&stripped, true)
            .map_err(|e| anyhow::anyhow!("decode: {e}"))?;
        Ok(GenOut {
            text,
            prompt_tokens: prompt_tokens as u32,
            completion_tokens: completion.len() as u32,
            finish_reason: finish,
        })
    }
}

pub struct GenOut {
    pub text: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub finish_reason: &'static str,
}

fn eos_ids(tok: &Tokenizer, family: Family) -> Vec<u32> {
    let names: &[&str] = match family {
        Family::Qwen2 => &["<|im_end|>", "<|endoftext|>"],
        Family::Gemma4 => &["<eos>", "<turn|>", "<end_of_turn>"],
    };
    names.iter().filter_map(|t| tok.token_to_id(t)).collect()
}

#[cfg(test)]
mod tests {
    use super::{detect_family, format_compile_progress_line, format_hub_progress_line, Family};
    use crate::compiler::{classify_config, ServeKind};

    #[test]
    fn compile_only_progress_lines_are_parseable() {
        let line = format_hub_progress_line("downloading", 2_500, 10_000, Some(12));
        assert_eq!(
            line,
            "late-infer: progress phase=downloading bytes=2500 total=10000 percent=25 eta_sec=12"
        );
        assert!(!line.to_lowercase().contains("cwd"));
        assert!(!line.to_lowercase().contains("allowlist"));
        assert!(!line.contains('/'));
        let compile = format_compile_progress_line();
        assert_eq!(compile, "late-infer: progress phase=compiling");
        let done = format_hub_progress_line("downloading", 10_000, 10_000, Some(0));
        assert!(done.contains("percent=100"));
        assert!(done.contains("eta_sec=0"));
        let unknown = format_hub_progress_line("downloading", 0, 0, None);
        assert_eq!(
            unknown,
            "late-infer: progress phase=downloading bytes=0 total=0 percent=0"
        );
    }

    #[test]
    fn detects_qwen2_and_gemma4() {
        let qwen = br#"{"architectures":["Qwen2ForCausalLM"],"model_type":"qwen2","hidden_act":"silu"}"#;
        assert_eq!(detect_family(qwen).unwrap(), Family::Qwen2);
        let gemma = br#"{"architectures":["Gemma4ForConditionalGeneration"],"model_type":"gemma4","text_config":{}}"#;
        assert_eq!(detect_family(gemma).unwrap(), Family::Gemma4);
    }

    #[test]
    fn rejects_gemma3() {
        let g3 = br#"{"architectures":["Gemma3ForConditionalGeneration"],"model_type":"gemma3"}"#;
        let err = detect_family(g3).unwrap_err().to_string();
        assert!(err.contains("Gemma 3") || err.contains("gemma3"));
        let plan = classify_config(g3).unwrap();
        assert_eq!(plan.serve, ServeKind::Mlc);
    }

    #[test]
    fn llama_is_mlc_not_candle() {
        let raw = br#"{"model_type":"llama","architectures":["LlamaForCausalLM"]}"#;
        assert!(detect_family(raw).is_err());
        let p = classify_config(raw).unwrap();
        assert_eq!(p.serve, ServeKind::Mlc);
    }
}

/// Prefer a writable Late dir. `~/.cache/huggingface` on this computer is root-owned.
fn hub_cache_dir() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("LATE_HF_HOME").or_else(|_| std::env::var("HF_HOME")) {
        let p = PathBuf::from(p);
        fs::create_dir_all(&p)
            .with_context(|| format!("Hugging Face cache {} is not writable", p.display()))?;
        return Ok(p);
    }
    let p = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("late")
        .join("hf");
    fs::create_dir_all(&p).with_context(|| format!("create {}", p.display()))?;
    Ok(p)
}
