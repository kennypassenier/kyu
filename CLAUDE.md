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
| Current phase | **Mini-round W11 (2026-09-18) built, 3.3.0 unreleased** — `message.expired` once per subscription per `KYU_EXPIRED_EVENT_WINDOW_MS` (a day) with the count since the last one (migration 5), and a startup refusal when `KYU_STATE_DIR` and `KYU_DATA_DIR` disagree (fix-state-1). Found from the HA side: the 2026-09-04→10 flood of 27,991 events looping through Home Assistant, and the empty store the 3.x unit opened on CT 109 on 2026-09-10 (old store untouched at `/appdata/kyu/kyu-config/data/`, tokens gone, handed to Homelab Rust). Both fixes drove red first; gates green |
| Last completed gate | Mini-round decision form (2026-09-18): hub window 24 h counted · HA throttle 24 h per topic · client tokens → Homelab Rust session. Before that: chassis-rs 2.0.2 bump, released (3.2.1, 2026-09-10) |
| Next gate | The mini-round report + two correction forms + the release-go for 3.3.0, all in one form (2026-09-18). After the go: `scripts/sign-release.sh v3.3.0` is Kenny's, the CT 109 deploy is Homelab Rust's — and fix-state-1's guard WILL refuse to start there until `kyu.env` drops `KYU_DATA_DIR` (or the store question is settled), which is the measurement. Still open from 3.2.1: v3.2.1 was never signed/deployed (3.2.0 is live); chassis-rs's fix-3 follow-up; the `/api/clients` empty-array quirk |
| Next action | waiting on Kenny: the 3.3.0 report/correction/release form of 2026-09-18 |
| AFK mode | off |

### Queued mini-rounds (Phase 2 mandatory items, added to the procedure after this project's freeze)

| Item | Status |
|---|---|
| fix-events-1 · measurement | **OPEN** — at the first day on CT 109 with 3.3.0 live and a subscription nobody polls: `journalctl -u kyu | grep 'event=message.expired'` shows at most one per subscription per day. Fallback: the HA throttle holds the queue regardless |
| fix-state-1 · measurement | **OPEN** — at the 3.3.0 deploy on CT 109, before the store question is settled: `kyu --check` under the unit's environment refuses while `kyu.env` still carries `KYU_DATA_DIR`. Then Kenny decides old store (move three files) or new store (tokens re-issued by Homelab Rust) |
| W11 expiry window (mini-round 2026-09-18) | **BUILT, unreleased** — see Current phase. Release = Kenny's go in the open form |
| chassis-rs 2.0.2 bump | **DONE** 2026-09-10 — kit-only version bump (2.0.0 → 2.0.2), no public-API change. Surfaced a latent bug in kyu's own `tests/p7_cli.rs::run()` (read a child's stdout/stderr only after it exited; `--help`'s 8.4 KB of output now exceeds this sandbox's pipe buffer and deadlocks that pattern), fixed with two concurrent reader threads. Shipped as 3.2.1 |
| chassis-rs 2.0.0 adoption | **DONE** 2026-09-10 — `Client::adopted(id, name, token, issued_at)` replaces the field-by-field literal `Client` construction (the type is `#[non_exhaustive]` since 2.0.0). `chassis sync --write` folded kyu's own musl release pipeline (G1, below) back into full scaffold ownership and fixed `deploy/service.yml`'s `update_cmd` (fix-3). `tests/common`'s manual login workaround for a fixed `extra_env` token is gone (CF-12, reported back to chassis-rs). Shipped as 3.2.0 |
| G1 · static musl release binary, distroless/static restored | **DONE** 2026-09-09 — found live: Homelab Rust's attempted v3.1.0 deploy to CT 109 failed with `GLIBC_2.39' not found` (rolled back safely, no damage), because the chassis-rs 3.0.0 migration's scaffold Dockerfile had silently replaced kyu's own frozen T9 (static musl on `gcr.io/distroless/static`) with a dynamically-linked `debian:trixie-slim` image — nobody checked the adopted scaffold against kyu's own frozen architecture. README.md and compose.yml never stopped claiming the image was distroless/uid 65532; they were describing the promise, not what shipped. Restored exactly as T9 specifies, plus a CI gate (`ldd` on `dist/kyu`) that now fails the build if it ever regresses. Shipped as 3.1.1 |
| chassis-rs 1.8.0 upgrade + five additive adoptions | **DONE** 2026-09-09 — chassis-rs 1.7.1 → 1.8.0, kp-themes 3.1.0 → 5.0.0 (`chassis sync` reports "in sync"). K-vocabulary (`App::vocabulary("app","apps")` replaces `clients_label`), K-harness (`tests/common` rebuilt on `chassis::testing::TestApp`), K-cli (README names `chassis clients` as a headless alternative), K-kitdocs (README's door/license sections point at the new generated `docs/KIT.md` instead of re-explaining kit behaviour), K-actions ("Prune every dead letter" on the status page, `Engine::prune_dead_letters`). All five were "Onmisbaar" at the mini-round gate; K-actions needed one follow-up question since its own consequence text required Kenny to name the concrete button. Shipped as 3.1.0 |
| Dashboard usability from live use (W14/W15/W16) | **DONE** 2026-09-05 — Kenny's own feedback after using 2.4.0: human-readable timestamps everywhere (W14), deleting a dead letter instead of only requeuing it (W15), and a per-subscription backlog page reached by clicking its name on the topic page (W16). Editing a payload before requeue was considered and set aside — the payload lives on the message, shared by every subscription's delivery of it, so editing it in place would rewrite what every other subscription sees; the existing "Publish a test message" form is today's workaround. Shipped as 2.5.0 |
| The apps page exists even without a door | **DONE** 2026-09-05 — found by Kenny testing the 2.4.0 local preview: no `KYU_TOKEN` meant no "Apps" nav link and a bare JSON error on `GET /apps`. AR11's real guarantee (a bootstrap token gates creating app tokens) is unchanged; the page itself now always renders, explaining that and handing over a freshly generated `KYU_TOKEN`/`KYU_SECRET_KEY` pair. Shipped as 2.4.1 |
| Replace Bootstrap with the kp components | **DONE** 2026-09-05 — Kenny reopened this on the same day as the v3.0.0 mini-round below and asked for it outright. All four templates plus the login page now wear the package's own button, badge, card, alert, table, nav and form-field classes; `bootstrap.min.css` (233 KB) and `theme-bridge.css` (4 KB) are gone. `static/kyu.css` is what is left of kyu's own layer: layout glue, three badge tones the package deliberately does not ship, and a `:user-invalid` override (see kp-themes v3.0.0 row) |
| kp-themes v3.0.0 | **DONE** 2026-09-05 — eight files vendored from the v3.0.0 tag (`components.js` and `strings.js` are new); `static/kyu-init.js` calls the four attach functions kyu's markup needs instead of the package's own `js/auto.js`, since every module import became pure at 3.0.0. Two gaps in the release's own manifest, solved on kyu's side rather than raised with the project: `strings.js` is missing from `SHA256SUMS` despite being a hard import of both vendored modules kyu serves (hashed from the tag instead); `components.css`'s native `:invalid` styling paints every empty required field red before it is touched, worked around with `:user-invalid` in `kyu.css`. Revoking an app token now arms before it acts (DI10), via the vendored `components.js` rather than a hand-rolled confirm. Kenny decides whether either gap is worth telling kp-themes about |
| kp-themes v1.2.0 | DONE 2026-09-04, superseded by v3.0.0 above — six files vendored from the v1.2.0 tag (`no-flash.js` was new then), verified against the release's own checksums. Measured first: only the version banner in `themes.css` had moved, so nothing visible changed. The hand-written no-flash snippet was gone; a test compared the inlined text against the package's |
| Backup alerting — measurement 1 | **DONE** 2026-09-03 — the loop from correction form F179 is closed. Verified against its own criterion, not on a report: `LastTriggerUSec` of the **timer** = Thu 2026-09-03 03:00:45 UTC (the first firing after the fix), that run reports `Result=success`, and it wrote `kyu.backup-1788404445073.db` (593920 bytes) and pruned the oldest. Measurement 2 was proven on 2026-09-02 |
| Graceful shutdown + release assets | **DONE** 2026-09-02 — two homelab requirements (their D93/F172 and F168/T72). SIGTERM now finishes in-flight requests, checkpoints the store and exits 0, bounded by KYU_SHUTDOWN_TIMEOUT_MS; every tag attaches the binary and SHA256SUMS, extracted from the image so one compile serves both |
| Rename mailbox → kyu | **DONE** 2026-08-29 — the old name said email about a queue. Everything moved (env vars, headers, metrics, cookie, event topic, paths), so it shipped as 2.0.0 |
| Shared-token auth (W2) | **DONE** 2026-08-28 — door, per-app tokens, login page, masked snippets |
| Update & distribution (M1) | **DONE** 2026-08-28 — `release-image.yml` adopted from the homelab template; K13's false "verified end-to-end" claim removed. Proven end-to-end 2026-09-05: `homelab install-native stacks/kyu` took CT 109 from 2.2.0 to 2.4.1, `kyu --version` and `/healthz` confirmed after |
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
