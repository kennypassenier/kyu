# mailbox

A self-documenting message hub for the homelab: durable topics, named
subscriptions, send/receive/ack over plain HTTP, dashboard as
documentation. Rust; deployed as a Docker container in a Proxmox LXC.

This project follows the dev procedure in `~/Projects/dev-procedure/`
(`/project-flow`). Standing rules apply to every change:
`~/Projects/dev-procedure/STANDING_RULES.md`.
**Open sessions in THIS directory** — hooks and repo-scoped tools only
work here (standing rule 19).

## Procedure status

| Field | Value |
|---|---|
| Current phase | 6 · Development loop — L0 next |
| Last completed gate | Phase 5 realization plan (2026-08-12): L0–L8 + all 18 standing rules approved; enforcement installed and verified |
| Next gate | Phase 6 report form for L0 (walking skeleton) |
| Blocked on | Kenny's explicit go to create the public GitHub repo + first push (SR13) |
| AFK mode | off |

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

Commits are blocked by `.claude/hooks/check-commit.sh` unless
`.claude/hooks/gates.sh` passes and the message carries IDs in
brackets (`[W12]`, `[L4b]`, `[meta]`). CI re-runs the same gates on
every push; red blocks merge.
