# Late inference — Intel Arc Pro B70 (2× Battlemage)

Docker is for **vLLM** (`compose.yml`) and optional **Edgeshark** (`compose.edgeshark.yml`). SSH, serial, SFTP, and the UI stay on the host. Packet files are parsed on the host; Edgeshark is the Siemens Ghostwire + Packetflix stack for live capture of *local* container traffic into desktop Wireshark. Remote SSH boxes use `tcpdump` over a second SSH exec, not Edgeshark.

## Hardware mapping

Arrow Lake’s iGPU is typically PCI `00:02.0` and must **not** be in the affinity mask. The two Arc Pro B70 cards (`8086:e223`) are the discrete devices.

```bash
./docker/detect-gpus.sh docker/.env
# writes host ZE_AFFINITY_MASK plus VIDEO_GID and RENDER_GID for *this* host
# typical dual-B70 with iGPU present: host ZE_AFFINITY_MASK=0,2 (iGPU is index 1)
# compact list for the container (not injected): DOCKER_ZE_AFFINITY_MASK=0,1
# if Level Zero sees only discrete GPUs: ZE_AFFINITY_MASK=0,1
```

Host `ZE_AFFINITY_MASK` (e.g. `0,2`) must **not** be passed into the vLLM container. This image only lists the two B70s as devices `0,1`.

## Image pin

| Image | When |
|---|---|
| `intel/llm-scaler-vllm:0.21.0-b3` | Default (current Hub tag, Aug 2026) |
| `intel/llm-scaler-vllm:0.14.0-b8.2.1` (also `1.4`) | Fallback — first release with official Arc Pro B70 support |
| `intel/llm-scaler-vllm:<same tag>` | Alternate repository name on some mirrors |

```bash
export VLLM_IMAGE=intel/llm-scaler-vllm:0.14.0-b8.2.1   # if 0.21.0-b3 404s
```

## Start

```bash
cd docker
./detect-gpus.sh .env
docker compose up
# API: http://127.0.0.1:8000/v1
```

The `intel/llm-scaler-vllm:0.21.0-b3` image already runs `vllm serve` (venv is on `PATH`). Do **not** source `/opt/intel/oneapi/setvars.sh` — that file is not in this image and will crash-loop the container.

Compose uses `privileged: true`, `/dev/dri` plus `/dev/dri/by-path`, `ipc: host`, `shm_size: 32gb`, and `VLLM_WORKER_MULTIPROC_METHOD=spawn`. Tensor-parallel oneCCL workers need the extra by-path mount and privileged mode (intel/ipex-llm#12990: `opendir` / `ze_fd_manager` otherwise fails). Intel-style IPC env is set in compose (`CCL_ZE_IPC_EXCHANGE=sockets`, `CCL_ZE_CACHE_OPEN_IPC_HANDLES=0`, `CCL_ATL_TRANSPORT=ofi`). Do **not** set `FI_PROVIDER=shm` — that failed ATL init in the sandbox. Restart is `on-failure:3` so a GPU init crash does not loop forever.

SYCL persistent disk cache is **off** (`SYCL_CACHE_PERSISTENT=0`, `SYCL_CACHE_DIR=/tmp/empty-sycl-cache`). With `=1`, TP=2 died on the first xccl allreduce: `sycl::detail::PersistentDeviceCodeCache::getItemFromDisc` → `arc_ll256_allreduce`. Do **not** raise `CCL_SYCL_ALLREDUCE_SIMPLE_THRESHOLD` / `CCL_SYCL_ALLGATHERV_SIMPLE_THRESHOLD` to 1GiB to “avoid ll256” — Intel’s note on [vllm#50545](https://github.com/vllm-project/vllm/issues/50545) is that those knobs keep the Low Latency (`arc_ll256`) path, which is the crashing symbol.

Host `video`/`render` **GIDs are detected at launch** (`getent group`) and written to `docker/.env` (gitignored). Do not copy another machine’s GID numbers.

## Tensor parallel

Default **TP=1** on one B70 (`Qwen/Qwen3-8B`). Do **not** pass the host `ZE_AFFINITY_MASK` into the container — this image only lists the discrete cards as devices `0,1`. Compose sets `ZE_FLAT_DEVICE_HIERARCHY=FLAT` instead (no `ZE_AFFINITY_MASK` on the service).

Battlemage has known XPU / BCS-reset instability under TP=2 on some kernels. Fallback:

```bash
TP=1 docker compose up
```

That uses a single XPU inside the container (index `0`). Do not export the host mask (`0,2`) on that command. For two independent TP=1 servers, run a second compose project with a different host port and pin `ZE_AFFINITY_MASK=0` or `1` **inside** that container only.

## Models

Late probes this machine’s discrete GPUs (nvidia-smi, sysfs, PCI BAR size) and recommends Hugging Face ids whose weights fit. iGPUs at PCI `00:02.0` are skipped. Intel multi-GPU tensor parallel is opt-in (`LATE_VLLM_ALLOW_TP=1`) because XPU IPC is still unreliable.

The Agent pane lists those recommendations with Download. A 35B-class checkpoint is offered only if the detected VRAM can hold it.

```bash
# same download Late uses (cache is often root-owned, so run via the image)
docker run --rm --entrypoint /opt/venv/bin/hf \
  -v "$HOME/.cache/huggingface:/root/.cache/huggingface" \
  intel/llm-scaler-vllm:0.21.0-b3 \
  download Qwen/Qwen3-8B
```

Hide any `:cloud` endpoint IDs. Context overflow must be a hard refuse (the sidecar does not truncate).

Set `MODEL_PATH` to a **complete** Hub snapshot (resolved via `$HF_HOME`) or a path under `/models`. The served OpenAI name is always `local`.

## Smoke

```bash
curl -sS http://127.0.0.1:8000/v1/models
curl -sS http://127.0.0.1:8000/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{"model":"local","messages":[{"role":"user","content":"Reply with the word pong."}],"max_tokens":16}'
```

Tool-calling smoke (Hermes parser is enabled in compose):

```bash
curl -sS http://127.0.0.1:8000/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{"model":"local","messages":[{"role":"user","content":"Call ping with host 127.0.0.1"}],"tools":[{"type":"function","function":{"name":"ping","parameters":{"type":"object","properties":{"host":{"type":"string"}}}}}]}'
```
