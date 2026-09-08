//! Host RAM vs idle-GPU VRAM. Fail closed instead of mmaping a 15–26B
//! snapshot into DRAM and OOM-killing your computer.

use anyhow::{bail, Result};
use std::fs;
use std::path::Path;

pub const HOST_RAM_VS_VRAM: &str =
    "this convert would use system RAM, not idle GPU VRAM";
pub const DESKTOP_HEADROOM_BYTES: u64 = 8 * 1024 * 1024 * 1024;
/// OpenVINO / optimum export keeps PyTorch + IR copies (~2.2×).
pub const CONVERT_HOST_NUM: u64 = 22;
pub const CONVERT_HOST_DEN: u64 = 10;
pub const SWAP_MIN_FREE_BYTES: u64 = 256 * 1024 * 1024;
pub const SWAP_FULL_TOTAL_BYTES: u64 = 1024 * 1024 * 1024;
pub const GPU_STAGING_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemInfo {
    pub available: u64,
    pub swap_total: u64,
    pub swap_free: u64,
}

pub fn parse_meminfo(text: &str) -> Option<MemInfo> {
    let mut available = None;
    let mut swap_total = None;
    let mut swap_free = None;
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let key = parts.next()?;
        let val = parts.next()?.parse::<u64>().ok()?;
        let bytes = val.saturating_mul(1024);
        match key {
            "MemAvailable:" => available = Some(bytes),
            "SwapTotal:" => swap_total = Some(bytes),
            "SwapFree:" => swap_free = Some(bytes),
            _ => {}
        }
    }
    Some(MemInfo {
        available: available?,
        swap_total: swap_total.unwrap_or(0),
        swap_free: swap_free.unwrap_or(0),
    })
}

pub fn read_meminfo() -> Option<MemInfo> {
    let text = fs::read_to_string("/proc/meminfo").ok()?;
    parse_meminfo(&text)
}

pub fn weight_bytes(paths: &[impl AsRef<Path>]) -> u64 {
    paths
        .iter()
        .filter_map(|p| fs::metadata(p.as_ref()).ok())
        .map(|m| m.len())
        .sum()
}

pub fn convert_peak_bytes(weights: u64) -> u64 {
    weights.saturating_mul(CONVERT_HOST_NUM) / CONVERT_HOST_DEN
}

/// Refuse convert/load that would materialize Hub weights in DRAM.
/// `on_gpu` is Level Zero / CUDA / Metal — not Candle CPU labeled as that card.
pub fn refuse_host_weight_store(
    weights: u64,
    have_ir: bool,
    on_gpu: bool,
    mem: Option<MemInfo>,
) -> Result<()> {
    let Some(mem) = mem else {
        bail!("{HOST_RAM_VS_VRAM} (could not read MemAvailable on your computer)");
    };
    if mem.swap_total >= SWAP_FULL_TOTAL_BYTES && mem.swap_free < SWAP_MIN_FREE_BYTES {
        bail!(
            "{HOST_RAM_VS_VRAM} (swap is full on your computer; Start would OOM the desktop)"
        );
    }
    if !on_gpu {
        let peak = if weights == 0 {
            convert_peak_bytes(15 * 1024 * 1024 * 1024)
        } else {
            weights
        };
        if peak.saturating_add(DESKTOP_HEADROOM_BYTES) > mem.available {
            bail!("{HOST_RAM_VS_VRAM}");
        }
        return Ok(());
    }
    if !have_ir {
        if weights == 0 {
            bail!(
                "{HOST_RAM_VS_VRAM} (OpenVINO IR is missing; convert would mmap Hub safetensors into DRAM)"
            );
        }
        let peak = convert_peak_bytes(weights);
        if peak.saturating_add(DESKTOP_HEADROOM_BYTES) > mem.available {
            bail!(
                "{HOST_RAM_VS_VRAM} (convert needs ~{} MiB host RAM plus desktop headroom; MemAvailable is {} MiB)",
                peak / (1024 * 1024),
                mem.available / (1024 * 1024)
            );
        }
        // RAM is enough for Convert — Start still must not 2× mmap. Compile/Convert writes IR first.
        return Ok(());
    }
    if GPU_STAGING_BYTES.saturating_add(DESKTOP_HEADROOM_BYTES) > mem.available {
        bail!("{HOST_RAM_VS_VRAM} (MemAvailable is too low to stage weights onto idle GPU VRAM)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem(avail_gib: u64, swap_total_gib: u64, swap_free_mib: u64) -> MemInfo {
        MemInfo {
            available: avail_gib * 1024 * 1024 * 1024,
            swap_total: swap_total_gib * 1024 * 1024 * 1024,
            swap_free: swap_free_mib * 1024 * 1024,
        }
    }

    #[test]
    fn parse_meminfo_reads_available_and_swap() {
        let text = "\
MemTotal:       65000000 kB
MemAvailable:   25165824 kB
SwapTotal:       8388608 kB
SwapFree:         102400 kB
";
        let m = parse_meminfo(text).unwrap();
        assert_eq!(m.available, 25165824 * 1024);
        assert_eq!(m.swap_total, 8388608 * 1024);
        assert_eq!(m.swap_free, 102400 * 1024);
    }

    #[test]
    fn swap_full_refuses_even_with_ir() {
        let e = refuse_host_weight_store(1_000_000_000, true, true, Some(mem(24, 8, 100)))
            .unwrap_err()
            .to_string();
        assert!(e.contains(HOST_RAM_VS_VRAM), "{e}");
        assert!(e.contains("swap is full"), "{e}");
    }

    #[test]
    fn gemma_convert_without_ir_refuses_on_24g() {
        let weights = 10 * 1024 * 1024 * 1024;
        let e = refuse_host_weight_store(weights, false, true, Some(mem(24, 8, 4096)))
            .unwrap_err()
            .to_string();
        assert!(e.contains(HOST_RAM_VS_VRAM), "{e}");
        assert!(e.contains("convert"), "{e}");
    }

    #[test]
    fn missing_ir_unknown_size_refuses() {
        let e = refuse_host_weight_store(0, false, true, Some(mem(40, 8, 4096)))
            .unwrap_err()
            .to_string();
        assert!(e.contains(HOST_RAM_VS_VRAM), "{e}");
        assert!(e.contains("IR is missing"), "{e}");
    }

    #[test]
    fn tiny_ir_on_gpu_ok_when_ram_and_swap_free() {
        refuse_host_weight_store(1_000_000_000, true, true, Some(mem(40, 8, 4096))).unwrap();
    }

    #[test]
    fn cpu_mmap_of_15gb_refuses() {
        let e = refuse_host_weight_store(15 * 1024 * 1024 * 1024, true, false, Some(mem(20, 8, 4096)))
            .unwrap_err()
            .to_string();
        assert!(e.contains(HOST_RAM_VS_VRAM), "{e}");
    }

    #[test]
    fn convert_peak_is_2_2x() {
        assert_eq!(convert_peak_bytes(10_000), 22_000);
    }
}
