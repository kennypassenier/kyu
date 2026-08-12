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
| Current phase | 3 · Tech choice |
| Last completed gate | Phase 2 feature freeze (2026-08-12): 24 features, 15 Essential / 6 Desired / 3 Later / 0 Don't do |
| Next gate | Phase 3 decision form (Rust libraries, dependency policy, MSRV) |
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
