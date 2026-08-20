//! Detect whatever GPUs are on *this* machine and recommend Hub ids that fit.
//! No SKU table: VRAM comes from nvidia-smi, sysfs, or the largest PCI BAR.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::process::Command;

/// Popular instruct checkpoints with approximate BF16 weight size (GB).
/// This is a model-size catalog, not a GPU list.
const LIBRARY: &[(&str, u32, &str)] = &[
    ("Qwen/Qwen3-1.7B", 4, "Tiny smoke test"),
    ("Qwen/Qwen3-4B", 8, "Small tool-calling"),
    ("Qwen/Qwen3-8B", 16, "Default 16GB-class card"),
    ("Qwen/Qwen3-14B", 28, "Needs a ~32GB card"),
    ("Qwen/Qwen2.5-7B-Instruct", 14, "Instruct, one 16–24GB card"),
    ("Qwen/Qwen2.5-14B-Instruct", 28, "Needs a ~32GB card"),
    (
        "Qwen/Qwen3-32B",
        64,
        "Needs ~64GB (one 80GB or TP=2 × 32GB+)",
    ),
    ("Qwen/Qwen2.5-32B-Instruct", 64, "Needs ~64GB"),
    ("Qwen/Qwen3.6-35B-A3B", 70, "BF16 MoE, needs ~70GB"),
    (
        "Qwen/Qwen3.6-35B-A3B-FP8",
        37,
        "FP8 MoE, needs ~37GB or TP=2",
    ),
    ("meta-llama/Llama-3.1-8B-Instruct", 16, "Llama 8B instruct"),
    (
        "mistralai/Mistral-7B-Instruct-v0.3",
        14,
        "Mistral 7B instruct",
    ),
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelReco {
    pub id: String,
    pub weight_gb: u32,
    pub tp: u32,
    pub recommended: bool,
    pub reason: String,
}

pub fn probe() -> GpuProfile {
    let mut cards = Vec::new();
    cards.extend(probe_nvidia());
    if cards.is_empty() {
        cards.extend(probe_drm());
    }
    let vendor = cards
        .iter()
        .find(|c| !c.igpu)
        .map(|c| c.vendor.clone())
        .unwrap_or_else(|| "none".into());
    let discrete_n = cards.iter().filter(|c| !c.igpu).count();
    // NVIDIA/AMD multi-GPU TP is routine. Intel XPU TP in Docker is still
    // fragile (IPC), so default TP=1 unless the operator opts in.
    let tp_ok = match vendor.as_str() {
        "nvidia" | "amd" => discrete_n >= 2,
        "intel" => {
            std::env::var("LATE_VLLM_ALLOW_TP").ok().as_deref() == Some("1") && discrete_n >= 2
        }
        _ => false,
    };
    summarize(&vendor, cards, tp_ok)
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
    let mut cards = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let parts: Vec<&str> = line.split(',').map(str::trim).collect();
        if parts.len() < 2 {
            continue;
        }
        let mem_mib: u64 = parts[1].parse().unwrap_or(0);
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
        // Intel iGPU is always function 00:02.0 on the CPU complex.
        let igpu = vendor_id == "8086" && pci.contains("00:02.0");
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
        "No discrete GPU found. Local vLLM needs a dedicated GPU.".into()
    } else if vendor == "intel" && discrete_count >= 2 && !tp_ok {
        format!(
            "{} × {} — using one-card recommendations (Intel XPU tensor-parallel is opt-in via LATE_VLLM_ALLOW_TP=1).",
            discrete_count,
            names.first().cloned().unwrap_or_else(|| "GPU".into())
        )
    } else {
        format!("{} discrete GPU(s): {}", discrete_count, names.join(", "))
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
    let mut out = Vec::new();
    for (id, weight_gb, blurb) in LIBRARY {
        let w = u64::from(*weight_gb);
        let tp1_fits = profile.vram_gb > 0 && w <= profile.vram_gb;
        let tp2_fits = profile.tp_ok
            && profile.discrete_count >= 2
            && w <= profile
                .vram_gb
                .saturating_mul(u64::from(profile.discrete_count));
        let (tp, reason) = if tp1_fits {
            let tight = w + 6 > profile.vram_gb;
            (
                1,
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
            (
                2,
                format!(
                    "{blurb}. Split across {} × {}GB (TP=2).",
                    profile.discrete_count, profile.vram_gb
                ),
            )
        } else {
            continue;
        };
        out.push(ModelReco {
            id: (*id).into(),
            weight_gb: *weight_gb,
            tp,
            recommended: true,
            reason,
        });
    }
    out.sort_by_key(|r| r.weight_gb);
    out
}

pub fn default_model(profile: &GpuProfile) -> String {
    let recs = recommend(profile);
    recs.iter()
        .filter(|r| r.tp == 1 && u64::from(r.weight_gb) + 6 <= profile.vram_gb)
        .max_by_key(|r| r.weight_gb)
        .or_else(|| {
            recs.iter()
                .filter(|r| r.recommended)
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
        assert!(!rec.iter().any(|r| r.id.contains("32B") && r.recommended));
        let d = default_model(&intel_32x2());
        assert!(d.contains("14B") || d.contains("8B"));
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
}
