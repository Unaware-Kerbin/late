# Change management

- Source of truth: git on GitHub (`Unaware-Kerbin/late`).
- CI on push and pull request: `cargo test -p late-core`, daemon build, isolation-check, dependency advisory scans.
- Releases: `v*` tags or workflow_dispatch; installers currently **unsigned**.
- Product security changes that affect loopback, tokens, or cloud egress should update this ISMS SoA and CHANGELOG in the same change when practical.
- Isolation CI is a grep of the sidecar for keyring / russh / `secrets.json` vault code — a detective control, not a sandbox. The sidecar still reads `sidecar.token` for auth.

Organizational residual: required reviews, CODEOWNERS, signed commits, and a change ticket system are not enforced in git.
