# kyu

A self-documenting message hub for the homelab: durable topics, named
subscriptions, send/receive/ack over plain HTTP, dashboard as
documentation. Rust; runs as a native binary under systemd in a Proxmox
LXC (a container image is published too, but the live route is native).

This project follows the dev procedure in `~/Projects/dev-procedure/`
(`/project-flow`). Standing rules apply to every change:
`~/Projects/dev-procedure/STANDING_RULES.md`. Sessions may be opened
from anywhere — the gates live in git hooks, not in session config.

## Procedure status

| Field | Value |
|---|---|
| Current phase | **Done** — all eleven phases complete |
| Last completed gate | Mini-round (2026-09-02): graceful shutdown on SIGTERM (W12) and a release that publishes the binary with checksums |
| Next gate | None. kyu is released (2.1.0), deployed on LXC 109 and monitored. Further work arrives as mini-rounds |
| AFK mode | off |

### Queued mini-rounds (Phase 2 mandatory items, added to the procedure after this project's freeze)

| Item | Status |
|---|---|
| Backup alerting — measurement 1 | **OPEN** — the loop from correction form F179. Criterion sharpened 2026-09-02 after the homelab read manual drill runs as the nightly one: a fresh backup and `Result=success` are NOT enough, because a hand-started run produces both. It closes only when `systemctl show kyu-backup.timer -p LastTriggerUSec` is later than 2026-09-02 01:14 UTC **and** that run reports success **and** left a copy. Measurement 2 (reproducing the fault to prove the alarm fires) is DONE and proven |
| Graceful shutdown + release assets | **DONE** 2026-09-02 — two homelab requirements (their D93/F172 and F168/T72). SIGTERM now finishes in-flight requests, checkpoints the store and exits 0, bounded by KYU_SHUTDOWN_TIMEOUT_MS; every tag attaches the binary and SHA256SUMS, extracted from the image so one compile serves both |
| Rename mailbox → kyu | **DONE** 2026-08-29 — the old name said email about a queue. Everything moved (env vars, headers, metrics, cookie, event topic, paths), so it shipped as 2.0.0 |
| Shared-token auth (W2) | **DONE** 2026-08-28 — door, per-app tokens, login page, masked snippets |
| Update & distribution (M1) | **DONE** 2026-08-28 — `release-image.yml` adopted from the homelab template; K13's false "verified end-to-end" claim removed. Unproven until the first tag (Phase 9) |
| Ecosystem integration (M2) | **DONE** 2026-08-28 — `presets/kyu/` committed in ~/Projects/homelab (8c7b5e8, not pushed). Native-binary deployment investigated and rejected: not built or planned there |
| Backup & restore (M3) | **DONE** 2026-08-28 — rides the homelab's restic backup via the preset's `/appdata` bind + pause label; no in-hub scheduler, on purpose |
| Toolchain pin (M4) | **DONE** 2026-08-28 — `rust-toolchain.toml`, after a green local gate let a red CI through |

<!-- Update this block after every completed gate. -->

## Project documents

| Doc | Purpose |
|---|---|
| docs/SCOPE.md | goals, non-goals, success criteria, constraints (Phase 0) |
| docs/FEATURES.md | rated feature list with permanent IDs (Phase 2) |
| docs/ARCHITECTURE_DECISIONS.md | frozen AR decisions incl. tech choice (Phases 3-4) |
| docs/REALIZATION_PLAN.md | milestones + status table (Phase 5) |
| docs/TEST_PLAN.md | what is proven where + accepted limitations (Phase 7) |

## Gates (enforced)

Commits are blocked unless `.claude/hooks/gates.sh` passes and the
message carries IDs in brackets (`[K6, AR9]`, `[L4]`, `[meta]`).
Enforced twice over:

- **`.githooks/pre-commit` + `.githooks/commit-msg`** — repo-scoped, so
  they fire for every commit from any session, terminal or tool. A fresh
  clone activates them with `git config core.hooksPath .githooks`.
- **`.claude/hooks/check-commit.sh`** via `.claude/settings.json` — the
  same two gates for sessions opened in this directory.

CI re-runs everything on every push; red CI blocks the next commit.
