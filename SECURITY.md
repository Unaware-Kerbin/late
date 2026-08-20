# Security

Late is a local desktop control plane. Please report vulnerabilities privately rather than opening a public issue with exploit detail.

**Contact:** open a GitHub security advisory on [Unaware-Kerbin/late](https://github.com/Unaware-Kerbin/late), or email the maintainer listed on that repository.

**Please include:** Late version or commit, OS, whether daemon/sidecar were local, and a short reproduction. Do not send production secrets.

**Scope:** `late-daemon`, agent sidecar, desktop shell, path jail, token/CORS, cloud-chat opt-in, Hugging Face GGUF download, vendor permit lists.

**Out of scope:** attacking devices you do not own; social engineering the operator into Approve; issues that require already reading `sidecar.token` as the same OS user (that is the trust boundary).

The ISMS for this project lives in [docs/isms/](docs/isms/README.md). Late is **not** SOC 2 or ISO 27001 certified.
