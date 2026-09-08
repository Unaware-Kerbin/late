//! Vendor-agnostic GPU pin for compile / serve.
//!
//! Empty `LATE_INFER_ACCEL` is **not** NVIDIA: probe PCI/DRM (and OneAPI/ZE vs
//! CUDA vs HIP env). Idle-primary vendor comes from detect (NVIDIA → CUDA,
//! Intel → OpenVINO GenAI / Level Zero, AMD → HIP when wired). Intel-only
//! hardware must never bind CUDA 0 or start on CPU as if that were the
//! discrete card on your computer.

use candle_core::Device;
use std::fs;
use std::path::Path;
use std::process::Command;

/// PCI vendor IDs (sysfs / lspci). Source of truth — not a marketing name.
const PCI_INTEL: &str = "8086";
const PCI_NVIDIA: &str = "10de";
const PCI_AMD: &str = "1002";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccelKind {
    Intel,
    Nvidia,
    Amd,
}

impl AccelKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AccelKind::Intel => "intel",
            AccelKind::Nvidia => "nvidia",
            AccelKind::Amd => "amd",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuCard {
    pub vendor: AccelKind,
    pub pci_id: Option<String>,
    pub device_id: Option<String>,
    pub igpu: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccelPlan {
    pub accel: Option<AccelKind>,
    /// `LATE_INFER_ACCEL` after aliases (`xpu`→intel, `cuda`→nvidia, `rocm`→amd).
    pub pin: Option<AccelKind>,
    pub toolkit: Option<AccelKind>,
    pub pci_id: Option<String>,
    pub cards: Vec<GpuCard>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompileAccelMeta {
    pub vendor: Option<String>,
    pub accel: Option<String>,
    pub pci_id: Option<String>,
    pub idle: Option<bool>,
    pub display: Option<bool>,
}

/// Parse `intel|nvidia|amd` (and OneAPI/CUDA/HIP aliases). Unknown tokens fail closed.
pub fn parse_accel_token(raw: &str) -> Result<Option<AccelKind>, String> {
    let a = raw.trim().to_ascii_lowercase();
    if a.is_empty() {
        return Ok(None);
    }
    match a.as_str() {
        "intel" | "xpu" | "level_zero" | "level-zero" | "oneapi" => Ok(Some(AccelKind::Intel)),
        "nvidia" | "cuda" => Ok(Some(AccelKind::Nvidia)),
        "amd" | "rocm" | "hip" => Ok(Some(AccelKind::Amd)),
        _ => Err(format!(
            "LATE_INFER_ACCEL={raw:?} is not intel|nvidia|amd. late-infer will not guess NVIDIA on your computer."
        )),
    }
}

fn env_trim(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn env_set_nonempty(name: &str) -> bool {
    match std::env::var(name) {
        Ok(s) => {
            let t = s.trim();
            !t.is_empty() && t != "-1"
        }
        Err(_) => false,
    }
}

/// OneAPI / Level Zero vs CUDA vs HIP from the process environment.
pub fn toolkit_from_env() -> Result<Option<AccelKind>, String> {
    let ze = env_set_nonempty("ONEAPI_DEVICE_SELECTOR")
        || env_set_nonempty("ZE_AFFINITY_MASK")
        || env_set_nonempty("ZE_FLAT_DEVICE_HIERARCHY");
    let cuda = env_set_nonempty("CUDA_VISIBLE_DEVICES");
    let hip = env_set_nonempty("HIP_VISIBLE_DEVICES") || env_set_nonempty("ROCR_VISIBLE_DEVICES");
    match (ze, cuda, hip) {
        (false, false, false) => Ok(None),
        (true, false, false) => Ok(Some(AccelKind::Intel)),
        (false, true, false) => Ok(Some(AccelKind::Nvidia)),
        (false, false, true) => Ok(Some(AccelKind::Amd)),
        _ => Err(
            "GPU toolkit env is mixed (OneAPI/ZE vs CUDA vs HIP) on your computer. Set LATE_INFER_ACCEL=intel|nvidia|amd and keep the matching selector only."
                .into(),
        ),
    }
}

pub fn vendor_from_pci_id(id: &str) -> Option<AccelKind> {
    match id.trim().trim_start_matches("0x").to_ascii_lowercase().as_str() {
        PCI_INTEL => Some(AccelKind::Intel),
        PCI_NVIDIA => Some(AccelKind::Nvidia),
        PCI_AMD => Some(AccelKind::Amd),
        _ => None,
    }
}

/// Arrow Lake / typical Intel iGPU slot `00:02.0`. Battlemage `e2xx` is discrete.
pub fn is_intel_igpu(device_id: Option<&str>, pci_id: Option<&str>) -> bool {
    let pci = pci_id.unwrap_or("").to_ascii_lowercase();
    let dev = device_id
        .unwrap_or("")
        .trim()
        .trim_start_matches("0x")
        .to_ascii_lowercase();
    if dev.starts_with("e2") {
        return false;
    }
    if dev == "7d67" {
        return true;
    }
    pci.ends_with(":00:02.0") || pci.ends_with("00:02.0")
}

fn normalize_pci(raw: &str) -> Option<String> {
    let t = raw.trim().to_ascii_lowercase().replace("pci@", "");
    if t.is_empty() {
        return None;
    }
    let bytes = t.as_bytes();
    let mut hex: Vec<u8> = Vec::new();
    for &b in bytes {
        if b.is_ascii_hexdigit() {
            hex.push(b.to_ascii_lowercase());
        } else if b == b':' || b == b'.' {
            hex.push(b);
        }
    }
    let s = String::from_utf8_lossy(&hex);
    let parts: Vec<&str> = s.split(|c| c == ':' || c == '.').filter(|p| !p.is_empty()).collect();
    // domain:bus:dev.fn  or  bus:dev.fn
    if parts.len() == 4 {
        return Some(format!(
            "{:0>4}:{:0>2}:{:0>2}.{}",
            parts[0], parts[1], parts[2], parts[3]
        ));
    }
    if parts.len() == 3 {
        return Some(format!("0000:{:0>2}:{:0>2}.{}", parts[0], parts[1], parts[2]));
    }
    Some(t)
}

fn unique_vendor(cards: &[GpuCard]) -> Option<AccelKind> {
    let discrete: Vec<AccelKind> = cards
        .iter()
        .filter(|c| !c.igpu)
        .map(|c| c.vendor)
        .collect();
    let pool = if discrete.is_empty() {
        cards.iter().map(|c| c.vendor).collect::<Vec<_>>()
    } else {
        discrete
    };
    let Some(first) = pool.first().copied() else {
        return None;
    };
    if pool.iter().all(|v| *v == first) {
        Some(first)
    } else {
        None
    }
}

fn preferred_pci(cards: &[GpuCard], vendor: Option<AccelKind>) -> Option<String> {
    let mut cand: Vec<&GpuCard> = cards.iter().collect();
    if let Some(v) = vendor {
        cand.retain(|c| c.vendor == v);
    }
    cand.iter()
        .find(|c| !c.igpu)
        .or_else(|| cand.first())
        .and_then(|c| c.pci_id.clone())
}

/// Live DRM + lspci. Vendor from PCI id only (Intel `8086` is never NVIDIA `10de`).
pub fn detect_gpus() -> Vec<GpuCard> {
    let mut cards = detect_drm();
    if cards.is_empty() {
        cards = detect_lspci();
    }
    cards
}

fn detect_drm() -> Vec<GpuCard> {
    let Ok(entries) = fs::read_dir("/sys/class/drm") else {
        return Vec::new();
    };
    let mut cards = Vec::new();
    for ent in entries.flatten() {
        let name = ent.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("card") || name.chars().skip(4).any(|c| !c.is_ascii_digit()) {
            continue;
        }
        let dev = ent.path().join("device");
        let Some(vendor) = read_pci_hex(&dev.join("vendor")).and_then(|id| vendor_from_pci_id(&id))
        else {
            continue;
        };
        let device_id = read_pci_hex(&dev.join("device"));
        let pci_id = read_uevent_slot(&dev.join("uevent")).and_then(|s| normalize_pci(&s));
        let igpu = vendor == AccelKind::Intel && is_intel_igpu(device_id.as_deref(), pci_id.as_deref());
        cards.push(GpuCard {
            vendor,
            pci_id,
            device_id,
            igpu,
        });
    }
    cards.sort_by(|a, b| a.pci_id.cmp(&b.pci_id));
    cards
}

fn read_pci_hex(path: &Path) -> Option<String> {
    let s = fs::read_to_string(path).ok()?;
    let t = s.trim().trim_start_matches("0x").to_ascii_lowercase();
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

fn read_uevent_slot(path: &Path) -> Option<String> {
    let s = fs::read_to_string(path).ok()?;
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("PCI_SLOT_NAME=") {
            return Some(rest.trim().to_string());
        }
    }
    None
}

fn detect_lspci() -> Vec<GpuCard> {
    let Ok(out) = Command::new("lspci").arg("-nn").output() else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    parse_lspci_nn(&String::from_utf8_lossy(&out.stdout))
}

fn parse_lspci_nn(text: &str) -> Vec<GpuCard> {
    let mut cards = Vec::new();
    for line in text.lines() {
        let l = line.to_ascii_lowercase();
        if !(l.contains("vga compatible") || l.contains("3d controller") || l.contains("display controller"))
        {
            continue;
        }
        let Some((pci_raw, rest)) = line.split_once(' ') else {
            continue;
        };
        let Some(ids) = rest.rsplit_once('[').and_then(|(_, r)| r.split_once(']')) else {
            continue;
        };
        let Some((ven, dev)) = ids.0.split_once(':') else {
            continue;
        };
        let Some(vendor) = vendor_from_pci_id(ven) else {
            continue;
        };
        let pci_id = normalize_pci(pci_raw);
        let device_id = Some(dev.trim().to_ascii_lowercase());
        let igpu = vendor == AccelKind::Intel && is_intel_igpu(device_id.as_deref(), pci_id.as_deref());
        cards.push(GpuCard {
            vendor,
            pci_id,
            device_id,
            igpu,
        });
    }
    cards
}

fn mismatch(left: AccelKind, right: AccelKind, why: &str) -> String {
    format!(
        "LATE_INFER_ACCEL / toolkit / PCI disagree on your computer ({why}: {} vs {}). This is not NVIDIA unless the pin and the devices say nvidia.",
        left.as_str(),
        right.as_str()
    )
}

/// Combine pin + OneAPI/ZE vs CUDA vs HIP + PCI/DRM. Empty pin is not nvidia.
pub fn resolve_accel(
    pin_raw: &str,
    toolkit: Option<AccelKind>,
    cards: &[GpuCard],
    pci_env: Option<&str>,
) -> Result<AccelPlan, String> {
    let pin = parse_accel_token(pin_raw)?;
    let probed = unique_vendor(cards);
    if let (Some(p), Some(t)) = (pin, toolkit) {
        if p != t {
            return Err(mismatch(p, t, "pin vs OneAPI/ZE/CUDA/HIP env"));
        }
    }
    if let (Some(p), Some(h)) = (pin, probed) {
        if p != h {
            return Err(mismatch(p, h, "pin vs PCI devices"));
        }
    }
    if let (Some(t), Some(h)) = (toolkit, probed) {
        if t != h {
            return Err(mismatch(t, h, "toolkit env vs PCI devices"));
        }
    }
    if pin == Some(AccelKind::Nvidia) && probed == Some(AccelKind::Intel) {
        return Err(intel_not_nvidia_msg(true));
    }
    if toolkit == Some(AccelKind::Nvidia) && probed == Some(AccelKind::Intel) {
        return Err(intel_not_nvidia_msg(true));
    }

    let accel = pin.or(toolkit).or(probed);
    let pci_id = pci_env
        .and_then(normalize_pci)
        .or_else(|| preferred_pci(cards, accel));
    Ok(AccelPlan {
        accel,
        pin,
        toolkit,
        pci_id,
        cards: cards.to_vec(),
    })
}

pub fn resolve_from_env() -> Result<AccelPlan, String> {
    let pin = env_trim("LATE_INFER_ACCEL").unwrap_or_default();
    let toolkit = toolkit_from_env()?;
    let cards = detect_gpus();
    let pci = env_trim("LATE_INFER_PCI");
    resolve_accel(&pin, toolkit, &cards, pci.as_deref())
}

fn intel_not_nvidia_msg(compile_hint: bool) -> String {
    if compile_hint {
        "GPUs on your computer are Intel (Arc / XPU / Level Zero), not NVIDIA. Set LATE_INFER_ACCEL=intel. late-infer will not take a CUDA path or label this compile nvidia."
            .into()
    } else {
        "GPUs on your computer are Intel (Arc / XPU / Level Zero), not NVIDIA. Set LATE_INFER_ACCEL=intel. Candle cannot use that card; CPU is not the discrete Intel GPU, and late-infer will not bind CUDA 0."
            .into()
    }
}

fn intel_serve_msg() -> String {
    crate::intel::runtime_missing_msg()
}

fn amd_serve_msg() -> String {
    "late-infer cannot use the AMD GPU on your computer (this engine is CUDA/Metal/CPU, not ROCm). Refusing to start on CPU as if it were that card. This is not NVIDIA."
        .into()
}

fn intel_compile_unpinned_msg() -> String {
    "GPUs on your computer are Intel (not NVIDIA). Set LATE_INFER_ACCEL=intel so compile is not labeled nvidia and does not take a CUDA path. CPU is not that card."
        .into()
}

/// Serve: Intel uses OpenVINO GenAI when that stack is on your computer.
/// AMD HIP is not wired. NVIDIA may use CUDA. Never CPU-as-that-card.
pub fn serve_gate(plan: &AccelPlan) -> Result<(), String> {
    match plan.accel {
        Some(AccelKind::Intel) => {
            if crate::intel::intel_runtime_ok() {
                Ok(())
            } else if plan.pin.is_none() {
                Err(intel_not_nvidia_msg(false))
            } else {
                Err(intel_serve_msg())
            }
        }
        Some(AccelKind::Amd) => Err(amd_serve_msg()),
        Some(AccelKind::Nvidia) => Ok(()),
        None => Ok(()),
    }
}

/// Compile-only: vendor-neutral Hub graphs are allowed when the pin is explicit
/// (`LATE_INFER_ACCEL=intel|amd|nvidia`) and recorded as that vendor — never nvidia
/// on Intel. Empty pin + Intel-only PCI fails closed (do not compile as CPU/CUDA).
pub fn compile_gate(plan: &AccelPlan) -> Result<(), String> {
    match (plan.pin, plan.accel) {
        (None, Some(AccelKind::Intel)) => Err(intel_compile_unpinned_msg()),
        (None, Some(AccelKind::Amd)) => Err(
            "GPUs on your computer are AMD (not NVIDIA). Set LATE_INFER_ACCEL=amd. late-infer will not take a CUDA path or label this compile nvidia."
                .into(),
        ),
        (Some(AccelKind::Nvidia), Some(AccelKind::Intel)) => Err(intel_not_nvidia_msg(true)),
        (Some(AccelKind::Nvidia), _) => Ok(()),
        (Some(AccelKind::Intel), _) => Ok(()),
        (Some(AccelKind::Amd), _) => Ok(()),
        (None, Some(AccelKind::Nvidia)) => Ok(()),
        (None, None) => Ok(()),
    }
}

/// Recorded on `late-compile.json`. Pin wins; else probed vendor. Never defaults to nvidia.
pub fn compile_meta_from(plan: &AccelPlan) -> CompileAccelMeta {
    let vendor = plan.pin.or(plan.accel).map(|v| v.as_str().to_string());
    let flag = |name: &str| -> Option<bool> {
        match std::env::var(name).ok()?.trim() {
            "1" | "true" | "yes" => Some(true),
            "0" | "false" | "no" => Some(false),
            _ => None,
        }
    };
    CompileAccelMeta {
        vendor: vendor.clone(),
        accel: vendor,
        pci_id: plan.pci_id.clone(),
        idle: flag("LATE_INFER_GPU_IDLE"),
        display: flag("LATE_INFER_GPU_DISPLAY"),
    }
}

fn idle_display_flags() -> (Option<bool>, Option<bool>) {
    let flag = |name: &str| -> Option<bool> {
        match std::env::var(name).ok()?.trim() {
            "1" | "true" | "yes" => Some(true),
            "0" | "false" | "no" => Some(false),
            _ => None,
        }
    };
    (flag("LATE_INFER_GPU_IDLE"), flag("LATE_INFER_GPU_DISPLAY"))
}

pub fn compile_meta() -> CompileAccelMeta {
    match resolve_from_env() {
        Ok(plan) => compile_meta_from(&plan),
        Err(_) => {
            let pin = parse_accel_token(&env_trim("LATE_INFER_ACCEL").unwrap_or_default())
                .ok()
                .flatten();
            let probed = unique_vendor(&detect_gpus());
            // Intel PCI is never recorded as nvidia, even if a stale pin disagreed.
            let vendor = match (pin, probed) {
                (_, Some(AccelKind::Intel)) => Some(AccelKind::Intel),
                (Some(p), _) => Some(p),
                (None, p) => p,
            };
            let (idle, display) = idle_display_flags();
            CompileAccelMeta {
                vendor: vendor.map(|v| v.as_str().into()),
                accel: vendor.map(|v| v.as_str().into()),
                pci_id: env_trim("LATE_INFER_PCI").or_else(|| preferred_pci(&detect_gpus(), vendor)),
                idle,
                display,
            }
        }
    }
}

/// Orchestrator sets `LATE_INFER_ACCEL=intel|nvidia|amd` from idle-first detect
/// on your computer (not a hardcoded intel default). Intel serve needs OpenVINO
/// GenAI; otherwise refuse CPU (or CUDA 0) as if it were that card.
#[allow(dead_code)]
pub fn refuse_unwired_accel(accel: &str) -> Result<(), String> {
    match parse_accel_token(accel)? {
        Some(AccelKind::Intel) => {
            if crate::intel::intel_runtime_ok() {
                Ok(())
            } else {
                Err(intel_serve_msg())
            }
        }
        Some(AccelKind::Amd) => Err(amd_serve_msg()),
        Some(AccelKind::Nvidia) | None => Ok(()),
    }
}

/// Compile-only string pin. Explicit intel/amd is allowed (vendor-neutral graphs).
/// Call [`compile_gate`] after [`resolve_from_env`] so empty pin + Intel PCI fails closed.
#[allow(dead_code)]
pub fn refuse_unwired_compile(accel: &str) -> Result<(), String> {
    parse_accel_token(accel)?;
    Ok(())
}

/// CUDA only when the plan is NVIDIA. Intel never binds CUDA 0; OpenVINO
/// GenAI is the Intel device (label only — Candle `Device::Cpu` is unused).
pub fn pick(force_cpu: bool, plan: &AccelPlan) -> Result<(Device, String), String> {
    serve_gate(plan)?;
    if plan.accel == Some(AccelKind::Amd) {
        return Err(amd_serve_msg());
    }
    if plan.accel == Some(AccelKind::Intel) {
        if force_cpu {
            return Err(
                "LATE_INFER_ACCEL=intel on your computer needs the Intel GPU (OpenVINO GenAI). --cpu is not that card."
                    .into(),
            );
        }
        let mut label = "intel-xpu:openvino-genai".to_string();
        if let Some(pci) = plan.pci_id.as_deref() {
            label.push_str(" · ");
            label.push_str(pci);
        }
        return Ok((Device::Cpu, label));
    }
    if plan.accel == Some(AccelKind::Nvidia) {
        if force_cpu {
            return Err(
                "LATE_INFER_ACCEL=nvidia on your computer needs CUDA. --cpu is not that GPU."
                    .into(),
            );
        }
        #[cfg(feature = "cuda")]
        {
            if candle_core::utils::cuda_is_available() {
                match Device::new_cuda(0) {
                    Ok(d) => return Ok((d, "cuda:0".into())),
                    Err(e) => {
                        return Err(format!(
                            "LATE_INFER_ACCEL=nvidia but CUDA device 0 failed on your computer: {e}"
                        ));
                    }
                }
            }
        }
        return Err(
            "LATE_INFER_ACCEL=nvidia but CUDA is not available on your computer. late-infer will not start on CPU as if it were that card."
                .into(),
        );
    }
    if force_cpu {
        return Ok((Device::Cpu, "cpu (--cpu)".into()));
    }
    #[cfg(feature = "metal")]
    {
        if candle_core::utils::metal_is_available() {
            match Device::new_metal(0) {
                Ok(d) => return Ok((d, "metal:0".into())),
                Err(e) => tracing::warn!("Metal visible but Device::new_metal failed: {e}"),
            }
        }
    }
    Ok((Device::Cpu, "cpu".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn restore_intel_runtime(prev: Option<String>) {
        match prev {
            Some(v) => std::env::set_var("LATE_INFER_INTEL_RUNTIME", v),
            None => std::env::remove_var("LATE_INFER_INTEL_RUNTIME"),
        }
    }

    fn intel_b70(pci: &str, igpu: bool) -> GpuCard {
        GpuCard {
            vendor: AccelKind::Intel,
            pci_id: Some(pci.into()),
            device_id: Some(if igpu { "7d67" } else { "e223" }.into()),
            igpu,
        }
    }

    fn nvidia_card(pci: &str) -> GpuCard {
        GpuCard {
            vendor: AccelKind::Nvidia,
            pci_id: Some(pci.into()),
            device_id: Some("2684".into()),
            igpu: false,
        }
    }

    #[test]
    fn pci_8086_is_intel_not_nvidia() {
        assert_eq!(vendor_from_pci_id("8086"), Some(AccelKind::Intel));
        assert_eq!(vendor_from_pci_id("0x8086"), Some(AccelKind::Intel));
        assert_eq!(vendor_from_pci_id("10de"), Some(AccelKind::Nvidia));
        assert_eq!(vendor_from_pci_id("1002"), Some(AccelKind::Amd));
        assert_eq!(vendor_from_pci_id("1234"), None);
    }

    #[test]
    fn intel_igpu_slot_vs_battlemage() {
        assert!(is_intel_igpu(Some("7d67"), Some("0000:00:02.0")));
        assert!(!is_intel_igpu(Some("e223"), Some("0000:08:00.0")));
        assert!(!is_intel_igpu(Some("e223"), Some("0000:04:00.0")));
    }

    #[test]
    fn empty_pin_intel_only_fails_closed() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("LATE_INFER_INTEL_RUNTIME").ok();
        std::env::set_var("LATE_INFER_INTEL_RUNTIME", "0");
        let cards = vec![
            intel_b70("0000:04:00.0", false),
            intel_b70("0000:08:00.0", false),
            intel_b70("0000:00:02.0", true),
        ];
        let plan = resolve_accel("", None, &cards, None).unwrap();
        assert_eq!(plan.accel, Some(AccelKind::Intel));
        assert_eq!(plan.pin, None);
        assert!(!plan.cards.iter().any(|c| c.vendor == AccelKind::Nvidia));
        let serve = serve_gate(&plan).unwrap_err();
        assert!(serve.contains("not NVIDIA"), "{serve}");
        assert!(serve.contains("LATE_INFER_ACCEL=intel"), "{serve}");
        assert!(!serve.to_ascii_lowercase().contains("cuda:0"), "{serve}");
        let compile = compile_gate(&plan).unwrap_err();
        assert!(compile.contains("not NVIDIA"), "{compile}");
        assert!(compile.contains("LATE_INFER_ACCEL=intel"), "{compile}");
        assert!(!compile.contains("nvidia") || compile.contains("not NVIDIA") || compile.contains("not labeled nvidia") || compile.contains("label this compile nvidia"), "{compile}");
        restore_intel_runtime(prev);
    }

    #[test]
    fn pin_intel_allows_vendor_neutral_compile_not_serve_without_runtime() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("LATE_INFER_INTEL_RUNTIME").ok();
        std::env::set_var("LATE_INFER_INTEL_RUNTIME", "0");
        let cards = vec![intel_b70("0000:08:00.0", false)];
        let plan = resolve_accel("intel", Some(AccelKind::Intel), &cards, Some("0000:08:00.0")).unwrap();
        assert_eq!(plan.accel, Some(AccelKind::Intel));
        assert_eq!(plan.pci_id.as_deref(), Some("0000:08:00.0"));
        compile_gate(&plan).expect("vendor-neutral compile");
        let meta = compile_meta_from(&plan);
        assert_eq!(meta.vendor.as_deref(), Some("intel"));
        assert_eq!(meta.accel.as_deref(), Some("intel"));
        assert_ne!(meta.vendor.as_deref(), Some("nvidia"));
        assert!(serve_gate(&plan).is_err());
        assert!(pick(false, &plan).is_err());
        assert!(pick(true, &plan).is_err());
        restore_intel_runtime(prev);
    }

    #[test]
    fn pin_intel_serve_ok_when_openvino_runtime_flag() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("LATE_INFER_INTEL_RUNTIME").ok();
        std::env::set_var("LATE_INFER_INTEL_RUNTIME", "1");
        let cards = vec![intel_b70("0000:08:00.0", false)];
        let plan = resolve_accel("intel", Some(AccelKind::Intel), &cards, Some("0000:08:00.0")).unwrap();
        serve_gate(&plan).expect("OpenVINO runtime flag");
        let (dev, label) = pick(false, &plan).expect("intel pick");
        assert!(dev.is_cpu(), "{label}");
        assert!(label.contains("intel-xpu"), "{label}");
        assert!(label.contains("openvino-genai"), "{label}");
        assert!(!label.to_ascii_lowercase().contains("cuda"), "{label}");
        assert!(pick(true, &plan).is_err());
        restore_intel_runtime(prev);
    }

    #[test]
    fn pin_nvidia_on_intel_only_fails() {
        let cards = vec![intel_b70("0000:08:00.0", false)];
        let e = resolve_accel("nvidia", None, &cards, None).unwrap_err();
        assert!(e.contains("not NVIDIA") || e.contains("disagree"), "{e}");
        assert!(!e.contains("cuda:0"), "{e}");
    }

    #[test]
    fn pin_nvidia_on_nvidia_ok() {
        let cards = vec![nvidia_card("0000:01:00.0")];
        let plan = resolve_accel("nvidia", Some(AccelKind::Nvidia), &cards, None).unwrap();
        assert_eq!(plan.accel, Some(AccelKind::Nvidia));
        compile_gate(&plan).unwrap();
        serve_gate(&plan).unwrap();
    }

    #[test]
    fn pin_amd_fails_serve_allows_compile() {
        let cards = vec![GpuCard {
            vendor: AccelKind::Amd,
            pci_id: Some("0000:12:00.0".into()),
            device_id: Some("744c".into()),
            igpu: false,
        }];
        let plan = resolve_accel("amd", Some(AccelKind::Amd), &cards, None).unwrap();
        compile_gate(&plan).unwrap();
        assert!(serve_gate(&plan).unwrap_err().contains("AMD"));
    }

    #[test]
    fn unknown_accel_fails_closed() {
        let e = parse_accel_token("potato").unwrap_err();
        assert!(e.contains("intel|nvidia|amd"), "{e}");
    }

    #[test]
    fn ze_and_cuda_together_fail() {
        let e = resolve_accel("intel", Some(AccelKind::Nvidia), &[], None).unwrap_err();
        assert!(e.contains("disagree") || e.contains("not NVIDIA"), "{e}");
    }

    #[test]
    fn parse_lspci_this_fixture_is_intel_only() {
        let text = "\
00:02.0 VGA compatible controller [0300]: Intel Corporation Arrow Lake-S [Intel Graphics] [8086:7d67] (rev 06)
04:00.0 VGA compatible controller [0300]: Intel Corporation Battlemage G31 [Intel Graphics] [8086:e223]
08:00.0 VGA compatible controller [0300]: Intel Corporation Battlemage G31 [Intel Graphics] [8086:e223]
";
        let cards = parse_lspci_nn(text);
        assert_eq!(cards.len(), 3);
        assert!(cards.iter().all(|c| c.vendor == AccelKind::Intel));
        assert!(cards.iter().any(|c| c.igpu));
        assert_eq!(unique_vendor(&cards), Some(AccelKind::Intel));
    }

    #[test]
    fn live_probe_this_computer_is_not_nvidia() {
        let cards = detect_gpus();
        if cards.is_empty() {
            return;
        }
        assert!(
            !cards.iter().any(|c| c.vendor == AccelKind::Nvidia),
            "live PCI listed NVIDIA on an Intel-only QA box: {cards:?}"
        );
        if cards.iter().any(|c| c.vendor == AccelKind::Intel && !c.igpu) {
            let plan = resolve_accel("", None, &cards, None).unwrap();
            assert_eq!(plan.accel, Some(AccelKind::Intel));
            assert!(compile_gate(&plan).is_err());
            let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let prev = std::env::var("LATE_INFER_INTEL_RUNTIME").ok();
            std::env::set_var("LATE_INFER_INTEL_RUNTIME", "0");
            assert!(serve_gate(&plan).is_err());
            restore_intel_runtime(prev);
        }
    }

    #[test]
    fn intel_accel_fails_closed() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("LATE_INFER_INTEL_RUNTIME").ok();
        std::env::set_var("LATE_INFER_INTEL_RUNTIME", "0");
        assert!(refuse_unwired_accel("intel").is_err());
        assert!(refuse_unwired_accel("xpu").is_err());
        assert!(refuse_unwired_accel("amd").is_err());
        assert!(refuse_unwired_accel("cuda").is_ok());
        assert!(refuse_unwired_accel("nvidia").is_ok());
        assert!(refuse_unwired_accel("").is_ok());
        restore_intel_runtime(prev);
    }

    #[test]
    fn intel_compile_pin_is_not_nvidia() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("LATE_INFER_INTEL_RUNTIME").ok();
        std::env::set_var("LATE_INFER_INTEL_RUNTIME", "0");
        assert!(refuse_unwired_compile("intel").is_ok());
        assert!(refuse_unwired_compile("amd").is_ok());
        assert!(refuse_unwired_compile("nvidia").is_ok());
        assert!(refuse_unwired_compile("").is_ok());
        let e = refuse_unwired_accel("intel").unwrap_err();
        assert!(e.contains("Intel"), "{e}");
        assert!(e.contains("OpenVINO") || e.contains("not XPU") || e.contains("Level Zero"), "{e}");
        assert!(!e.contains("cuda:0"), "{e}");
        restore_intel_runtime(prev);
    }

    #[test]
    fn pick_never_prints_cuda_on_intel() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("LATE_INFER_INTEL_RUNTIME").ok();
        std::env::set_var("LATE_INFER_INTEL_RUNTIME", "0");
        let plan = AccelPlan {
            accel: Some(AccelKind::Intel),
            pin: Some(AccelKind::Intel),
            toolkit: Some(AccelKind::Intel),
            pci_id: Some("0000:08:00.0".into()),
            cards: vec![intel_b70("0000:08:00.0", false)],
        };
        let e = pick(false, &plan).unwrap_err();
        assert!(!e.to_ascii_lowercase().contains("cuda:0"), "{e}");
        assert!(!e.contains("NVIDIA GPU 0"), "{e}");
        std::env::set_var("LATE_INFER_INTEL_RUNTIME", "1");
        let (dev, label) = pick(false, &plan).expect("runtime flag");
        assert!(dev.is_cpu());
        assert!(label.contains("intel-xpu"), "{label}");
        assert!(!label.to_ascii_lowercase().contains("cuda"), "{label}");
        restore_intel_runtime(prev);
    }
}
