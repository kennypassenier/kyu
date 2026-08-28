# mailbox

A self-documenting message hub for the homelab: durable topics, named
subscriptions, send/receive/ack over plain HTTP, dashboard as
documentation. Rust; deployed as a Docker container in a Proxmox LXC.

This project follows the dev procedure in `~/Projects/dev-procedure/`
(`/project-flow`). Standing rules apply to every change:
`~/Projects/dev-procedure/STANDING_RULES.md`. Sessions may be opened
from anywhere — the gates live in git hooks, not in session config.

## Procedure status

| Field | Value |
|---|---|
| Current phase | 7 · Hardening |
| Last completed gate | Phase 7 gap decisions (2026-08-28): 16 gaps, all closed; 144 tests |
| Next gate | Phase 7 report, then queued mini-rounds from the 2026-08-28 procedure update |
| AFK mode | off |

### Queued mini-rounds (Phase 2 mandatory items, added to the procedure after this project's freeze)

| Item | Status |
|---|---|
| Update & distribution mechanism | covered in substance by K13 (tag → GitHub Release → ghcr image, pulled by compose); to be recorded formally |
| Ecosystem integration | NOT discussed — mailbox ships a compose file but no homelab v2 preset manifest |
| Backup & restore | partially: W8 is a manual endpoint with an exercised restore; no AUTOMATIC scheduled backup |

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
