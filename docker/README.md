# Local inference

Late does not bundle a GPU runtime. Point the Agent pane at **vLLM**, **llama.cpp**, or **Ollama** on loopback. NVIDIA, AMD, and Intel GPUs all work if the server you run uses them.

SSH, serial, SCP, and the UI stay on the host. Docker in this folder is optional.

## Pick a stack

Point the Agent pane at the server. Late never installs CUDA, ROCm, oneAPI, Ollama, llama.cpp, or model weights for you.

| Backend in Late | Typical URL | GPU |
|---|---|---|
| **Ollama** | `http://127.0.0.1:11434/v1` | NVIDIA, AMD, Apple, Intel (whatever Ollama supports on that OS) |
| **llama.cpp** | `http://127.0.0.1:8080/v1` | CUDA, Metal, Vulkan, ROCm, CPU — via `llama-server` |
| **local vLLM** | `http://127.0.0.1:8000/v1` | vLLM (CUDA / ROCm / Intel XPU) |

### Ollama

Install from [ollama.com](https://ollama.com), start it, then `ollama pull <model>`. In the Agent pane choose **Ollama**.

### llama.cpp

Build or install [llama.cpp](https://github.com/ggml-org/llama.cpp) with the backend you want (CUDA, Vulkan, Metal, …), then:

```bash
llama-server -m /path/to/model.gguf --port 8080 --host 127.0.0.1
```

In Late choose **llama.cpp**. Default API: `http://127.0.0.1:8080/v1` (Settings if you used another port).

### vLLM

Run vLLM on the GPU you have, listening on `127.0.0.1`. In Late choose **local vLLM** and set the base URL / model in Settings if they are not `http://127.0.0.1:8000/v1` and `local`.

Smoke:

```bash
curl -sS http://127.0.0.1:8000/v1/models
curl -sS http://127.0.0.1:8000/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{"model":"local","messages":[{"role":"user","content":"Reply with the word pong."}],"max_tokens":16}'
```

Hide any `:cloud` endpoint IDs. Context overflow must be a hard refuse (the sidecar does not truncate).

## Optional: Docker Compose in this repo (Intel XPU)

`compose.yml` is an example **Intel Arc / XPU** vLLM image (`intel/llm-scaler-vllm`). It is not required for NVIDIA or AMD — use Ollama, llama.cpp, or a CUDA/ROCm vLLM install instead.

Skip integrated GPUs (on Intel platforms that is often PCI `00:02.0`). `detect-gpus.sh` writes host Level Zero indices and `video`/`render` GIDs for *this* machine into `docker/.env` (gitignored). Do not copy another machine’s GID numbers. Do **not** pass the host `ZE_AFFINITY_MASK` into the container; the image enumerates discrete XPUs as `0,1`.

```bash
cd docker
./detect-gpus.sh .env
docker compose up
# API: http://127.0.0.1:8000/v1
```

| Image | When |
|---|---|
| `intel/llm-scaler-vllm:0.21.0-b3` | Default (current Hub tag, Aug 2026) |
| `intel/llm-scaler-vllm:0.14.0-b8.2.1` | Fallback if that tag 404s |

```bash
export VLLM_IMAGE=intel/llm-scaler-vllm:0.14.0-b8.2.1
```

The image already runs `vllm serve`. Do **not** source `/opt/intel/oneapi/setvars.sh`. Compose uses `privileged: true`, `/dev/dri` plus `/dev/dri/by-path`, `ipc: host`, and `shm_size: 32gb` for oneCCL workers. SYCL persistent disk cache is off (`SYCL_CACHE_PERSISTENT=0`). Restart is `on-failure:3`. Default tensor parallel is **TP=1**. Multi-GPU Intel TP is opt-in (`LATE_VLLM_ALLOW_TP=1` / `TP=2`) because XPU IPC is still unreliable on some kernels.

Late probes discrete GPUs (nvidia-smi, sysfs, PCI BAR size) and can recommend Hugging Face ids that fit. The Agent pane lists those with Download when this compose stack is in use. The served model name is `local`. llama.cpp GGUF download is separate (host Hugging Face fetch into Late’s data dir, not this image).
