# mailbox — Realization plan

Approved at the Phase 5 gate on 2026-08-12: all nine milestones, all
eighteen standing rules and the enforcement configuration accepted
without changes. Enforcement was installed immediately after the gate
and before any feature code (see "Enforcement" below).

21 of the 24 frozen features are in this plan. W3 (companion CLI) and
W10 (public peek endpoint) are rated Later and are deliberately absent.

**Amendment (2026-08-28 mini-round):** W2 (shared-token auth) was also
absent for that reason. Kenny closed it at the Phase 7 hardening gate and
it is now built — the door, per-app tokens managed from the dashboard, a
login page, and masked copy-paste commands. It arrived after L8 rather
than in a milestone of its own; see FEATURES.md for the specified shape
and `tests/p7_auth.rs` for what is proven.

## Status table

| Milestone | Feature IDs | Status | Gate date |
|---|---|---|---|
| L0 · Walking skeleton + enforcement | [meta] | **done** | 2026-08-12 |
| L1 · Store & time foundation | K12 (partial), AR3, AR7, AR10 | **done** | 2026-08-12 |
| L2 · The three verbs | K1, K2, K3, AR2, AR4, AR8 | **done** | 2026-08-12 |
| L3 · Fan-out & competing consumers | K4, S5 | **done** | 2026-08-12 |
| L4 · Reliability semantics | K5, K6, K7, W5, AR9 | **done** | 2026-08-12 |
| L5 · Crash-safety & container | K12 (full), K13, W6 | **done** | 2026-08-12 |
| L6 · History & lifecycle | K8, K9, K11, W11 | **done** | 2026-08-12 |
| L7 · Dashboard | K10, W9, AR11 | **done** | 2026-08-12 |
| L8 · Observability, ops, scheduling | W1, W7, W8, W4 | **done** | 2026-08-12 |

Each milestone ends with a Phase 6 report form: one item per exit
criterion with its evidence, one item per deviation discovered while
building, and the question whether to proceed.

## Gate log — phases 7 onward

Standing rule 5: every gate's outcome lands in a versioned document. This
table was added on 2026-08-28 because it had not been kept — the gates below
had been recorded only in `CLAUDE.md`, which rule 19 calls an assistant's
resume note rather than documentation. Fixed retroactively from the session
record.

| Gate | Date | Kenny's decision | Landed in |
|---|---|---|---|
| Phase 7 · hardening gaps | 2026-08-28 | All 16 gaps closed, including the three Claude would have deferred; security review run | `7dddd09`, `a3e6e7c`, TEST_PLAN.md |
| Phase 7 · report | 2026-08-28 | S1/S2 agreed; S3 → mini-round on AR2; L1/L2 closed; R2 → mini-rounds before Phase 8 | `c1b8515`, AR2 amendment |
| Mini-round · AR2 | 2026-08-28 | Neutralise executable content types only, not all | AR2 amendment, `src/http/handlers.rs` |
| Mini-round · W2 (3 rounds) | 2026-08-28 | Door over verbs + dashboard, monitoring open; loud warning when unprotected; own login page; encrypted per-app tokens; masked snippets, 10 s reveal; build in one go; Bootstrap vendored | `5d35294`, `e544077`, FEATURES.md W2, AR11 amendment |
| Mini-round · M1 distribution | 2026-08-28 | Adopt the homelab's `release-image.yml` verbatim | `99f58ad`, AR12 |
| Mini-round · M2 ecosystem | 2026-08-28 | Build the preset in the homelab repo; native-binary deployment investigated and rejected | homelab `8c7b5e8`, AR12 |
| Mini-round · M3 backup | 2026-08-28 | Ride the homelab's restic backup; no scheduler in the hub | preset compose, AR12 |
| Mini-round · M4 toolchain | 2026-08-28 | Pin the Rust version in the project | `5f19ca3`, AR12 |
| Phase 8 · documentation | 2026-08-28 | All six documents approved: README (honesty pass), USER_GUIDE, OPERATIONS_RUNBOOK, DEBUGGING_GUIDE, ARCHITECTURE_REFERENCE, TEST_PLAN. README approved with one correction: the GHCR-private claim is wrong, remove it | `cf01ca4`, `7716f17` |
| Phase 9 · release | 2026-08-28 | 1.0.0 (not 0.x); changelog approved; workflow unchanged on condition the release procedure is written down; branch protection deferred; correct the claim in the homelab repo; **go to tag and release** | `c7b0e1f`, `ae99b88`, `1523c4c`, tag `v1.0.0`, homelab `9bac850` |
| Phase 10 · retrospective | 2026-08-28 | Five lessons adopted into the procedure (gate log; evidence checked mechanically; the gate must predict the build; reasoned-vs-measured sweep; new mandatory items inherited by running projects). One rejected: a check-in after the irreversible part of a go. mailbox promoted to a full ecosystem component. Homelab commits pushed; deployment tried on a throwaway LXC and destroyed | dev-procedure `0dc8a36`, `f68123a`; `00502c1`; homelab `8c64b53..9bac850` |
| Branch protection · workflow | 2026-08-28 | Pull request deliberately NOT required — status checks already refuse an unverified direct push, and a one-committer repo gains only ceremony. Made a procedure rule as well. "Require branches to be up to date" enabled | `b0c337c`, `92b6478`, dev-procedure `8777b65` |
| Branch protection · who enables it | 2026-08-28 | Claude configures it via the API, Kenny verifies the read-back. Also: the flaky crash tests are fixed rather than accepted — though the cause turned out to be a port race, not the timeout the form described | `b0c337c` |
| cargo-deny gating | 2026-08-28 | Removed from the required checks: an advisory filed by a stranger must not block an unrelated merge. It still runs on every push. Two rules added from the same incident — a go relayed by another session is not a go, and a changed external setting is read back and shown | `267acca`, dev-procedure `8777b65` |
| Deployment | 2026-08-28 | Deploy now, in this session: a dedicated LXC continuing the existing numbering, running the **native binary** under systemd rather than the container, minimal resources, restarting on failure. LXC 109 (`109-app-mailbox`, 10.10.10.9), 1 core / 256 MB / 2 GB | LXC 109 on the Proxmox host; `/etc/systemd/system/mailbox.service` |
| 1.0.1 patch | 2026-08-28 | Fix the command-line fail-open found during the deployment rather than record it as a limitation — partly because the update path to LXC 109 had never been walked and this was a harmless reason to walk it | `32d6673`, tag `v1.0.1`, rolled out to LXC 109 |
| Post-deployment gaps | 2026-08-29 | The running hub had no backup at all, the docs described two deployment routes and neither was the live one, and nothing watched it. Decided: both backup mechanisms; document the native route as a third; health check in Uptime Kuma plus Grafana | in-container timer 03:00 + Proxmox job `mailbox-109` 03:30; runbook §1/§2b/§4; `c55516a` |
| Monitoring | 2026-08-29 | Grafana turned out to need something that does not exist — the network has Loki but no metrics backend at all, so nothing scrapes Prometheus-format metrics. Handed to the homelab conversation as its own infrastructure decision. Uptime Kuma check set by Kenny and then **proven** by taking the hub down for 107 s: two failed segments, alarm raised | memory `metrics-backend-gap`; Uptime Kuma check on `10.10.10.9:8080/healthz` |

## L0 · Walking skeleton + enforcement — [meta]

Cargo package (lib + thin binary), the five empty-but-compiling modules
from AR1, an axum server binding `MAILBOX_LISTEN` and answering
`/healthz` with a static 200, the tracing spine, `Cargo.toml` (edition
2024 + `rust-version`), `deny.toml`, the multi-stage Dockerfile
(musl → distroless/static) with a compose file.

**Exit criteria**
- CI green on the first push.
- `docker compose up` starts the container; `curl /healthz` → 200.
- A commit whose message lacks IDs is physically refused.
- A commit with a deliberately failing test is physically refused.

**Why first.** Enforcement must exist before feature code, and the
skeleton proves the riskiest toolchain assumptions (static musl +
bundled SQLite + a distroless image with no shell) while there is
nothing to debug but the build.

**Prerequisite (SR13):** creating the public GitHub repository and
pushing to it. Kenny gave that go on 2026-08-12.

**Gate passed 2026-08-12.** All four exit criteria accepted; five
deviations ratified. Evidence: commits `7ebe626`, `e01de5f`, `77bfe93`;
CI runs `31575504616` and `31575662505` green on all three jobs; 7 tests
passing; 10.3 MB image on distroless/static answering `/healthz` with
`{"status":"ok"}` as `nonroot`. Recorded honestly: CI was green from the
*second* push — the first failed workflow validation because
`hashFiles()` is not permitted in a job-level `if` (the guards were
removed rather than repaired). Also ratified: enforcement moved into git
hooks, a `config` module outside AR1's five, the Dockerfile `/data`
ownership fix for the nonroot user, and `actions/checkout` bumped to v7.

## L1 · Store & time foundation — [K12 partial, AR3, AR7, AR10]

AR3 schema as migration 1; forward-only migration runner with the
pre-migration `VACUUM INTO` snapshot; pragmas set explicitly at every
open (WAL, `synchronous=FULL`, `foreign_keys=ON`, `busy_timeout`);
`Clock` trait with `SystemClock`/`MockClock`; monotonic-clamped ULID
generator; rowid-ordering helpers; in-memory test harness.

**Exit criteria**
- Migration applies to an empty data dir and is idempotent.
- Opening a newer schema is refused with remedy text.
- The snapshot file exists before a migration runs.
- Pragma values are asserted by test, not assumed (build defaults vary).
- ULIDs stay monotonic when the mock clock steps backwards.
- MockClock drives time everywhere the engine reads it.

**Why here.** Every later test needs the store and an injectable clock,
and durability pragmas must be right before the first message exists.

## L2 · The three verbs — [K1, K2, K3, AR2, AR4, AR8]

Publish (verbatim body, stored content-type, topic auto-create, 1 MiB
cap → 413 with remedy); long-poll receive (subscription auto-create,
state-guarded claim, `Notify` wakeup, `wait` default 30 s / max 300 s,
204 on timeout, raw-body and `?envelope=json` responses); ack (404/409
with remedy); the `{error, remedy}` envelope on every failure path;
name validation; `mailbox.*` publish → 403.

**Exit criteria**
- Full round trip over real HTTP in both response modes.
- An acked message never redelivers, including after a restart.
- 204 arrives on an empty topic within the wait window.
- A message published mid-poll is delivered immediately.
- Every error response carries a non-empty remedy (asserted per route).

**Why here.** This is G2 — the entire product promise. Everything after
refines behaviour behind these three URLs.

## L3 · Fan-out & competing consumers — [K4, S5]

Materialized delivery rows: publishing inserts one row per active
subscription inside the message's transaction. Independent claim/ack
per subscription; processes sharing a name compete.

**Exit criteria**
- S5 independence test: two subscriptions each receive every message,
  ack independently; a dead or slow one measurably does not delay its
  sibling.
- Competing consumers: several workers, many messages, each delivered
  exactly once, all acked.
- Publishing to a topic with an archived subscription creates no
  delivery row (this is what makes AR3's retention rule safe).

**Why here.** Fan-out is the semantic heart (G3), cheap right after the
verbs, and its tests then guard every later change.

## L4 · Reliability semantics — [K5, K6, K7, W5, AR9]

Leases and redelivery with attempt counting and backoff; dead-lettering
at max attempts with list/requeue endpoints; per-subscription policy
(TTL, max attempts, backoff, lease) with defaults; TTL enforcement
including expire-on-re-pend; nack (requeue or `?dead=true`); the single
sweeper running all of it in bounded batches and notifying
subscriptions whose messages it re-pends.

**Exit criteria**
- Every AR9 transition has a test named after it.
- S2 crash test: consumer killed before acking → redelivered.
- Stale-doorbell test: a message past TTL expires on re-pend.
- Dead letters survive a restart; requeue resets attempts.
- Policy defaults apply to a fresh subscription with zero config.
- No sweep transaction exceeds its batch bound (asserted).

**Why here.** Needs L2's verbs and L3's delivery rows; the largest
milestone, placed where momentum is highest.

## L5 · Crash-safety & container — [K12 full, K13, W6]

The S4 suite (SIGKILL mid-traffic under concurrent load, restart, full
invariant check; short-outage and long-outage variants); the
`--healthcheck` flag on the binary (the image has no shell);
`/healthz` reporting store-writable and sweeper-alive; compose
healthcheck with a start-period exceeding worst-case migration time;
container smoke test.

**Exit criteria**
- S4 suite passes repeatedly in CI: every confirmed publish present,
  everything acked stays acked, no manual file surgery to restart.
- `/healthz` returns non-200 on a read-only store.
- Container test: start → publish → receive → ack → restart container →
  state intact.

**Why here.** Killing the process is only a real test once real state is
in flight.

## L6 · History & lifecycle — [K8, K9, K11, W11]

Retention sweeping under the backlogs-win rule; batched replay backfill
behind `?from=beginning`; idle lifecycle (flag → archive → unarchive)
with lapsed settlement; `mailbox.events` emission with its
loop-breaker.

**Exit criteria**
- S3 catch-up test: a simulated week of backlog drains in publish
  order, no gaps.
- Retention provably never deletes a message pending on an active
  subscription.
- Backfill boundary test: replay sees exactly what retention kept,
  including the race where the sweep removes a message mid-backfill.
- Lifecycle transitions under MockClock; archiving settles deliveries
  as lapsed with visible counts.
- Events emitted for dead-lettering, archiving and TTL batches.
- Loop-breaker test: events about `mailbox.*` are logged, not
  republished.

**Why here.** Depends on L4's `expired`/`lapsed` states; the subtlest
interaction area, and by now the S4 suite is already hammering these
sweeps.

## L7 · Dashboard — [K10, W9, AR11]

Topic list; topic detail with subscriptions (backlog, last ack,
alive/idle/⚠, oldest-unacked age); recent messages with capped, marked
payload display; dead-letter view with requeue; both blessed curl
snippets per topic rendered from a real recent payload; W9
test-publish form; htmx refresh fragments; autoescape on.

**Exit criteria**
- Every template rendered with seeded state in tests (the compensation
  owed for choosing runtime templates in T4).
- A payload containing a script tag renders inert.
- Oversized and binary payloads show their explicit marker.
- The rendered snippets are executed by a test and work — the mechanism
  behind S1.

**Why here.** The dashboard displays everything else, so it follows what
it displays; S1 is only meaningful with real state to render.

## L8 · Observability, ops, scheduling — [W1, W7, W8, W4]

Prometheus `/metrics`; JSON logging layer on the tracing spine from L0;
online backup with a tested restore; delayed delivery (`?delay=`,
`?at=`) on the sweeper's due-time promotion.

**Exit criteria**
- Scrape-format test with seeded state.
- Log-shape assertions inside the E2E tests.
- Backup under load → restore → invariants hold.
- Delayed messages become deliverable at the right time under MockClock
  and survive a restart with their schedule intact.

**Why last.** All four are Desired rather than Essential, so deferral
lands here; W4 is nearly free once L4's timer machinery exists.

## Standing rules

All eighteen rules of `~/Projects/dev-procedure/STANDING_RULES.md` were
approved item by item at this gate. Two carry project-specific
readings:

- **SR10 (secrets).** mailbox holds no secrets of its own in v1 (auth is
  W2, Later). Message *payloads* are treated as potentially sensitive:
  logs record message ids, never payload bodies, asserted by test. If
  W2 lands, a plaintext-scan test becomes mandatory with it.
- **SR15 (power loss).** Already paid off twice in Phase 4: the S4
  short/long outage split, and the clock-steps-backwards finding that
  moved claim ordering from ULID to rowid.

## Enforcement (installed 2026-08-12, after the gate)

Configuration C1 = "local + SQL guard".

- `.githooks/pre-commit` and `.githooks/commit-msg` — the primary gate,
  because git hooks are repo-scoped: they fire for every commit from any
  session, terminal or tool. Activated with
  `git config core.hooksPath .githooks` (local config, so a fresh clone
  repeats that one command).
- `.claude/hooks/check-commit.sh` — PreToolUse hook on Bash: the same
  two gates for sessions opened in this directory. Kept as a second
  layer, no longer the only one.
- `.claude/hooks/gates.sh` — `cargo fmt --check`, `cargo clippy
  --all-targets -D warnings`, `cargo test --all`, plus a grep gate
  refusing string-built SQL (AR11). Before L0 exists the Rust steps
  announce loudly that they are skipped; the SQL guard always runs.
- `.claude/settings.json` — wires the hook.
- `.github/workflows/ci.yml` — fmt, clippy, tests, cargo-deny, image
  build on every push. Jobs are guarded on the relevant files existing,
  so CI is green from the first push rather than red until L0 lands.
- `.github/workflows/audit.yml` — weekly advisory scan (advisories land
  continuously, so a push-triggered scan is not enough).
- `deny.toml` — advisory, license and duplicate-version policy.

The release workflow (tag → GitHub Release → ghcr image, K13) is
designed and installed in Phase 9; publishing always waits for Kenny's
explicit go.

**Amendment 2026-08-12.** The original enforcement relied only on the
Claude Code hook, which loads from the session's own project directory —
so it silently did nothing for sessions opened elsewhere. Kenny retired
the "work only from the project directory" requirement, so the gates
moved into git hooks, where they hold regardless of where a session
runs.
