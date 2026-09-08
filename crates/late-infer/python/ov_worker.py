#!/usr/bin/env python3
"""OpenVINO GenAI worker for late-infer Intel XPU (JSONL stdin/stdout).

Serve is GPU / Level Zero only. CPU is not the discrete Intel card on your
computer. Device pick is PCI + ZE_AFFINITY_MASK from the parent — not a
hardcoded SKU. Convert/load must not mmap 15–26B Hub weights into DRAM.
Logs go to stderr; stdout is one JSON object per line.
"""

from __future__ import annotations

import contextlib
import gc
import json
import os
import re
import sys
import threading
import traceback
from pathlib import Path

os.environ.setdefault("TRANSFORMERS_VERBOSITY", "error")
os.environ.setdefault("HF_HUB_DISABLE_PROGRESS_BARS", "1")
os.environ.setdefault("TOKENIZERS_PARALLELISM", "false")

HOST_RAM_VS_VRAM = "this convert would use system RAM, not idle GPU VRAM"
DESKTOP_HEADROOM_BYTES = 8 * 1024 * 1024 * 1024
CONVERT_HOST_MULT = 2.2
SWAP_MIN_FREE_BYTES = 256 * 1024 * 1024
SWAP_FULL_TOTAL_BYTES = 1024 * 1024 * 1024
GPU_STAGING_BYTES = 2 * 1024 * 1024 * 1024

_PIPE = None
_DEVICE = None
_DEVICE_NAME = None
_PCI = None


def emit(obj: dict) -> None:
    sys.stdout.write(json.dumps(obj, ensure_ascii=False) + "\n")
    sys.stdout.flush()


def fail(message: str, **extra) -> None:
    payload = {"ok": False, "error": message}
    payload.update(extra)
    emit(payload)


def parse_meminfo(text: str) -> dict[str, int] | None:
    available = None
    swap_total = 0
    swap_free = 0
    for line in text.splitlines():
        parts = line.split()
        if len(parts) < 2:
            continue
        try:
            kib = int(parts[1])
        except ValueError:
            continue
        key = parts[0]
        if key == "MemAvailable:":
            available = kib * 1024
        elif key == "SwapTotal:":
            swap_total = kib * 1024
        elif key == "SwapFree:":
            swap_free = kib * 1024
    if available is None:
        return None
    return {"available": available, "swap_total": swap_total, "swap_free": swap_free}


def read_meminfo() -> dict[str, int] | None:
    try:
        return parse_meminfo(Path("/proc/meminfo").read_text())
    except OSError:
        return None


def current_rss_bytes() -> int:
    try:
        statm = Path("/proc/self/statm").read_text().split()
        pages = int(statm[1])
        return pages * os.sysconf("SC_PAGE_SIZE")
    except (OSError, IndexError, ValueError):
        return 0


def safetensor_bytes(model_dir: Path) -> int:
    if not model_dir.is_dir():
        return 0
    total = 0
    for p in model_dir.glob("*.safetensors"):
        try:
            total += p.stat().st_size
        except OSError:
            continue
    return total


def convert_peak_bytes(weights: int) -> int:
    return int(weights * CONVERT_HOST_MULT)


def refuse_host_weight_store(weights: int, have_ir: bool, mem: dict[str, int] | None) -> None:
    if mem is None:
        raise RuntimeError(f"{HOST_RAM_VS_VRAM} (could not read MemAvailable on your computer)")
    if mem["swap_total"] >= SWAP_FULL_TOTAL_BYTES and mem["swap_free"] < SWAP_MIN_FREE_BYTES:
        raise RuntimeError(
            f"{HOST_RAM_VS_VRAM} (swap is full on your computer; Start would OOM the desktop)"
        )
    avail = mem["available"]
    if not have_ir:
        if weights <= 0:
            raise RuntimeError(
                f"{HOST_RAM_VS_VRAM} (OpenVINO IR is missing; convert would mmap Hub safetensors into DRAM)"
            )
        peak = convert_peak_bytes(weights)
        if peak + DESKTOP_HEADROOM_BYTES > avail:
            raise RuntimeError(
                f"{HOST_RAM_VS_VRAM} (convert needs ~{peak // (1024 * 1024)} MiB host RAM "
                f"plus desktop headroom; MemAvailable is {avail // (1024 * 1024)} MiB)"
            )
        return
    if GPU_STAGING_BYTES + DESKTOP_HEADROOM_BYTES > avail:
        raise RuntimeError(
            f"{HOST_RAM_VS_VRAM} (MemAvailable is too low to stage weights onto idle GPU VRAM)"
        )


def install_as_cap(extra_bytes: int) -> tuple[int, int] | None:
    """Cap process VAS during convert so a 2× mmap cannot OOM the desktop."""
    if os.name != "posix" or extra_bytes <= 0:
        return None
    try:
        import resource

        soft, hard = resource.getrlimit(resource.RLIMIT_AS)
        rss = current_rss_bytes()
        cap = rss + extra_bytes
        if hard != resource.RLIM_INFINITY:
            cap = min(cap, hard)
        resource.setrlimit(resource.RLIMIT_AS, (cap, hard))
        return soft, hard
    except (ValueError, OSError, ImportError):
        return None


def restore_as_cap(prev: tuple[int, int] | None) -> None:
    if prev is None:
        return
    try:
        import resource

        resource.setrlimit(resource.RLIMIT_AS, prev)
    except (ValueError, OSError, ImportError):
        pass


def watch_host_rss(max_rss: int, stop: threading.Event) -> None:
    while not stop.wait(0.4):
        rss = current_rss_bytes()
        if max_rss > 0 and rss > max_rss:
            print(
                f"late-infer: aborting — host RSS {rss // (1024 * 1024)} MiB would OOM your computer "
                f"({HOST_RAM_VS_VRAM})",
                file=sys.stderr,
            )
            os._exit(2)


def parse_pci(raw: str | None) -> tuple[int, int, int, int] | None:
    if not raw or not str(raw).strip():
        return None
    t = str(raw).strip().lower().replace("pci@", "")
    parts = [p for p in re.split(r"[:.]", t) if p]
    try:
        if len(parts) == 4:
            return tuple(int(p, 16) for p in parts)  # type: ignore[return-value]
        if len(parts) == 3:
            return (0, int(parts[0], 16), int(parts[1], 16), int(parts[2], 16))
    except ValueError:
        return None
    return None


def ov_pci(core, device: str) -> tuple[int, int, int, int] | None:
    try:
        info = core.get_property(device, "DEVICE_PCI_INFO")
    except Exception:
        return None
    if isinstance(info, dict):
        def n(key: str) -> int:
            v = info.get(key, 0)
            if isinstance(v, str):
                return int(v, 0)
            return int(v)

        try:
            return (n("domain"), n("bus"), n("device"), n("function"))
        except Exception:
            pass
    s = str(info)
    m = re.search(
        r"domain:\s*(0x)?([0-9a-f]+).*"
        r"bus:\s*(0x)?([0-9a-f]+).*"
        r"device:\s*(0x)?([0-9a-f]+).*"
        r"function:\s*(0x)?([0-9a-f]+)",
        s,
        re.I | re.S,
    )
    if not m:
        return None
    nums = [m.group(2), m.group(4), m.group(6), m.group(8)]
    try:
        return tuple(int(nums[i], 16 if m.group(1 + i * 2) else 10) for i in range(4))  # type: ignore[return-value]
    except ValueError:
        return tuple(int(x, 16) for x in nums)  # type: ignore[return-value]


def format_pci(pci: tuple[int, int, int, int] | None) -> str | None:
    if not pci:
        return None
    return f"{pci[0]:04x}:{pci[1]:02x}:{pci[2]:02x}.{pci[3]:x}"


def device_type_name(core, device: str) -> str:
    try:
        t = core.get_property(device, "DEVICE_TYPE")
        return str(t)
    except Exception:
        return ""


def gpu_total_mem_bytes(core, device: str) -> int:
    keys = ["GPU_DEVICE_TOTAL_MEM_SIZE"]
    try:
        import openvino as ov

        keys.append(ov.properties.intel_gpu.device_total_mem_size())
    except Exception:
        pass
    for key in keys:
        try:
            return int(core.get_property(device, key))
        except Exception:
            continue
    return 0


def usm_stats(core, device: str) -> dict[str, int]:
    raw = None
    try:
        raw = core.get_property(device, "GPU_MEMORY_STATISTICS")
    except Exception:
        try:
            import openvino as ov

            raw = core.get_property(device, ov.properties.intel_gpu.memory_statistics())
        except Exception:
            return {}
    if raw is None:
        return {}
    out: dict[str, int] = {}
    if isinstance(raw, dict):
        items = raw.items()
    else:
        items = []
        s = str(raw)
        for m in re.finditer(r"([A-Za-z0-9_]+)\s*[:=]\s*(\d+)", s):
            items.append((m.group(1), m.group(2)))
    for k, v in items:
        try:
            out[str(k).lower()] = int(v)
        except (TypeError, ValueError):
            continue
    return out


def pick_gpu(core, want_pci: tuple[int, int, int, int] | None):
    gpus = [d for d in core.available_devices if str(d).upper().startswith("GPU")]
    if not gpus:
        raise RuntimeError(
            "OpenVINO sees no GPU on your computer (Level Zero / Intel GPU plugin). "
            "late-infer will not start on CPU as if it were that card."
        )
    chosen = None
    if want_pci:
        for d in gpus:
            got = ov_pci(core, d)
            if got == want_pci:
                chosen = d
                break
        if chosen is None:
            seen = ", ".join(
                f"{d}={format_pci(ov_pci(core, d))}" for d in gpus
            )
            raise RuntimeError(
                f"OpenVINO GPU list has no PCI {format_pci(want_pci)} on your computer ({seen}). "
                "late-infer will not pick a different card or CPU."
            )
    else:
        chosen = gpus[0] if len(gpus) == 1 else next((d for d in gpus if d == "GPU"), gpus[0])

    dtype = device_type_name(core, chosen)
    idle = os.environ.get("LATE_INFER_GPU_IDLE", "").strip() in ("1", "true", "yes")
    if idle and re.search(r"INTEGRATED", dtype, re.I):
        raise RuntimeError(
            "Idle GPU pin resolved to an integrated Intel GPU. "
            "late-infer will not run on the iGPU (or CPU) as if it were the idle discrete card."
        )
    if re.search(r"CPU", dtype, re.I) and not re.search(r"GPU", str(chosen), re.I):
        raise RuntimeError(
            "OpenVINO picked CPU. late-infer will not start on CPU as if it were the Intel GPU."
        )
    if re.search(r"INTEGRATED", dtype, re.I):
        raise RuntimeError(
            f"{HOST_RAM_VS_VRAM} (OpenVINO DEVICE_TYPE={dtype} is the iGPU / unified host RAM, not idle discrete VRAM)"
        )
    name = ""
    try:
        name = str(core.get_property(chosen, "FULL_DEVICE_NAME"))
    except Exception:
        name = chosen
    return chosen, name, ov_pci(core, chosen)


def prove_discrete_vram(core, device: str, name: str) -> int:
    dtype = device_type_name(core, device)
    if re.search(r"INTEGRATED", dtype, re.I):
        raise RuntimeError(
            f"{HOST_RAM_VS_VRAM} (OpenVINO {device} {name} is INTEGRATED, not idle discrete VRAM)"
        )
    mem = gpu_total_mem_bytes(core, device)
    if 0 < mem < 4 * 1024 * 1024 * 1024:
        raise RuntimeError(
            f"{HOST_RAM_VS_VRAM} (OpenVINO {device} reports {mem // (1024 * 1024)} MiB — "
            "that is not the idle 32 GB B70 VRAM)"
        )
    return mem


def tiny_gpu_infer(core, device: str) -> None:
    import numpy as np
    import openvino as ov

    param = ov.opset13.parameter([2, 2], ov.Type.f32)
    const = ov.opset13.constant(np.ones((2, 2), dtype=np.float32))
    add = ov.opset13.add(param, const)
    model = ov.Model([add], [param], "late_infer_gpu_probe")
    compiled = core.compile_model(model, device)
    compiled.create_infer_request().infer({0: np.zeros((2, 2), dtype=np.float32)})


def gpu_pipeline_config(ov_dir: Path) -> dict:
    cfg: dict = {
        "CACHE_DIR": str(ov_dir / "ov_cache"),
        "PERFORMANCE_HINT": "LATENCY",
        "INFERENCE_PRECISION_HINT": "f16",
    }
    try:
        import openvino as ov

        cfg[ov.properties.intel_gpu.hint.enable_large_allocations()] = True
        cfg[ov.properties.enable_mmap()] = False
    except Exception:
        cfg["GPU_ENABLE_LARGE_ALLOCATIONS"] = "YES"
        cfg["ENABLE_MMAP"] = "NO"
    return cfg


def assert_weights_on_vram(core, device: str) -> dict[str, int]:
    stats = usm_stats(core, device)
    usm_device = 0
    usm_host = 0
    for key, val in stats.items():
        if "usm_device" in key or key == "device":
            usm_device = max(usm_device, val)
        if "usm_host" in key or key in ("host", "usm_host"):
            usm_host = max(usm_host, val)
    if usm_device <= 0 and usm_host > 256 * 1024 * 1024:
        raise RuntimeError(
            f"{HOST_RAM_VS_VRAM} (OpenVINO usm_device is 0; weights stayed in host USM)"
        )
    return stats


def has_ir(directory: Path) -> bool:
    if not directory.is_dir():
        return False
    if (directory / "openvino_model.xml").is_file():
        return True
    return any(p.suffix == ".xml" for p in directory.glob("*.xml") if "tokenizer" not in p.name)


def export_ov_tokenizers(src: Path, dest: Path) -> None:
    """LLMPipeline.encode needs openvino_tokenizer.xml, not only Hugging Face tokenizer.json."""
    from transformers import AutoTokenizer
    from openvino_tokenizers import convert_tokenizer
    import openvino as ov

    hf_tok = AutoTokenizer.from_pretrained(str(src), trust_remote_code=True)
    ov_tok, ov_detok = convert_tokenizer(hf_tok, with_detokenizer=True)
    ov.save_model(ov_tok, str(dest / "openvino_tokenizer.xml"))
    ov.save_model(ov_detok, str(dest / "openvino_detokenizer.xml"))
    hf_tok.save_pretrained(str(dest))
    del hf_tok, ov_tok, ov_detok
    gc.collect()


def convert_hf_to_ov(src: Path, dest: Path) -> None:
    """Stream Hub weights to OpenVINO IR on disk. Prefer export-to-disk; do not 2× mmap into DRAM."""
    dest.mkdir(parents=True, exist_ok=True)
    err = None
    with contextlib.redirect_stdout(sys.stderr):
        try:
            from optimum.exporters.openvino import main_export

            main_export(
                model_name_or_path=str(src),
                output=str(dest),
                task="text-generation-with-past",
                trust_remote_code=True,
                weight_format="fp16",
            )
            if not (dest / "openvino_tokenizer.xml").is_file():
                export_ov_tokenizers(src, dest)
            gc.collect()
            return
        except MemoryError as e:
            raise RuntimeError(f"{HOST_RAM_VS_VRAM} ({e})") from e
        except Exception as e:
            err = e
            gc.collect()
        try:
            from optimum.intel import OVModelForCausalLM

            model = OVModelForCausalLM.from_pretrained(
                str(src),
                export=True,
                compile=False,
                trust_remote_code=True,
                ov_config={"DYNAMIC_QUANTIZATION_GROUP_SIZE": "32"},
            )
            model.save_pretrained(str(dest))
            del model
            gc.collect()
            export_ov_tokenizers(src, dest)
            return
        except MemoryError as e:
            raise RuntimeError(f"{HOST_RAM_VS_VRAM} ({e})") from e
        except Exception as e:
            err = e
            gc.collect()
        try:
            from optimum.intel import OVModelForVisualCausalLM

            model = OVModelForVisualCausalLM.from_pretrained(
                str(src),
                export=True,
                compile=False,
                trust_remote_code=True,
            )
            model.save_pretrained(str(dest))
            del model
            gc.collect()
            export_ov_tokenizers(src, dest)
            return
        except Exception as e2:
            gc.collect()
            raise RuntimeError(
                "OpenVINO could not convert this Hugging Face snapshot for the Intel GPU on your computer. "
                "A CUDA compiled blob is not that card, and CPU is not that card. "
                f"CausalLM: {err}; visual: {e2}"
            ) from e2


IR_MISSING_START = (
    "OpenVINO IR is missing. Start refused to protect your computer. "
    "Download/compile must produce IR, or Convert only when MemAvailable is safe."
)
# Shared-library mmap slop after preload (RLIMIT_AS must not block .so maps).
CONVERT_VAS_SLOP_BYTES = 1024 * 1024 * 1024


def preload_convert_deps() -> None:
    """Import exporters before RLIMIT_AS so regex/transformers .so files can mmap."""
    with contextlib.redirect_stdout(sys.stderr):
        try:
            from optimum.exporters.openvino import main_export  # noqa: F401
        except Exception:
            pass
        try:
            import transformers  # noqa: F401
        except Exception:
            pass
        try:
            import openvino_tokenizers  # noqa: F401
        except Exception:
            pass
    gc.collect()


def export_ir_to_disk(model_dir: Path, ov_dir: Path) -> None:
    """Compile-only / Convert: write IR under compiled/<slug>/openvino/. Never binds :8010."""
    if not model_dir.is_dir():
        raise RuntimeError(f"Hub snapshot is missing on your computer ({model_dir})")
    if not ov_dir.parts:
        raise RuntimeError("ov_dir missing")
    if has_ir(ov_dir):
        if not (ov_dir / "openvino_tokenizer.xml").is_file():
            export_ov_tokenizers(model_dir, ov_dir)
        return
    weights = safetensor_bytes(model_dir)
    mem = read_meminfo()
    refuse_host_weight_store(weights, False, mem)
    print(
        "late-infer: exporting OpenVINO IR on your computer "
        "(capped — will not 2× mmap Hub safetensors into DRAM)…",
        file=sys.stderr,
    )
    preload_convert_deps()
    stop = threading.Event()
    max_rss = 0
    if mem:
        max_rss = current_rss_bytes() + max(0, mem["available"] - DESKTOP_HEADROOM_BYTES)
    watcher = threading.Thread(target=watch_host_rss, args=(max_rss, stop), daemon=True)
    watcher.start()
    # Do not RLIMIT_AS here: Optimum/transformers mmap .so files into VAS (not DRAM
    # weight copies). The RSS watcher + refuse_host_weight_store (2.2× + 8 GiB) is
    # the OOM guard. A tight AS cap made Qwen 0.5B fail before IR hit disk.
    try:
        convert_hf_to_ov(model_dir, ov_dir)
    except MemoryError as e:
        raise RuntimeError(f"{HOST_RAM_VS_VRAM} ({e})") from e
    finally:
        stop.set()
        gc.collect()
    if not has_ir(ov_dir):
        raise RuntimeError(
            "OpenVINO IR is missing after convert. late-infer will not load Candle CPU as that Intel GPU."
        )
    if not (ov_dir / "openvino_tokenizer.xml").is_file():
        export_ov_tokenizers(model_dir, ov_dir)


def _want_pci(msg: dict):
    return parse_pci(os.environ.get("LATE_INFER_PCI") or msg.get("pci"))


def op_probe(msg: dict) -> None:
    import openvino as ov

    core = ov.Core()
    devices = [str(d) for d in core.available_devices]
    if not any(d.upper().startswith("GPU") for d in devices):
        raise RuntimeError(
            "OpenVINO sees no GPU on your computer. late-infer will not start on CPU as if it were that card."
        )
    device, name, pci = pick_gpu(core, _want_pci(msg))
    ov_device = "GPU" if device.startswith("GPU") else device
    gpu_mem = prove_discrete_vram(core, device, name)
    tiny_gpu_infer(core, ov_device)
    mem = read_meminfo() or {}
    emit(
        {
            "ok": True,
            "op": "probe",
            "kind": "openvino-genai",
            "device": ov_device,
            "device_name": name,
            "pci": format_pci(pci),
            "device_type": device_type_name(core, device),
            "gpu_mem_bytes": gpu_mem,
            "mem_available": mem.get("available"),
            "swap_free": mem.get("swap_free"),
            "available_devices": devices,
        }
    )


def op_convert(msg: dict) -> None:
    model_dir = Path(str(msg.get("model_dir") or "")).expanduser()
    ov_dir = Path(str(msg.get("ov_dir") or "")).expanduser()
    export_ir_to_disk(model_dir, ov_dir)
    emit(
        {
            "ok": True,
            "op": "convert",
            "kind": "openvino-genai",
            "ir": True,
        }
    )


def op_load(msg: dict) -> None:
    global _PIPE, _DEVICE, _DEVICE_NAME, _PCI
    import openvino as ov
    import openvino_genai as ov_genai

    model_dir = Path(str(msg.get("model_dir") or "")).expanduser()
    ov_dir = Path(str(msg.get("ov_dir") or "")).expanduser()
    if not model_dir.is_dir():
        raise RuntimeError(f"Hub snapshot is missing on your computer ({model_dir})")
    if not ov_dir.parts:
        raise RuntimeError("ov_dir missing")

    ir_ready = has_ir(ov_dir)
    weights = safetensor_bytes(model_dir)
    mem = read_meminfo()
    if not ir_ready:
        raise RuntimeError(
            f"{IR_MISSING_START} {HOST_RAM_VS_VRAM} "
            "(OpenVINO IR is missing; convert would mmap Hub safetensors into DRAM)"
        )
    refuse_host_weight_store(weights, True, mem)

    core = ov.Core()
    device, name, pci = pick_gpu(core, _want_pci(msg))
    ov_device = "GPU" if device.startswith("GPU") else device
    prove_discrete_vram(core, device, name)
    tiny_gpu_infer(core, ov_device)

    stop = threading.Event()
    max_rss = 0
    if mem:
        max_rss = current_rss_bytes() + max(0, mem["available"] - DESKTOP_HEADROOM_BYTES)
    watcher = threading.Thread(target=watch_host_rss, args=(max_rss, stop), daemon=True)
    watcher.start()
    try:
        if not (ov_dir / "openvino_tokenizer.xml").is_file():
            print("late-infer: adding OpenVINO tokenizer on your computer…", file=sys.stderr)
            with contextlib.redirect_stdout(sys.stderr):
                export_ov_tokenizers(model_dir, ov_dir)
        if not has_ir(ov_dir):
            raise RuntimeError(
                "OpenVINO IR is missing after convert. late-infer will not load Candle CPU as that Intel GPU."
            )
        if not (ov_dir / "openvino_tokenizer.xml").is_file():
            raise RuntimeError(
                "OpenVINO tokenizer XML is missing. late-infer will not generate on CPU as if it were the Intel GPU."
            )

        print(f"late-infer: OpenVINO GenAI loading on {ov_device} ({name})", file=sys.stderr)
        cfg = gpu_pipeline_config(ov_dir)
        with contextlib.redirect_stdout(sys.stderr):
            try:
                pipe = ov_genai.LLMPipeline(str(ov_dir), ov_device, **cfg)
            except TypeError:
                pipe = ov_genai.LLMPipeline(str(ov_dir), ov_device)
        stats = assert_weights_on_vram(core, ov_device)
    finally:
        stop.set()

    _PIPE = pipe
    _DEVICE = ov_device
    _DEVICE_NAME = name
    _PCI = format_pci(pci)
    emit(
        {
            "ok": True,
            "op": "load",
            "kind": "openvino-genai",
            "device": ov_device,
            "device_name": name,
            "pci": _PCI,
            "usm_device": stats.get("usm_device", stats.get("device", 0)),
        }
    )


def op_generate(msg: dict) -> None:
    if _PIPE is None:
        raise RuntimeError("OpenVINO worker has no loaded model")
    prompt = str(msg.get("prompt") or "")
    if not prompt:
        raise RuntimeError("empty prompt")
    max_tokens = int(msg.get("max_tokens") or 512)
    max_tokens = max(1, min(max_tokens, 2048))
    temperature = msg.get("temperature")
    top_p = msg.get("top_p")
    import openvino_genai as ov_genai

    cfg = ov_genai.GenerationConfig()
    cfg.max_new_tokens = max_tokens
    if temperature is None or (isinstance(temperature, (int, float)) and float(temperature) <= 0):
        cfg.temperature = 0.0
        try:
            cfg.do_sample = False
        except Exception:
            pass
    else:
        cfg.temperature = float(temperature)
        try:
            cfg.do_sample = True
        except Exception:
            pass
    if isinstance(top_p, (int, float)) and 0 < float(top_p) < 1:
        cfg.top_p = float(top_p)
    with contextlib.redirect_stdout(sys.stderr):
        text = _PIPE.generate(prompt, cfg)
    if not isinstance(text, str):
        text = str(text)
    emit(
        {
            "ok": True,
            "op": "generate",
            "text": text,
            "finish_reason": "stop",
            "device": _DEVICE,
        }
    )


def main() -> int:
    for raw in sys.stdin:
        line = raw.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except json.JSONDecodeError as e:
            fail(f"worker json: {e}")
            continue
        op = str(msg.get("op") or "")
        try:
            if op == "quit":
                emit({"ok": True, "op": "quit"})
                return 0
            if op == "probe":
                op_probe(msg)
            elif op == "convert":
                op_convert(msg)
            elif op == "load":
                op_load(msg)
            elif op == "generate":
                op_generate(msg)
            else:
                fail(f"unknown op {op!r}")
        except Exception as e:
            traceback.print_exc(file=sys.stderr)
            fail(str(e))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        raise SystemExit(1)
