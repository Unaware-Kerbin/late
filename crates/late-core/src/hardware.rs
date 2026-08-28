//! Detect whatever GPUs are on *this* machine and recommend Hub ids that fit.
//! No SKU table: VRAM comes from nvidia-smi, sysfs, or the largest PCI BAR.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::process::Command;

/// Popular SFW instruct checkpoints with approximate BF16 weight size (GB).
/// Catalog only — any valid Hub id can still be typed in Download.
/// No uncensored / abliterated / NSFW fine-tunes.
const LIBRARY: &[(&str, u32, &str)] = &[
    ("Qwen/Qwen3-0.6B", 2, "Tiny Qwen3 smoke test"),
    ("Qwen/Qwen3-1.7B", 4, "Tiny smoke test"),
    ("Qwen/Qwen3-4B", 8, "Small tool-calling"),
    ("Qwen/Qwen3-8B", 16, "Default 16GB-class card"),
    ("Qwen/Qwen3-14B", 28, "Needs a ~32GB card"),
    ("Qwen/Qwen3-32B", 64, "Needs ~64GB (one 80GB or TP=2 × 32GB+)"),
    ("Qwen/Qwen3.5-0.8B", 2, "Qwen3.5 tiny instruct (needs recent vLLM)"),
    ("Qwen/Qwen3.5-2B", 5, "Qwen3.5 2B instruct (needs recent vLLM)"),
    ("Qwen/Qwen3.5-4B", 9, "Qwen3.5 4B instruct (needs recent vLLM)"),
    ("Qwen/Qwen3.5-9B", 19, "Qwen3.5 9B instruct (needs recent vLLM)"),
    ("Qwen/Qwen3.5-27B", 54, "Qwen3.5 27B instruct (needs recent vLLM)"),
    ("Qwen/Qwen3.5-35B-A3B", 70, "Qwen3.5 MoE, needs ~70GB"),
    ("Qwen/Qwen3.8-27B", 53, "Newest Qwen dense 27B (needs recent vLLM)"),
    ("Qwen/Qwen3.6-35B-A3B", 70, "BF16 MoE, needs ~70GB"),
    ("Qwen/Qwen3.6-35B-A3B-FP8", 37, "FP8 MoE, needs ~37GB or TP=2"),
    ("Qwen/Qwen2.5-0.5B-Instruct", 1, "Tiny Qwen2.5 (previous)"),
    ("Qwen/Qwen2.5-1.5B-Instruct", 3, "Small Qwen2.5 (previous)"),
    ("Qwen/Qwen2.5-7B-Instruct", 14, "Instruct, one 16–24GB card (previous)"),
    ("Qwen/Qwen2.5-14B-Instruct", 28, "Needs a ~32GB card (previous)"),
    ("Qwen/Qwen2.5-32B-Instruct", 64, "Needs ~64GB (previous)"),
    ("google/gemma-4-E2B-it", 10, "Gemma 4 E2B instruct (ungated Apache-2.0)"),
    ("google/gemma-4-E4B-it", 16, "Gemma 4 E4B instruct (ungated Apache-2.0)"),
    ("google/gemma-4-12B-it", 23, "Gemma 4 12B instruct (ungated Apache-2.0)"),
    ("google/gemma-4-26B-A4B-it", 50, "Gemma 4 26B-A4B MoE instruct (ungated Apache-2.0)"),
    ("google/gemma-4-31B-it", 62, "Gemma 4 31B instruct (ungated Apache-2.0)"),
    ("google/gemma-3-1b-it", 3, "Gemma 3 tiny instruct (gated Hub, previous)"),
    ("google/gemma-3-4b-it", 8, "Gemma 3 small instruct (gated Hub, previous)"),
    ("google/gemma-3-12b-it", 24, "Gemma 3 12B instruct (gated Hub, previous)"),
    ("google/gemma-3-27b-it", 54, "Gemma 3 27B, needs ~54GB (gated Hub, previous)"),
    ("google/gemma-2-2b-it", 5, "Gemma 2 tiny instruct (gated Hub, previous)"),
    ("google/gemma-2-9b-it", 18, "Gemma 2 9B instruct (gated Hub, previous)"),
    ("google/gemma-2-27b-it", 54, "Gemma 2 27B instruct (gated Hub, previous)"),
    ("meta-llama/Llama-4-Scout-17B-16E-Instruct", 207, "Llama 4 Scout MoE instruct (gated Hub)"),
    ("meta-llama/Llama-3.3-70B-Instruct", 140, "Llama 3.3 70B instruct (gated Hub)"),
    ("meta-llama/Llama-3.2-1B-Instruct", 3, "Llama 3.2 tiny (gated Hub)"),
    ("meta-llama/Llama-3.2-3B-Instruct", 7, "Llama 3.2 3B instruct (gated Hub)"),
    ("meta-llama/Llama-3.1-8B-Instruct", 16, "Llama 8B instruct (gated Hub, previous)"),
    ("meta-llama/Llama-3.1-70B-Instruct", 140, "Llama 3.1 70B instruct (gated Hub, previous)"),
    ("mistralai/Mistral-Small-3.2-24B-Instruct-2506", 48, "Mistral Small 3.2 24B instruct"),
    ("mistralai/Ministral-8B-Instruct-2410", 16, "Ministral 8B instruct"),
    ("mistralai/Mistral-7B-Instruct-v0.3", 14, "Mistral 7B instruct (previous)"),
    ("microsoft/Phi-4-mini-instruct", 8, "Phi-4 mini instruct"),
    ("microsoft/phi-4", 28, "Phi-4, needs ~32GB"),
    ("ibm-granite/granite-4.2-3b", 7, "Granite 4.2 3B instruct"),
    ("ibm-granite/granite-4.2-8b", 17, "Granite 4.2 8B instruct"),
    ("ibm-granite/granite-4.2-30b", 60, "Granite 4.2 30B instruct"),
    ("ibm-granite/granite-3.3-8b-instruct", 16, "Granite 3.3 8B instruct (previous)"),
    ("allenai/Olmo-3-7B-Instruct", 14, "OLMo 3 7B instruct"),
    ("allenai/Olmo-3.1-32B-Instruct", 64, "OLMo 3.1 32B instruct"),
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuCard {
    pub name: String,
    pub pci: String,
    pub vendor: String,
    pub igpu: bool,
    pub vram_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuProfile {
    pub vendor: String,
    pub discrete_count: u32,
    pub vram_gb: u64,
    pub vram_bytes_each: u64,
    pub tp_ok: bool,
    pub summary: String,
    pub cards: Vec<GpuCard>,
}

/// How Local Start should use GPUs on this computer (not a multi-machine cluster).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GpuLaunchPlan {
    pub multi_visible: bool,
    pub use_all: bool,
    pub device_count: u32,
    pub tensor_parallel: u32,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelReco {
    pub id: String,
    pub weight_gb: u32,
    pub tp: u32,
    pub recommended: bool,
    #[serde(default)]
    pub newest: bool,
    pub reason: String,
}

pub fn probe() -> GpuProfile {
    let mut cards = Vec::new();
    cards.extend(probe_nvidia());
    if cards.is_empty() {
        cards.extend(probe_amd());
    }
    if cards.is_empty() {
        cards.extend(probe_drm());
    }
    let vendor = cards
        .iter()
        .find(|c| !c.igpu)
        .map(|c| c.vendor.clone())
        .unwrap_or_else(|| "none".into());
    let discrete_n = cards.iter().filter(|c| !c.igpu).count();
    // NVIDIA/AMD tensor-parallel is routine. Intel XPU TP in Docker works but
    // can fail on IPC — the Agent pane checkbox is the opt-in (default on).
    let tp_ok = matches!(vendor.as_str(), "nvidia" | "amd" | "intel") && discrete_n >= 2;
    summarize(&vendor, cards, tp_ok)
}

/// Recommendations and Start fit checks follow the “use all GPUs” choice.
pub fn serving_profile(profile: &GpuProfile, use_all: bool) -> GpuProfile {
    let mut p = profile.clone();
    p.tp_ok = profile.tp_ok && use_all;
    p
}

pub fn device_mask(count: u32) -> String {
    let n = count.max(1);
    (0..n)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

pub fn launch_plan(profile: &GpuProfile, use_all: bool) -> GpuLaunchPlan {
    let n = profile.discrete_count;
    let multi = n >= 2;
    let want_all = use_all && n >= 2 && matches!(profile.vendor.as_str(), "nvidia" | "amd" | "intel");
    let tensor_parallel = if want_all { n } else { 1 };
    let note = if n == 0 {
        "No discrete GPU found on this computer.".into()
    } else if n == 1 {
        "One GPU on this computer.".into()
    } else if want_all {
        match profile.vendor.as_str() {
            "intel" => format!(
                "Using all {n} GPUs on this computer. Intel Docker split can fail — uncheck and Start again to use one card."
            ),
            "nvidia" | "amd" => format!("Using all {n} GPUs on this computer."),
            _ => format!(
                "{n} GPUs found, but this Start path can only use one."
            ),
        }
    } else {
        format!("Using one GPU on this computer ({n} found).")
    };
    GpuLaunchPlan {
        multi_visible: multi || n >= 2,
        use_all: want_all,
        device_count: if want_all { n } else { n.min(1) },
        tensor_parallel,
        note,
    }
}

/// llama-server flags. Empty when there is no discrete GPU (CPU builds reject `-ngl`).
pub fn llama_gpu_args(profile: &GpuProfile, use_all: bool) -> Vec<String> {
    if profile.discrete_count == 0 {
        return vec![];
    }
    let mut args = vec!["-ngl".into(), "99".into()];
    let plan = launch_plan(profile, use_all);
    if plan.use_all && plan.tensor_parallel >= 2 {
        args.push("-sm".into());
        args.push("layer".into());
        args.push("-ts".into());
        args.push(
            (0..plan.tensor_parallel)
                .map(|_| "1")
                .collect::<Vec<_>>()
                .join(","),
        );
    }
    args
}

/// Device pin for llama-server. Ollama is left alone (it already uses every GPU).
pub fn llama_gpu_env(profile: &GpuProfile, use_all: bool) -> Vec<(String, String)> {
    if profile.discrete_count == 0 {
        return vec![];
    }
    let n = if use_all {
        profile.discrete_count.max(1)
    } else {
        1
    };
    let mask = device_mask(n);
    match profile.vendor.as_str() {
        "nvidia" => vec![("CUDA_VISIBLE_DEVICES".into(), mask)],
        "amd" => vec![
            ("HIP_VISIBLE_DEVICES".into(), mask.clone()),
            ("ROCR_VISIBLE_DEVICES".into(), mask),
        ],
        "intel" => vec![(
            "ONEAPI_DEVICE_SELECTOR".into(),
            if n > 1 {
                "level_zero:gpu".into()
            } else {
                "level_zero:0".into()
            },
        )],
        _ => vec![],
    }
}

/// `docker/compose.yml` is an Intel XPU example. NVIDIA/AMD Start must not pull that image.
pub fn allow_intel_xpu_compose(vendor: &str) -> bool {
    vendor.eq_ignore_ascii_case("intel")
}

pub fn intel_xpu_compose_refuse(vendor: &str) -> String {
    match vendor.to_ascii_lowercase().as_str() {
        "nvidia" => {
            "This Start button runs the optional Intel XPU Docker example, not CUDA. Use Ollama, llama.cpp, or your own vLLM at http://127.0.0.1:8000/v1. Set LATE_VLLM_FORCE=1 only if you mean to run docker/compose.yml anyway.".into()
        }
        "amd" => {
            "This Start button runs the optional Intel XPU Docker example, not ROCm. Use Ollama, llama.cpp, or your own vLLM at http://127.0.0.1:8000/v1. Set LATE_VLLM_FORCE=1 only if you mean to run docker/compose.yml anyway.".into()
        }
        _ => {
            "No Intel discrete GPU for docker/compose.yml. Use Ollama, llama.cpp, or your own vLLM on loopback. Set LATE_VLLM_FORCE=1 only if you mean to run that Intel example anyway.".into()
        }
    }
}

fn probe_nvidia() -> Vec<GpuCard> {
    let out = Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,memory.total,pci.bus_id",
            "--format=csv,noheader,nounits",
        ])
        .output();
    let Ok(out) = out else {
        return vec![];
    };
    if !out.status.success() {
        return vec![];
    }
    parse_nvidia_smi_csv(&String::from_utf8_lossy(&out.stdout))
}

/// nvidia-smi `--format=csv,noheader,nounits` rows: name, memory.total (MiB), pci.bus_id.
pub fn parse_nvidia_smi_csv(text: &str) -> Vec<GpuCard> {
    let mut cards = Vec::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.split(',').map(str::trim).collect();
        if parts.len() < 2 {
            continue;
        }
        let mem_mib: u64 = parts[1].parse().unwrap_or(0);
        if parts[0].is_empty() || mem_mib == 0 {
            continue;
        }
        cards.push(GpuCard {
            name: parts[0].to_string(),
            pci: parts.get(2).unwrap_or(&"").to_string(),
            vendor: "nvidia".into(),
            igpu: false,
            vram_bytes: mem_mib * 1024 * 1024,
        });
    }
    cards
}

fn probe_amd() -> Vec<GpuCard> {
    let out = Command::new("rocm-smi")
        .args(["--showmeminfo", "vram"])
        .output();
    let Ok(out) = out else {
        return vec![];
    };
    if !out.status.success() {
        return vec![];
    }
    parse_rocm_smi_meminfo(&String::from_utf8_lossy(&out.stdout))
}

/// `rocm-smi --showmeminfo vram` lines such as `GPU[0] ... vram Total Memory (B): 17163091968`.
pub fn parse_rocm_smi_meminfo(text: &str) -> Vec<GpuCard> {
    let mut cards = Vec::new();
    for line in text.lines() {
        let lower = line.to_ascii_lowercase();
        if !lower.contains("vram") || !lower.contains("gpu[") {
            continue;
        }
        let Some(idx_start) = line.find("GPU[") else {
            continue;
        };
        let rest = &line[idx_start + 4..];
        let Some(idx_end) = rest.find(']') else {
            continue;
        };
        let pci = rest[..idx_end].trim().to_string();
        let bytes = line
            .rsplit(':')
            .next()
            .and_then(|s| {
                s.split_whitespace()
                    .next()
                    .and_then(|n| n.parse::<u64>().ok())
            })
            .unwrap_or(0);
        if bytes < 1_000_000 {
            continue;
        }
        cards.push(GpuCard {
            name: format!("AMD GPU {pci}"),
            pci,
            vendor: "amd".into(),
            igpu: false,
            vram_bytes: bytes,
        });
    }
    cards
}

fn probe_drm() -> Vec<GpuCard> {
    let Ok(rd) = fs::read_dir("/sys/class/drm") else {
        return vec![];
    };
    let mut cards = Vec::new();
    for e in rd.flatten() {
        let fname = e.file_name();
        let name = fname.to_string_lossy();
        if !name.starts_with("card") || name.contains('-') {
            continue;
        }
        let dev = e.path().join("device");
        if !dev.join("vendor").is_file() {
            continue;
        }
        let vendor_id = read_hex(&dev.join("vendor"));
        let device_id = read_hex(&dev.join("device"));
        let pci = fs::canonicalize(&dev)
            .ok()
            .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_default();
        let vendor_name = match vendor_id.as_str() {
            "8086" => "intel",
            "10de" => "nvidia",
            "1002" => "amd",
            other => other,
        };
        let igpu = vendor_id == "8086" && intel_drm_is_igpu(&pci);
        let vram = read_vram_sysfs(&dev)
            .or_else(|| lspci_prefetch_bytes(&pci))
            .unwrap_or(0);
        let pretty = lspci_name(&pci)
            .or_else(|| read_trimmed(dev.join("label")))
            .unwrap_or_else(|| format!("{vendor_name}:{device_id}"));
        cards.push(GpuCard {
            name: pretty,
            pci,
            vendor: vendor_name.into(),
            igpu,
            vram_bytes: vram,
        });
    }
    cards.sort_by(|a, b| a.pci.cmp(&b.pci));
    cards
}

/// Missing PCI basename fail-closes as iGPU so compose Start cannot treat an
/// unresolved Intel DRM node as a discrete XPU.
fn intel_drm_is_igpu(pci: &str) -> bool {
    pci.is_empty() || pci.contains("00:02.0")
}

fn read_hex(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_default()
        .trim()
        .trim_start_matches("0x")
        .to_ascii_lowercase()
}

fn read_trimmed(path: std::path::PathBuf) -> Option<String> {
    let s = fs::read_to_string(path).ok()?;
    let s = s.trim();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

fn read_vram_sysfs(dev: &Path) -> Option<u64> {
    for rel in [
        "mem_info_vram_total",
        "mem_info_vis_vram_total",
        "lmem_total_bytes",
    ] {
        if let Ok(s) = fs::read_to_string(dev.join(rel)) {
            if let Ok(n) = s.trim().parse::<u64>() {
                if n > 1_000_000 {
                    return Some(n);
                }
            }
        }
    }
    // xe / i915 sometimes expose local memory one directory down.
    let Ok(rd) = fs::read_dir(dev) else {
        return None;
    };
    let mut best = 0u64;
    for e in rd.flatten() {
        let n = e.file_name();
        let ns = n.to_string_lossy().to_ascii_lowercase();
        if !(ns.contains("vram") || ns.contains("lmem") || ns.contains("lmem")) {
            continue;
        }
        if let Ok(s) = fs::read_to_string(e.path()) {
            if let Ok(v) = s.trim().parse::<u64>() {
                if v > best {
                    best = v;
                }
            }
        }
    }
    (best > 1_000_000).then_some(best)
}

/// Largest prefetchable PCI BAR is a decent VRAM stand-in when sysfs is silent.
fn lspci_prefetch_bytes(bdf: &str) -> Option<u64> {
    if bdf.is_empty() {
        return None;
    }
    let out = Command::new("lspci")
        .args(["-v", "-s", bdf])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut best = 0u64;
    for line in text.lines() {
        if !line.contains("prefetchable") {
            continue;
        }
        if let Some(bytes) = parse_pci_size(line) {
            if bytes > best {
                best = bytes;
            }
        }
    }
    (best >= 1 << 30).then_some(best)
}

fn parse_pci_size(line: &str) -> Option<u64> {
    let start = line.find("[size=")?;
    let rest = &line[start + 6..];
    let end = rest.find(']')?;
    let tok = &rest[..end];
    let (num, mul) = tok.split_at(tok.len().saturating_sub(1));
    let n: u64 = num.parse().ok()?;
    Some(match mul {
        "K" | "k" => n << 10,
        "M" | "m" => n << 20,
        "G" | "g" => n << 30,
        _ => n,
    })
}

fn lspci_name(bdf: &str) -> Option<String> {
    if bdf.is_empty() {
        return None;
    }
    let out = Command::new("lspci")
        .args(["-s", bdf, "-nn"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&out.stdout);
    let line = line.lines().next()?.trim();
    let name = line.split(": ").nth(1)?.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn summarize(vendor: &str, cards: Vec<GpuCard>, tp_ok: bool) -> GpuProfile {
    let discrete: Vec<&GpuCard> = cards.iter().filter(|c| !c.igpu).collect();
    let discrete_count = discrete.len() as u32;
    let vram_bytes_each = discrete.iter().map(|c| c.vram_bytes).min().unwrap_or(0);
    let vram_gb = vram_bytes_each / (1 << 30);
    let names: Vec<String> = discrete
        .iter()
        .map(|c| {
            if c.vram_bytes > 0 {
                format!("{} ({}GB)", c.name, c.vram_bytes / (1 << 30))
            } else {
                c.name.clone()
            }
        })
        .collect();
    let summary = if discrete.is_empty() {
        "No discrete GPU found. Use llama.cpp on CPU, Ollama, or point Local at a server on loopback.".into()
    } else if vendor != "intel" {
        format!(
            "{} GPU(s) on this computer: {}. llama.cpp Start can use them. vLLM Start here is the Intel Docker example — for CUDA/ROCm start vLLM yourself or via MCP.",
            discrete_count,
            names.join(", ")
        )
    } else {
        format!("{} GPU(s) on this computer: {}", discrete_count, names.join(", "))
    };
    GpuProfile {
        vendor: vendor.into(),
        discrete_count,
        vram_gb,
        vram_bytes_each,
        tp_ok,
        summary,
        cards,
    }
}

pub fn recommend(profile: &GpuProfile) -> Vec<ModelReco> {
    rank_weight_catalog(LIBRARY, profile, true)
}

/// llama.cpp / Ollama: list every catalog row. `recommended` = fits; `newest` is separate.
pub fn rank_quant_catalog(
    library: &[(&'static str, u32, &'static str)],
    profile: &GpuProfile,
) -> Vec<ModelReco> {
    let budget = if profile.vram_gb == 0 {
        8
    } else {
        profile.vram_gb
    };
    let newest = newest_id_set(library.iter().map(|(id, _, _)| *id));
    library
        .iter()
        .map(|(id, gb, blurb)| {
            let fits = u64::from(*gb) <= budget.saturating_add(2);
            let is_newest = newest.contains(*id);
            let reason = if fits {
                if profile.vram_gb == 0 {
                    format!("{blurb}. Fine on CPU or a small GPU (~{gb}GB).")
                } else {
                    format!("{blurb}. Fits ~{}GB on this computer.", profile.vram_gb)
                }
            } else {
                format!("{blurb}. Too big for ~{budget}GB (needs ~{gb}GB).")
            };
            ModelReco {
                id: (*id).into(),
                weight_gb: *gb,
                tp: 1,
                recommended: fits,
                newest: is_newest,
                reason,
            }
        })
        .collect()
}

fn rank_weight_catalog(
    library: &[(&'static str, u32, &'static str)],
    profile: &GpuProfile,
    allow_tp: bool,
) -> Vec<ModelReco> {
    let newest = newest_id_set(library.iter().map(|(id, _, _)| *id));
    library
        .iter()
        .map(|(id, weight_gb, blurb)| {
            let w = u64::from(*weight_gb);
            let tp1_fits = profile.vram_gb > 0 && w <= profile.vram_gb;
            let tp_n = if allow_tp && profile.tp_ok && profile.discrete_count >= 2 {
                u64::from(profile.discrete_count)
            } else {
                1
            };
            let tp2_fits = tp_n >= 2 && w <= profile.vram_gb.saturating_mul(tp_n);
            let is_newest = newest.contains(*id);
            let (tp, fits, reason) = if tp1_fits {
                let tight = w + 6 > profile.vram_gb;
                (
                    1u32,
                    true,
                    if tight {
                        format!(
                            "{blurb}. Tight on {}GB (TP=1, keep context short).",
                            profile.vram_gb
                        )
                    } else {
                        format!("{blurb}. Fits one {}GB GPU (TP=1).", profile.vram_gb)
                    },
                )
            } else if tp2_fits {
                let tp = profile.discrete_count.max(2);
                (
                    tp,
                    true,
                    format!(
                        "{blurb}. Needs all GPUs on this computer: split across {} × {}GB (TP={tp}).",
                        profile.discrete_count, profile.vram_gb
                    ),
                )
            } else if profile.vram_gb == 0 {
                (
                    1,
                    false,
                    format!("{blurb}. Too big without a discrete GPU (~{weight_gb}GB weights)."),
                )
            } else {
                (
                    1,
                    false,
                    format!(
                        "{blurb}. Too big for {} × {}GB (needs ~{weight_gb}GB).",
                        profile.discrete_count.max(1),
                        profile.vram_gb
                    ),
                )
            };
            ModelReco {
                id: (*id).into(),
                weight_gb: *weight_gb,
                tp,
                recommended: fits,
                newest: is_newest,
                reason,
            }
        })
        .collect()
}

pub fn lineage_of(id: &str) -> &'static str {
    let l = id.to_ascii_lowercase();
    if l.contains("qwen") {
        "qwen"
    } else if l.contains("gemma") {
        "gemma"
    } else if l.contains("llama") {
        "llama"
    } else if l.contains("mistral") || l.contains("ministral") {
        "mistral"
    } else if l.contains("phi") {
        "phi"
    } else if l.contains("granite") {
        "granite"
    } else if l.contains("olmo") {
        "olmo"
    } else if l.contains("deepseek") {
        "deepseek"
    } else {
        "other"
    }
}

pub fn generation_milli(id: &str) -> u16 {
    let l = id.to_ascii_lowercase();
    if l.contains("qwen3.8") {
        380
    } else if l.contains("qwen3.6") || l.contains("qwen3.5") {
        350
    } else if l.contains("qwen3") {
        300
    } else if l.contains("qwen2.5") {
        250
    } else if l.contains("gemma-4") || l.contains("gemma4") {
        400
    } else if l.contains("gemma-3") || l.contains("gemma3") {
        300
    } else if l.contains("gemma-2") || l.contains("gemma2") {
        200
    } else if l.contains("llama-4") || l.contains("llama4") {
        400
    } else if l.contains("llama-3.3") || l.contains("llama3.3") {
        330
    } else if l.contains("llama-3.2") || l.contains("llama3.2") {
        320
    } else if l.contains("llama-3.1") || l.contains("llama3.1") {
        310
    } else if l.contains("granite-4") || l.contains("granite4") {
        420
    } else if l.contains("granite-3.3") || l.contains("granite3.3") {
        330
    } else if l.contains("olmo-3") || l.contains("olmo3") {
        300
    } else if l.contains("ministral") {
        800
    } else if l.contains("mistral-small-3.2") || l.contains("mistral-small") {
        320
    } else if l.contains("phi-4") || l.contains("phi4") {
        400
    } else {
        0
    }
}

pub fn size_slot(id: &str) -> &'static str {
    let l = id.to_ascii_lowercase();
    if l.contains("scout") || l.contains("70b") || l.contains("72b") || l.contains(":70") {
        "70b"
    } else if l.contains("a3b") || l.contains("16e") {
        "moe"
    } else if l.contains("0.8") || l.contains("0.6") || l.contains("0.5") {
        "tiny"
    } else if l.contains("1.7") {
        "2b"
    } else if l.contains("32b")
        || l.contains("35b")
        || l.contains("31b")
        || l.contains("30b")
        || l.contains("27b")
        || l.contains("26b")
        || l.contains("24b")
    {
        "32b"
    } else if l.contains("14b") || l.contains("13b") {
        "14b"
    } else if l.contains("12b") || l.contains("9b") || l.contains("e4b") {
        "12b"
    } else if l.contains("8b") || l.contains("7b") {
        "8b"
    } else if l.contains("4b") || l.contains("3b") || l.contains("e2b") {
        "4b"
    } else if l.contains("2b") || l.contains("1b") {
        "2b"
    } else if l.contains("llama3.3") {
        "70b"
    } else if l.contains("llama3.2") {
        "4b"
    } else {
        "tiny"
    }
}

fn newest_id_set<'a>(ids: impl Iterator<Item = &'a str>) -> HashSet<&'a str> {
    let rows: Vec<&str> = ids.collect();
    let mut best: std::collections::HashMap<String, u16> = std::collections::HashMap::new();
    for id in &rows {
        let key = format!("{}:{}", lineage_of(id), size_slot(id));
        let gen = generation_milli(id);
        let cur = best.get(&key).copied().unwrap_or(0);
        if gen > cur {
            best.insert(key, gen);
        }
    }
    rows.into_iter()
        .filter(|id| {
            let key = format!("{}:{}", lineage_of(id), size_slot(id));
            generation_milli(id) >= best.get(&key).copied().unwrap_or(0)
        })
        .collect()
}

pub fn default_model(profile: &GpuProfile) -> String {
    let recs = recommend(profile);
    recs.iter()
        .filter(|r| r.recommended && r.newest && r.tp == 1 && u64::from(r.weight_gb) + 6 <= profile.vram_gb)
        .max_by_key(|r| r.weight_gb)
        .or_else(|| {
            recs.iter()
                .filter(|r| r.recommended && r.newest)
                .max_by_key(|r| r.weight_gb)
        })
        .map(|r| r.id.clone())
        .unwrap_or_else(|| "Qwen/Qwen3-8B".into())
}

pub fn estimate_weight_gb(id: &str) -> Option<u32> {
    LIBRARY
        .iter()
        .find(|(i, _, _)| *i == id)
        .map(|(_, g, _)| *g)
}

/// Whether this Hub id should start on the probed GPUs.
pub fn fits(id: &str, profile: &GpuProfile) -> Result<(), String> {
    let Some(w) = estimate_weight_gb(id) else {
        return Ok(()); // unknown: let vLLM try
    };
    let w = u64::from(w);
    if w <= profile.vram_gb {
        return Ok(());
    }
    if profile.tp_ok
        && profile.discrete_count >= 2
        && w <= profile.vram_gb * u64::from(profile.discrete_count)
    {
        return Ok(());
    }
    Err(format!(
        "{id} wants ~{w}GB of weights; detected {} × {}GB ({}). Pick a recommended model.",
        profile.discrete_count.max(1),
        profile.vram_gb,
        profile.summary
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intel_32x2() -> GpuProfile {
        GpuProfile {
            vendor: "intel".into(),
            discrete_count: 2,
            vram_gb: 32,
            vram_bytes_each: 32 << 30,
            tp_ok: false,
            summary: "test".into(),
            cards: vec![],
        }
    }

    #[test]
    fn pci_size_parse() {
        assert_eq!(
            parse_pci_size("\t\tRegion 2: Memory at 80000000 (64-bit, prefetchable) [size=32G]"),
            Some(32 << 30)
        );
    }

    #[test]
    fn thirty_two_gb_picks_8b_or_14b_not_32b() {
        let rec = recommend(&intel_32x2());
        assert!(rec.iter().any(|r| r.id == "Qwen/Qwen3-8B" && r.recommended));
        assert!(rec
            .iter()
            .any(|r| r.id == "Qwen/Qwen3-14B" && r.recommended));
        assert!(rec
            .iter()
            .any(|r| r.id == "google/gemma-4-12B-it" && r.recommended));
        assert!(rec.iter().any(|r| r.id.contains("32B") && !r.recommended));
        assert_eq!(rec.len(), LIBRARY.len());
        assert!(LIBRARY.len() > 30, "vLLM catalog is not a 2-row slice");
        assert!(rec.iter().any(|r| r.id == "Qwen/Qwen2.5-7B-Instruct" && !r.newest));
        assert!(rec.iter().any(|r| r.id == "Qwen/Qwen3-8B" && r.newest));
        assert!(rec.iter().any(|r| r.id == "google/gemma-4-12B-it" && r.newest));
        assert!(rec.iter().any(|r| r.id == "google/gemma-3-12b-it" && !r.newest));
        assert!(rec.iter().any(|r| r.id == "google/gemma-2-9b-it" && !r.newest));
        let d = default_model(&intel_32x2());
        assert!(
            d.contains("12b")
                || d.contains("12B")
                || d.contains("14B")
                || d.contains("8B")
                || d.contains("E4B")
                || d.contains("phi-4"),
            "unexpected default {d}"
        );
    }

    #[test]
    fn catalog_includes_sfw_families_not_uncensored() {
        let ids: String = LIBRARY
            .iter()
            .map(|(id, _, _)| *id)
            .collect::<Vec<_>>()
            .join(" ");
        assert!(ids.contains("google/gemma-4-E4B-it"));
        assert!(ids.contains("google/gemma-4-E2B-it"));
        assert!(ids.contains("google/gemma-3-4b-it"));
        assert!(ids.contains("google/gemma-2-9b-it"));
        assert!(ids.contains("Qwen/Qwen3.8-27B"));
        assert!(ids.contains("Qwen/Qwen2.5-7B-Instruct"));
        assert!(ids.contains("meta-llama/Llama-3.3-70B-Instruct"));
        assert!(ids.contains("meta-llama/Llama-3.2-3B-Instruct"));
        assert!(ids.contains("microsoft/Phi-4-mini-instruct"));
        assert!(!ids.to_ascii_lowercase().contains("uncensored"));
        assert!(!ids.to_ascii_lowercase().contains("abliterat"));
        assert!(!ids.to_ascii_lowercase().contains("dolphin"));
    }

    #[test]
    fn nvidia_2x40_can_tp_32b() {
        let p = GpuProfile {
            vendor: "nvidia".into(),
            discrete_count: 2,
            vram_gb: 40,
            vram_bytes_each: 40 << 30,
            tp_ok: true,
            summary: "test".into(),
            cards: vec![],
        };
        let rec = recommend(&p);
        assert!(rec
            .iter()
            .any(|r| r.id == "Qwen/Qwen3-32B" && r.recommended && r.tp == 2));
    }

    #[test]
    fn intel_compose_is_not_the_nvidia_path() {
        assert!(allow_intel_xpu_compose("intel"));
        assert!(allow_intel_xpu_compose("Intel"));
        assert!(!allow_intel_xpu_compose("nvidia"));
        assert!(!allow_intel_xpu_compose("amd"));
        assert!(!allow_intel_xpu_compose("none"));
        assert!(intel_xpu_compose_refuse("nvidia").contains("CUDA"));
        assert!(intel_xpu_compose_refuse("amd").contains("ROCm"));
    }

    #[test]
    fn missing_pci_basename_does_not_count_intel_as_discrete() {
        assert!(intel_drm_is_igpu(""));
        assert!(intel_drm_is_igpu("0000:00:02.0"));
        assert!(!intel_drm_is_igpu("0000:03:00.0"));
    }

    fn nvidia_pair() -> GpuProfile {
        GpuProfile {
            vendor: "nvidia".into(),
            discrete_count: 2,
            vram_gb: 48,
            vram_bytes_each: 48 << 30,
            tp_ok: true,
            summary: "test".into(),
            cards: vec![],
        }
    }

    #[test]
    fn parse_nvidia_smi_counts_two_cards() {
        let csv = "NVIDIA RTX A6000, 49140, 00000000:01:00.0\nNVIDIA RTX A6000, 49140, 00000000:02:00.0\n";
        let cards = parse_nvidia_smi_csv(csv);
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].vendor, "nvidia");
        assert!(cards.iter().all(|c| c.vram_bytes > 40 << 30));
    }

    #[test]
    fn parse_rocm_smi_counts_two_cards() {
        let text = "\
GPU[0]          : vram Total Memory (B): 17163091968
GPU[1]          : vram Total Memory (B): 17163091968
";
        let cards = parse_rocm_smi_meminfo(text);
        assert_eq!(cards.len(), 2);
        assert!(cards.iter().all(|c| c.vendor == "amd"));
    }

    #[test]
    fn launch_plan_defaults_to_all_visible_gpus() {
        let p = nvidia_pair();
        let all = launch_plan(&p, true);
        assert!(all.multi_visible);
        assert!(all.use_all);
        assert_eq!(all.tensor_parallel, 2);
        assert_eq!(all.device_count, 2);
        assert_eq!(device_mask(2), "0,1");
        let one = launch_plan(&p, false);
        assert_eq!(one.tensor_parallel, 1);
        assert_eq!(one.device_count, 1);
        assert!(!one.use_all);
        assert!(serving_profile(&p, false).tp_ok == false);
        assert!(serving_profile(&p, true).tp_ok);
    }

    #[test]
    fn llama_argv_and_env_follow_use_all() {
        let p = nvidia_pair();
        let all = llama_gpu_args(&p, true);
        assert_eq!(all, vec!["-ngl", "99", "-sm", "layer", "-ts", "1,1"]);
        let env_all = llama_gpu_env(&p, true);
        assert_eq!(
            env_all,
            vec![("CUDA_VISIBLE_DEVICES".into(), "0,1".into())]
        );
        let one = llama_gpu_args(&p, false);
        assert_eq!(one, vec!["-ngl", "99"]);
        let env_one = llama_gpu_env(&p, false);
        assert_eq!(
            env_one,
            vec![("CUDA_VISIBLE_DEVICES".into(), "0".into())]
        );
        let amd = GpuProfile {
            vendor: "amd".into(),
            discrete_count: 2,
            vram_gb: 24,
            vram_bytes_each: 24 << 30,
            tp_ok: true,
            summary: "test".into(),
            cards: vec![],
        };
        let hip = llama_gpu_env(&amd, true);
        assert!(hip.iter().any(|(k, v)| k == "HIP_VISIBLE_DEVICES" && v == "0,1"));
        let none = GpuProfile {
            vendor: "none".into(),
            discrete_count: 0,
            vram_gb: 0,
            vram_bytes_each: 0,
            tp_ok: false,
            summary: "test".into(),
            cards: vec![],
        };
        assert!(llama_gpu_args(&none, true).is_empty());
        assert!(llama_gpu_env(&none, true).is_empty());
    }

    #[test]
    fn intel_all_gpus_uses_compose_tp() {
        let p = GpuProfile {
            vendor: "intel".into(),
            discrete_count: 2,
            vram_gb: 32,
            vram_bytes_each: 32 << 30,
            tp_ok: true,
            summary: "test".into(),
            cards: vec![],
        };
        let plan = launch_plan(&p, true);
        assert_eq!(plan.tensor_parallel, 2);
        assert!(plan.note.contains("Intel"));
        let env = llama_gpu_env(&p, false);
        assert_eq!(
            env,
            vec![("ONEAPI_DEVICE_SELECTOR".into(), "level_zero:0".into())]
        );
    }

    #[test]
    fn live_probe_never_panics_without_gpu() {
        let p = probe();
        let _ = launch_plan(&p, true);
        let _ = llama_gpu_args(&p, true);
        let _ = llama_gpu_env(&p, false);
    }
}
