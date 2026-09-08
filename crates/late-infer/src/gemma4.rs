//! Gemma 4 **text** decoder for Instruct safetensors (`google/gemma-4-E2B-it`).
//!
//! Candle 0.9 has no `gemma4` module. Candle 0.11 exports one, but it cannot load
//! E2B/E4B: wrong tensor prefix, no per-layer embeddings, no KV-shared /
//! double-wide MLP ([huggingface/candle#3448](https://github.com/huggingface/candle/issues/3448),
//! draft [#3608](https://github.com/huggingface/candle/pull/3608)). This file follows
//! that draft's text path (prefix `model.language_model`, PLE, KV share).

use candle_core::{DType, Device, Module, Result, Tensor, D};
use candle_nn::{linear_b as linear_bias, Activation, Linear, VarBuilder};
use candle_transformers::utils::repeat_kv;
use serde::Deserialize;
use std::sync::Arc;

fn default_attention_bias() -> bool {
    false
}
fn default_head_dim() -> usize {
    256
}
fn default_hidden_activation() -> Activation {
    Activation::GeluPytorchTanh
}
fn default_num_attention_heads() -> usize {
    8
}
fn default_num_key_value_heads() -> usize {
    1
}
fn default_rms_norm_eps() -> f64 {
    1e-6
}
fn default_rope_theta() -> f64 {
    1_000_000.
}
fn default_vocab_size() -> usize {
    262144
}
fn default_max_position_embeddings() -> usize {
    131072
}
fn default_tie_word_embeddings() -> bool {
    true
}
fn default_global_head_dim() -> usize {
    512
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Gemma4RopeLayerParams {
    rope_theta: Option<f64>,
    partial_rotary_factor: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Gemma4RopeParameters {
    full_attention: Option<Gemma4RopeLayerParams>,
    sliding_attention: Option<Gemma4RopeLayerParams>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Gemma4TextConfig {
    #[serde(default = "default_attention_bias")]
    pub attention_bias: bool,
    #[serde(default = "default_head_dim")]
    pub head_dim: usize,
    #[serde(default = "default_hidden_activation")]
    pub hidden_activation: Activation,
    pub hidden_size: usize,
    pub intermediate_size: usize,
    #[serde(default = "default_num_attention_heads")]
    pub num_attention_heads: usize,
    pub num_hidden_layers: usize,
    #[serde(default = "default_num_key_value_heads")]
    pub num_key_value_heads: usize,
    #[serde(default = "default_rms_norm_eps")]
    pub rms_norm_eps: f64,
    #[serde(default = "default_rope_theta")]
    pub rope_theta: f64,
    #[serde(default = "default_vocab_size")]
    pub vocab_size: usize,
    pub sliding_window: usize,
    pub final_logit_softcapping: Option<f64>,
    #[serde(default = "default_max_position_embeddings")]
    pub max_position_embeddings: usize,
    #[serde(default = "default_tie_word_embeddings")]
    pub tie_word_embeddings: bool,
    pub layer_types: Vec<String>,
    #[serde(default = "default_global_head_dim")]
    pub global_head_dim: usize,
    #[serde(default)]
    pub attention_k_eq_v: bool,
    pub num_global_key_value_heads: Option<usize>,
    rope_parameters: Option<Gemma4RopeParameters>,
    #[serde(default)]
    pub vocab_size_per_layer_input: usize,
    #[serde(default)]
    pub hidden_size_per_layer_input: usize,
    #[serde(default)]
    pub num_kv_shared_layers: usize,
    #[serde(default)]
    pub use_double_wide_mlp: bool,
}

impl Gemma4TextConfig {
    fn partial_rotary_factor(&self) -> f64 {
        self.rope_parameters
            .as_ref()
            .and_then(|rp| rp.full_attention.as_ref())
            .and_then(|fa| fa.partial_rotary_factor)
            .unwrap_or(0.25)
    }

    fn rope_local_base_freq(&self) -> f64 {
        self.rope_parameters
            .as_ref()
            .and_then(|rp| rp.sliding_attention.as_ref())
            .and_then(|sa| sa.rope_theta)
            .unwrap_or(10000.0)
    }

    fn is_sliding(&self, layer_idx: usize) -> bool {
        self.layer_types
            .get(layer_idx)
            .map(|s| s == "sliding_attention")
            .unwrap_or(false)
    }

    fn first_kv_shared_layer_idx(&self) -> usize {
        self.num_hidden_layers
            .saturating_sub(self.num_kv_shared_layers)
    }
}

#[derive(Debug, Clone, Deserialize)]
struct Gemma4HubFile {
    text_config: Gemma4TextConfig,
}

pub fn parse_text_config(bytes: &[u8]) -> anyhow::Result<Gemma4TextConfig> {
    let hub: Gemma4HubFile = serde_json::from_slice(bytes)
        .map_err(|e| anyhow::anyhow!("parse Gemma 4 config.json: {e}"))?;
    Ok(hub.text_config)
}

struct RmsNorm {
    weight: Tensor,
    eps: f64,
}

impl RmsNorm {
    fn new(dim: usize, eps: f64, vb: VarBuilder) -> Result<Self> {
        let weight = vb.get(dim, "weight")?;
        Ok(Self { weight, eps })
    }
}

impl Module for RmsNorm {
    fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let x_dtype = x.dtype();
        let internal_dtype = match x_dtype {
            DType::F16 | DType::BF16 => DType::F32,
            d => d,
        };
        let hidden_size = x.dim(D::Minus1)?;
        let x = x.to_dtype(internal_dtype)?;
        let norm_x = (x.sqr()?.sum_keepdim(D::Minus1)? / hidden_size as f64)?;
        let x_normed = x.broadcast_div(&(norm_x + self.eps)?.sqrt()?)?;
        x_normed.to_dtype(x_dtype)?.broadcast_mul(&self.weight)
    }
}

fn v_norm(v: &Tensor, eps: f64) -> Result<Tensor> {
    let original_dtype = v.dtype();
    let v_f32 = v.to_dtype(DType::F32)?;
    let mean_sq = v_f32.sqr()?.mean_keepdim(D::Minus1)?;
    let rms = (mean_sq + eps)?.sqrt()?;
    v_f32.broadcast_div(&rms)?.to_dtype(original_dtype)
}

struct RotaryEmbedding {
    sin: Tensor,
    cos: Tensor,
}

impl RotaryEmbedding {
    fn new(dtype: DType, head_dim: usize, rope_theta: f64, max_seq_len: usize, dev: &Device) -> Result<Self> {
        let inv_freq: Vec<_> = (0..head_dim)
            .step_by(2)
            .map(|i| 1f32 / rope_theta.powf(i as f64 / head_dim as f64) as f32)
            .collect();
        let inv_freq_len = inv_freq.len();
        let inv_freq = Tensor::from_vec(inv_freq, (1, inv_freq_len), dev)?.to_dtype(dtype)?;
        let t = Tensor::arange(0u32, max_seq_len as u32, dev)?
            .to_dtype(dtype)?
            .reshape((max_seq_len, 1))?;
        let freqs = t.matmul(&inv_freq)?;
        Ok(Self {
            sin: freqs.sin()?,
            cos: freqs.cos()?,
        })
    }

    fn cos_sin(&self, q: &Tensor, seqlen_offset: usize) -> Result<(Tensor, Tensor)> {
        let (_b_sz, _h, seq_len, _n_embd) = q.dims4()?;
        Ok((
            self.cos.narrow(0, seqlen_offset, seq_len)?,
            self.sin.narrow(0, seqlen_offset, seq_len)?,
        ))
    }
}

struct ProportionalRotaryEmbedding {
    sin: Tensor,
    cos: Tensor,
}

impl ProportionalRotaryEmbedding {
    fn new(
        dtype: DType,
        head_dim: usize,
        rope_theta: f64,
        partial_rotary_factor: f64,
        max_seq_len: usize,
        dev: &Device,
    ) -> Result<Self> {
        let rope_angles = (partial_rotary_factor * head_dim as f64 / 2.0) as usize;
        let half_dim = head_dim / 2;
        let mut inv_freq_vec = Vec::with_capacity(half_dim);
        for i in 0..rope_angles {
            inv_freq_vec.push(1f32 / (rope_theta as f32).powf((2 * i) as f32 / head_dim as f32));
        }
        inv_freq_vec.extend(std::iter::repeat(0f32).take(half_dim.saturating_sub(rope_angles)));
        let inv_freq = Tensor::from_vec(inv_freq_vec, (1, half_dim), dev)?;
        let t = Tensor::arange(0u32, max_seq_len as u32, dev)?
            .to_dtype(DType::F32)?
            .reshape((max_seq_len, 1))?;
        let freqs = t.matmul(&inv_freq)?;
        Ok(Self {
            cos: freqs.cos()?.to_dtype(dtype)?,
            sin: freqs.sin()?.to_dtype(dtype)?,
        })
    }

    fn cos_sin(&self, q: &Tensor, seqlen_offset: usize) -> Result<(Tensor, Tensor)> {
        let (_b_sz, _h, seq_len, _n_embd) = q.dims4()?;
        Ok((
            self.cos.narrow(0, seqlen_offset, seq_len)?,
            self.sin.narrow(0, seqlen_offset, seq_len)?,
        ))
    }
}

struct MLP {
    gate_proj: Linear,
    up_proj: Linear,
    down_proj: Linear,
    act_fn: Activation,
}

impl MLP {
    fn new(hidden_size: usize, intermediate_size: usize, act: Activation, bias: bool, vb: VarBuilder) -> Result<Self> {
        Ok(Self {
            gate_proj: linear_bias(hidden_size, intermediate_size, bias, vb.pp("gate_proj"))?,
            up_proj: linear_bias(hidden_size, intermediate_size, bias, vb.pp("up_proj"))?,
            down_proj: linear_bias(intermediate_size, hidden_size, bias, vb.pp("down_proj"))?,
            act_fn: act,
        })
    }
}

impl Module for MLP {
    fn forward(&self, xs: &Tensor) -> Result<Tensor> {
        let lhs = xs.apply(&self.gate_proj)?.apply(&self.act_fn)?;
        let rhs = xs.apply(&self.up_proj)?;
        (lhs * rhs)?.apply(&self.down_proj)
    }
}

enum KvCache {
    Normal(candle_nn::kv_cache::KvCache),
    Rotating(candle_nn::kv_cache::RotatingKvCache),
}

#[derive(Default)]
struct SharedKvStates {
    for_full: Option<(Tensor, Tensor)>,
    for_sliding: Option<(Tensor, Tensor)>,
}

enum KvSource {
    Computed {
        k_proj: Linear,
        v_proj: Option<Linear>,
        k_norm: RmsNorm,
        num_kv_heads: usize,
        rms_norm_eps: f64,
        kv_cache: KvCache,
        store_full_length_kv: bool,
    },
    Shared,
}

struct Attention {
    kv: KvSource,
    q_proj: Linear,
    o_proj: Linear,
    q_norm: RmsNorm,
    num_heads: usize,
    num_kv_groups: usize,
    head_dim: usize,
    is_sliding: bool,
    rotary_emb_global: Arc<ProportionalRotaryEmbedding>,
    rotary_emb_local: Arc<RotaryEmbedding>,
}

impl Attention {
    fn new(
        rotary_emb_global: Arc<ProportionalRotaryEmbedding>,
        rotary_emb_local: Arc<RotaryEmbedding>,
        cfg: &Gemma4TextConfig,
        layer_idx: usize,
        vb: VarBuilder,
    ) -> Result<Self> {
        let hidden_sz = cfg.hidden_size;
        let num_heads = cfg.num_attention_heads;
        let bias = cfg.attention_bias;
        let is_sliding = cfg.is_sliding(layer_idx);
        let head_dim = if is_sliding {
            cfg.head_dim
        } else {
            cfg.global_head_dim
        };
        let use_alternative_attention = cfg.attention_k_eq_v && !is_sliding;
        let num_kv_heads = if use_alternative_attention {
            cfg.num_global_key_value_heads.ok_or_else(|| {
                candle_core::Error::Msg(
                    "missing num_global_key_value_heads (attention_k_eq_v)".into(),
                )
            })?
        } else {
            cfg.num_key_value_heads
        };
        let num_kv_groups = num_heads / num_kv_heads.max(1);
        let q_proj = linear_bias(hidden_sz, num_heads * head_dim, bias, vb.pp("q_proj"))?;
        let o_proj = linear_bias(num_heads * head_dim, hidden_sz, bias, vb.pp("o_proj"))?;
        let q_norm = RmsNorm::new(head_dim, cfg.rms_norm_eps, vb.pp("q_norm"))?;
        let first_kv_shared = cfg.first_kv_shared_layer_idx();
        let is_kv_shared_layer = cfg.num_kv_shared_layers > 0 && layer_idx >= first_kv_shared;
        let kv = if is_kv_shared_layer {
            KvSource::Shared
        } else {
            let k_proj = linear_bias(hidden_sz, num_kv_heads * head_dim, bias, vb.pp("k_proj"))?;
            let v_proj = if use_alternative_attention {
                None
            } else {
                Some(linear_bias(
                    hidden_sz,
                    num_kv_heads * head_dim,
                    bias,
                    vb.pp("v_proj"),
                )?)
            };
            let k_norm = RmsNorm::new(head_dim, cfg.rms_norm_eps, vb.pp("k_norm"))?;
            let this_type = cfg.layer_types.get(layer_idx).map(String::as_str).unwrap_or("");
            let store_full_length_kv = cfg.layer_types[..first_kv_shared.min(cfg.layer_types.len())]
                .iter()
                .rposition(|t| t == this_type)
                == Some(layer_idx);
            let kv_cache = if is_sliding {
                KvCache::Rotating(candle_nn::kv_cache::RotatingKvCache::new(
                    2,
                    cfg.sliding_window,
                ))
            } else {
                KvCache::Normal(candle_nn::kv_cache::KvCache::new(
                    2,
                    cfg.max_position_embeddings,
                ))
            };
            KvSource::Computed {
                k_proj,
                v_proj,
                k_norm,
                num_kv_heads,
                rms_norm_eps: cfg.rms_norm_eps,
                kv_cache,
                store_full_length_kv,
            }
        };
        Ok(Self {
            kv,
            q_proj,
            o_proj,
            q_norm,
            num_heads,
            num_kv_groups,
            head_dim,
            is_sliding,
            rotary_emb_global,
            rotary_emb_local,
        })
    }

    fn forward(
        &mut self,
        xs: &Tensor,
        attention_mask: Option<&Tensor>,
        sliding_attention_mask: Option<&Tensor>,
        seqlen_offset: usize,
        shared_kv_states: &mut SharedKvStates,
    ) -> Result<Tensor> {
        let (b_sz, q_len, _) = xs.dims3()?;
        let mut q = self.q_proj.forward(xs)?;
        q = q
            .reshape((b_sz, q_len, self.num_heads, self.head_dim))?
            .transpose(1, 2)?;
        q = self.q_norm.forward(&q)?;
        let (cos, sin) = if self.is_sliding {
            self.rotary_emb_local.cos_sin(&q, seqlen_offset)?
        } else {
            self.rotary_emb_global.cos_sin(&q, seqlen_offset)?
        };
        let q = candle_nn::rotary_emb::rope(&q.contiguous()?, &cos, &sin)?;
        let (k, v) = match &mut self.kv {
            KvSource::Computed {
                k_proj,
                v_proj,
                k_norm,
                num_kv_heads,
                rms_norm_eps,
                kv_cache,
                store_full_length_kv,
            } => {
                let mut k = k_proj.forward(xs)?;
                let mut v = match v_proj {
                    Some(v_proj) => v_proj.forward(xs)?,
                    None => k.clone(),
                };
                k = k
                    .reshape((b_sz, q_len, *num_kv_heads, self.head_dim))?
                    .transpose(1, 2)?;
                v = v
                    .reshape((b_sz, q_len, *num_kv_heads, self.head_dim))?
                    .transpose(1, 2)?;
                k = k_norm.forward(&k)?;
                k = candle_nn::rotary_emb::rope(&k.contiguous()?, &cos, &sin)?;
                v = v_norm(&v, *rms_norm_eps)?;
                let (k, v) = match kv_cache {
                    KvCache::Normal(cache) => cache.append(&k, &v)?,
                    KvCache::Rotating(cache) => cache.append(&k, &v)?,
                };
                if *store_full_length_kv {
                    let kv = (k.clone(), v.clone());
                    if self.is_sliding {
                        shared_kv_states.for_sliding = Some(kv);
                    } else {
                        shared_kv_states.for_full = Some(kv);
                    }
                }
                (k, v)
            }
            KvSource::Shared => {
                if self.is_sliding {
                    shared_kv_states.for_sliding.clone().ok_or_else(|| {
                        candle_core::Error::Msg("missing shared sliding KV".into())
                    })?
                } else {
                    shared_kv_states
                        .for_full
                        .clone()
                        .ok_or_else(|| candle_core::Error::Msg("missing shared full KV".into()))?
                }
            }
        };
        let k = repeat_kv(k, self.num_kv_groups)?.contiguous()?;
        let v = repeat_kv(v, self.num_kv_groups)?.contiguous()?;
        let mask = if self.is_sliding {
            sliding_attention_mask
        } else {
            attention_mask
        };
        let scale = 1f64 / f64::sqrt(self.head_dim as f64);
        let attn_weights = (q.matmul(&k.transpose(2, 3)?)? * scale)?;
        let attn_weights = match mask {
            None => attn_weights,
            Some(mask) => attn_weights.broadcast_add(mask)?,
        };
        let attn_weights = candle_nn::ops::softmax_last_dim(&attn_weights)?;
        let attn_output = attn_weights.matmul(&v)?;
        attn_output
            .transpose(1, 2)?
            .reshape((b_sz, q_len, self.num_heads * self.head_dim))?
            .apply(&self.o_proj)
    }

    fn clear_kv_cache(&mut self) {
        if let KvSource::Computed { kv_cache, .. } = &mut self.kv {
            match kv_cache {
                KvCache::Normal(c) => c.reset(),
                KvCache::Rotating(c) => c.reset(),
            }
        }
    }
}

struct PerLayerInputMixer {
    per_layer_input_gate: Linear,
    act_fn: Activation,
    per_layer_projection: Linear,
    post_per_layer_input_norm: RmsNorm,
}

struct DecoderLayer {
    self_attn: Attention,
    mlp: MLP,
    input_layernorm: RmsNorm,
    post_attention_layernorm: RmsNorm,
    pre_feedforward_layernorm: RmsNorm,
    post_feedforward_layernorm: RmsNorm,
    layer_scalar: Tensor,
    pli_mixer: Option<PerLayerInputMixer>,
}

impl DecoderLayer {
    fn new(
        rotary_emb_global: Arc<ProportionalRotaryEmbedding>,
        rotary_emb_local: Arc<RotaryEmbedding>,
        cfg: &Gemma4TextConfig,
        layer_idx: usize,
        vb: VarBuilder,
    ) -> Result<Self> {
        let self_attn = Attention::new(
            rotary_emb_global,
            rotary_emb_local,
            cfg,
            layer_idx,
            vb.pp("self_attn"),
        )?;
        let first_kv_shared = cfg.first_kv_shared_layer_idx();
        let is_kv_shared = cfg.num_kv_shared_layers > 0 && layer_idx >= first_kv_shared;
        let effective_intermediate = if cfg.use_double_wide_mlp && is_kv_shared {
            cfg.intermediate_size * 2
        } else {
            cfg.intermediate_size
        };
        let mlp = MLP::new(
            cfg.hidden_size,
            effective_intermediate,
            cfg.hidden_activation,
            false,
            vb.pp("mlp"),
        )?;
        let pli_mixer = if cfg.hidden_size_per_layer_input > 0 {
            Some(PerLayerInputMixer {
                per_layer_input_gate: candle_nn::linear_no_bias(
                    cfg.hidden_size,
                    cfg.hidden_size_per_layer_input,
                    vb.pp("per_layer_input_gate"),
                )?,
                act_fn: cfg.hidden_activation,
                per_layer_projection: candle_nn::linear_no_bias(
                    cfg.hidden_size_per_layer_input,
                    cfg.hidden_size,
                    vb.pp("per_layer_projection"),
                )?,
                post_per_layer_input_norm: RmsNorm::new(
                    cfg.hidden_size,
                    cfg.rms_norm_eps,
                    vb.pp("post_per_layer_input_norm"),
                )?,
            })
        } else {
            None
        };
        Ok(Self {
            self_attn,
            mlp,
            input_layernorm: RmsNorm::new(cfg.hidden_size, cfg.rms_norm_eps, vb.pp("input_layernorm"))?,
            post_attention_layernorm: RmsNorm::new(
                cfg.hidden_size,
                cfg.rms_norm_eps,
                vb.pp("post_attention_layernorm"),
            )?,
            pre_feedforward_layernorm: RmsNorm::new(
                cfg.hidden_size,
                cfg.rms_norm_eps,
                vb.pp("pre_feedforward_layernorm"),
            )?,
            post_feedforward_layernorm: RmsNorm::new(
                cfg.hidden_size,
                cfg.rms_norm_eps,
                vb.pp("post_feedforward_layernorm"),
            )?,
            layer_scalar: vb.get(1, "layer_scalar")?,
            pli_mixer,
        })
    }

    fn forward(
        &mut self,
        xs: &Tensor,
        attention_mask: Option<&Tensor>,
        sliding_attention_mask: Option<&Tensor>,
        seqlen_offset: usize,
        per_layer_input: Option<&Tensor>,
        shared_kv_states: &mut SharedKvStates,
    ) -> Result<Tensor> {
        let residual = xs;
        let xs = self.input_layernorm.forward(xs)?;
        let xs = self.self_attn.forward(
            &xs,
            attention_mask,
            sliding_attention_mask,
            seqlen_offset,
            shared_kv_states,
        )?;
        let xs = xs.apply(&self.post_attention_layernorm)?;
        let xs = (xs + residual)?;
        let residual = &xs;
        let xs = xs.apply(&self.pre_feedforward_layernorm)?;
        let xs = xs.apply(&self.mlp)?;
        let xs = xs.apply(&self.post_feedforward_layernorm)?;
        let xs = (residual + xs)?;
        let xs = match (&self.pli_mixer, per_layer_input) {
            (Some(pli_mixer), Some(per_layer_input)) => {
                let residual = &xs;
                let gated = xs
                    .apply(&pli_mixer.per_layer_input_gate)?
                    .apply(&pli_mixer.act_fn)?;
                let pli = (gated * per_layer_input)?
                    .apply(&pli_mixer.per_layer_projection)?
                    .apply(&pli_mixer.post_per_layer_input_norm)?;
                // layer_scalar is ~0.02–0.25; it scales the PLE mix-in, not the residual stream.
                let pli = pli.broadcast_mul(&self.layer_scalar)?;
                (residual + pli)?
            }
            (None, None) => xs,
            _ => {
                return Err(candle_core::Error::Msg(
                    "Gemma 4 PLE mixer/input mismatch".into(),
                ))
            }
        };
        Ok(xs)
    }

    fn clear_kv_cache(&mut self) {
        self.self_attn.clear_kv_cache()
    }
}

fn prepare_decoder_attention_mask(
    b_size: usize,
    tgt_len: usize,
    seqlen_offset: usize,
    sliding_window: Option<usize>,
    dtype: DType,
    device: &Device,
) -> Result<Tensor> {
    let mask: Vec<_> = if let Some(sliding_window) = sliding_window {
        (0..tgt_len)
            .flat_map(|i| {
                (0..tgt_len).map(move |j| {
                    if i < j || j + sliding_window < i {
                        f32::NEG_INFINITY
                    } else {
                        0.
                    }
                })
            })
            .collect()
    } else {
        (0..tgt_len)
            .flat_map(|i| (0..tgt_len).map(move |j| if i < j { f32::NEG_INFINITY } else { 0f32 }))
            .collect()
    };
    let mask = Tensor::from_slice(&mask, (tgt_len, tgt_len), device)?;
    let mask = if seqlen_offset > 0 {
        let mask0 = Tensor::zeros((tgt_len, seqlen_offset), DType::F32, device)?;
        Tensor::cat(&[&mask0, &mask], D::Minus1)?
    } else {
        mask
    };
    mask.expand((b_size, 1, tgt_len, tgt_len + seqlen_offset))?
        .to_dtype(dtype)
}

struct PerLayerEmbeddings {
    hidden_size_per_layer_input: usize,
    embed_tokens_per_layer: candle_nn::Embedding,
    per_layer_model_projection: Linear,
    per_layer_projection_norm: RmsNorm,
}

pub struct TextModel {
    embed_tokens: candle_nn::Embedding,
    layers: Vec<DecoderLayer>,
    norm: RmsNorm,
    lm_head: Linear,
    final_logit_softcapping: Option<f64>,
    device: Device,
    dtype: DType,
    hidden_size: usize,
    sliding_window: usize,
    ple: Option<PerLayerEmbeddings>,
}

impl TextModel {
    pub fn new(cfg: &Gemma4TextConfig, vb: VarBuilder) -> Result<Self> {
        let embed_tokens =
            candle_nn::embedding(cfg.vocab_size, cfg.hidden_size, vb.pp("embed_tokens"))?;
        let rotary_emb_global = Arc::new(ProportionalRotaryEmbedding::new(
            vb.dtype(),
            cfg.global_head_dim,
            cfg.rope_theta,
            cfg.partial_rotary_factor(),
            cfg.max_position_embeddings,
            vb.device(),
        )?);
        let rotary_emb_local = Arc::new(RotaryEmbedding::new(
            vb.dtype(),
            cfg.head_dim,
            cfg.rope_local_base_freq(),
            cfg.max_position_embeddings,
            vb.device(),
        )?);
        let mut layers = Vec::with_capacity(cfg.num_hidden_layers);
        let vb_l = vb.pp("layers");
        for layer_idx in 0..cfg.num_hidden_layers {
            layers.push(DecoderLayer::new(
                rotary_emb_global.clone(),
                rotary_emb_local.clone(),
                cfg,
                layer_idx,
                vb_l.pp(layer_idx),
            )?);
        }
        let norm = RmsNorm::new(cfg.hidden_size, cfg.rms_norm_eps, vb.pp("norm"))?;
        let lm_head = if cfg.tie_word_embeddings {
            Linear::new(embed_tokens.embeddings().clone(), None)
        } else {
            candle_nn::linear_no_bias(cfg.hidden_size, cfg.vocab_size, vb.pp("lm_head"))?
        };
        let ple = if cfg.hidden_size_per_layer_input > 0 {
            let vocab = if cfg.vocab_size_per_layer_input == 0 {
                cfg.vocab_size
            } else {
                cfg.vocab_size_per_layer_input
            };
            Some(PerLayerEmbeddings {
                hidden_size_per_layer_input: cfg.hidden_size_per_layer_input,
                embed_tokens_per_layer: candle_nn::embedding(
                    vocab,
                    cfg.num_hidden_layers * cfg.hidden_size_per_layer_input,
                    vb.pp("embed_tokens_per_layer"),
                )?,
                per_layer_model_projection: candle_nn::linear_no_bias(
                    cfg.hidden_size,
                    cfg.num_hidden_layers * cfg.hidden_size_per_layer_input,
                    vb.pp("per_layer_model_projection"),
                )?,
                per_layer_projection_norm: RmsNorm::new(
                    cfg.hidden_size_per_layer_input,
                    cfg.rms_norm_eps,
                    vb.pp("per_layer_projection_norm"),
                )?,
            })
        } else {
            None
        };
        Ok(Self {
            embed_tokens,
            layers,
            norm,
            lm_head,
            final_logit_softcapping: cfg.final_logit_softcapping,
            device: vb.device().clone(),
            dtype: vb.dtype(),
            hidden_size: cfg.hidden_size,
            sliding_window: cfg.sliding_window,
            ple,
        })
    }

    fn create_attention_masks(
        &self,
        batch_size: usize,
        seq_len: usize,
        seqlen_offset: usize,
    ) -> Result<(Option<Tensor>, Option<Tensor>)> {
        if seq_len <= 1 {
            return Ok((None, None));
        }
        let mask = prepare_decoder_attention_mask(
            batch_size,
            seq_len,
            seqlen_offset,
            None,
            self.dtype,
            &self.device,
        )?;
        let sliding_mask = prepare_decoder_attention_mask(
            batch_size,
            seq_len,
            seqlen_offset,
            Some(self.sliding_window),
            self.dtype,
            &self.device,
        )?;
        Ok((Some(mask), Some(sliding_mask)))
    }

    fn embed_tokens(&self, input_ids: &Tensor) -> Result<Tensor> {
        let xs = self.embed_tokens.forward(input_ids)?;
        xs * (self.hidden_size as f64).sqrt()
    }

    pub fn forward(&mut self, input_ids: &Tensor, seqlen_offset: usize) -> Result<Tensor> {
        let (b_size, seq_len) = input_ids.dims2()?;
        let xs = self.embed_tokens(input_ids)?;
        self.forward_embeds(input_ids, &xs, seqlen_offset, b_size, seq_len)
    }

    fn forward_embeds(
        &mut self,
        input_ids: &Tensor,
        xs: &Tensor,
        seqlen_offset: usize,
        batch_size: usize,
        seq_len: usize,
    ) -> Result<Tensor> {
        let (attention_mask, sliding_attention_mask) =
            self.create_attention_masks(batch_size, seq_len, seqlen_offset)?;
        let n_layers = self.layers.len();
        let per_layer_inputs = match &self.ple {
            Some(ple) => {
                let per_layer_projection = (xs.apply(&ple.per_layer_model_projection)?
                    * (1.0 / (self.hidden_size as f64).sqrt()))?;
                let per_layer_projection = per_layer_projection.reshape((
                    batch_size,
                    seq_len,
                    n_layers,
                    ple.hidden_size_per_layer_input,
                ))?;
                let per_layer_projection =
                    per_layer_projection.apply(&ple.per_layer_projection_norm)?;
                let per_layer_inputs = (input_ids.apply(&ple.embed_tokens_per_layer)?
                    * (ple.hidden_size_per_layer_input as f64).sqrt())?
                .reshape((
                    batch_size,
                    seq_len,
                    n_layers,
                    ple.hidden_size_per_layer_input,
                ))?;
                let mixed = ((per_layer_projection + per_layer_inputs)?
                    * (1.0 / 2.0f64.sqrt()))?;
                Some(mixed)
            }
            None => None,
        };
        let mut shared_kv_states = SharedKvStates::default();
        let mut xs = xs.clone();
        for (i, layer) in self.layers.iter_mut().enumerate() {
            let layer_input = match &per_layer_inputs {
                Some(t) => Some(t.get_on_dim(2, i)?),
                None => None,
            };
            xs = layer.forward(
                &xs,
                attention_mask.as_ref(),
                sliding_attention_mask.as_ref(),
                seqlen_offset,
                layer_input.as_ref(),
                &mut shared_kv_states,
            )?;
        }
        let logits = xs
            .narrow(1, seq_len - 1, 1)?
            .apply(&self.norm)?
            .apply(&self.lm_head)?;
        match self.final_logit_softcapping {
            None => Ok(logits),
            Some(sc) => Ok(((logits / sc)?.tanh()? * sc)?),
        }
    }

    pub fn clear_kv_cache(&mut self) {
        for layer in self.layers.iter_mut() {
            layer.clear_kv_cache()
        }
    }
}
