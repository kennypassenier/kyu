# kyu — Features

**FROZEN 2026-08-12** via the Phase 2 gate (rating rounds 1+2, report
form R1–R3 all approved). Changes only via mini-rounds
(FORM_PROTOCOL §5), recorded here as dated amendments.

Tally: 17 Essential · 7 Desired · 2 Later · 0 Don't do.
*(W2 promoted from Later to Essential by the 2026-08-28 mini-round;
W12 added as Essential and W13 as Desired by the 2026-09-02
mini-rounds.)*
IDs are permanent: they appear in commits, test names, docs and forms.

Ratings use the canonical English scale (Essential/Desired/Later/Don't
do); the forms rendered them in Dutch (Onmisbaar/Gewenst/Later/Niet
doen) per FORM_PROTOCOL.

## Essential (17)

### K1 · Publish
`POST /t/<topic>` with arbitrary JSON/bytes body. Topic auto-created on
first use. Response carries the message id and arrives only after the
message is durably on disk. Oversized bodies rejected loudly (limit is
explicit, never silent).
**Proven by:** auto-tests for publish→id, topic auto-creation,
oversize rejection. Durability itself → K12 suite.

### K2 · Long-poll receive
`GET /t/<topic>/next?as=<sub>` blocks until a message arrives or the
poll timeout passes (empty response; client re-polls). First use of a
new `as=` name auto-creates the subscription. Receiving claims the
message: invisible to same-name competitors until acked or lease
expires.
**Proven by:** instant delivery when a message waits; blocking + wakeup
mid-poll; clean timeout; claim invisibility.

### K3 · Ack
`POST /t/<topic>/ack/<id>?as=<sub>` settles the message for that
subscription only. Unknown/duplicate acks return clear errors with
remedy text.
**Proven by:** acked-never-redelivered (incl. across restart);
double-ack and wrong-subscription error tests.

### K4 · Fan-out and load balancing via subscription names
Different `as=` names each receive every message, acking
independently. Same name from multiple processes competes: each message
delivered to exactly one.
**Proven by:** S5 fan-out independence test; competing-consumer test
(N workers, M messages, each delivered exactly once); dead-slow-sibling
test (zero effect on the other subscription).

### K5 · Leases and redelivery (at-least-once)
Received-but-unacked messages return to the subscription when the claim
lease expires. Redelivery count tracked per message per subscription
(feeds K6).
**Proven by:** S2 crash test (kill consumer pre-ack → redelivered);
mocked-clock lease-expiry timing; counter increments.

### K6 · Dead letters
At max retries the message moves to the subscription's dead-letter
list: payload, arrival time, retry history, failure age all visible on
the dashboard; one click / one API call requeues. Dead letters persist
until dealt with.
**Proven by:** transition at exactly max-retries; requeue-from-DLQ;
persistence across restart; manual dashboard check in Phase 8.

### K7 · Per-subscription delivery policy
TTL, max retries, retry backoff, lease duration — settable per
subscription (API or dashboard), sensible defaults, zero config needed
for a fresh subscription. TTL expiry is a recorded event, not silence.
**Proven by:** mocked-clock TTL expiry; backoff schedule; defaults
test.

### K8 · Start position
New subscriptions start at "now"; `?from=beginning` on first receive
starts at the oldest retained message. Depends on K9.
**Proven by:** both start positions; replay-sees-exactly-what-retention-
kept boundary test.

### K9 · Topic retention
Fully-acked messages are retained on the topic for replay (K8) and
dashboard history. Per-topic setting (default: keep 7 days; overridable
to more/less/forever). Cleanup is a visible sweep with counts in the
log.
**Proven by:** mocked-clock retention sweeps; replay-after-sweep
boundary test.

### K10 · Dashboard
Served by the same binary: all topics; per topic its subscriptions
(backlog, last ack, alive/idle/⚠), recent messages with payloads, dead
letters, and per-topic copy-paste curl examples rendered with a real
recent payload. English UI, no login (W2 is Later).
**Proven by:** route/render auto-tests with seeded data; S1 re-entry
walkthrough as scripted manual test (Phase 8).

### K11 · Idle-subscription lifecycle
Unpolled for X days → ⚠ flag on dashboard; after Y days → archived
(stops accumulating, keeps what it had), announced via log and W11
`kyu.events`. Unarchive is one click.
**Proven by:** mocked-clock lifecycle transitions
(active→flagged→archived→unarchived); announcement-emitted assertions.

### K12 · Crash-safe storage
A publish confirmed to the client survives SIGKILL and power loss;
acks, cursors, leases and DLQ state survive with it. Recovery after
hard kill is automatic — no manual file surgery ever. All writes
atomic.
**Proven by:** S4 suite — repeated SIGKILL-under-load in CI, restart,
full invariant check; short- and long-outage variants.

### K13 · Docker distribution via GitHub Actions *(amended at the gate)*
Ships as a Docker container. Pushing a version tag to GitHub triggers
GitHub Actions to build the binary, create a GitHub Release, and
publish the image to ghcr.io. The LXC deployment pulls that image via
compose (one service, one volume, one port). Release publishing always
behind Kenny's explicit go (Phase 9).
**Proven by:** CI builds the image on every push; container smoke test
(start → publish → receive → ack → restart container → state intact, plus
the same volume against a freshly built image, plus a protected hub).

**Correction (2026-08-28 mini-round M1):** this entry used to close with
"tag→release→ghcr pipeline verified end-to-end with a pre-release tag".
That was never true. `.github/workflows/` held only CI and the advisory
job — no GHCR reference, no release action, no tag trigger — and the repo
had no tags and no releases. The claim is removed rather than softened.
`release-image.yml` now exists, taken from the homelab's
`templates/rust-service/`, so every one of Kenny's Rust repos ships the
same way.

**Proven on 2026-08-28** by the `v1.0.0` tag: the workflow ran, published
`ghcr.io/kennypassenier/kyu:1.0.0` and `:latest`, and the image was
pulled anonymously and exercised — health, a refused tokenless publish, and
a message through publish → receive → ack — before the GitHub Release was
written. The claim this entry once made falsely is now true and dated.

One part of the wording above is still unmet and is left visible rather
than quietly dropped: the adopted workflow publishes the **image**, not a
**GitHub Release**. Creating the release stays a deliberate
`gh release create` at Phase 9, where release notes are written by a human
anyway. Adding a release step to the workflow would have meant diverging
from the shared template on my own initiative, which is not what Kenny
chose.

### W6 · Health endpoint
`/healthz` returns 200 + small JSON (store writable, sweeps alive).
Feeds Uptime Kuma and the Docker healthcheck in K13's compose.
**Proven by:** healthy and degraded (read-only store → non-200) tests.

### W11 · `kyu.events` system topic
The hub publishes its own noteworthy events as ordinary messages:
dead-lettered, subscription flagged/archived, TTL batch expired.
Consumed via K2 like any topic (e.g. HA automation → office light).
Rated Essential by Kenny (above Claude's Desired recommendation) — K6
and K11 announcements are built on it from the start.
**Proven by:** event-emitted assertions inside the K6/K11 suites.

### W2 · Shared-token auth *(promoted at the 2026-08-28 mini-round)*

Rated **Later** at the Phase 2 freeze on the reasoning that a LAN-only
hub with a single admin needs no door (N3). Kenny closed it at the
Phase 7 hardening gate and specified the shape across three rounds:

- **Guarded:** the three verbs (K1/K2/K3), the dashboard (K10) and its
  two write buttons. **Open:** `/healthz` (W6) and `/metrics` (W1), so
  Uptime Kuma and Grafana keep working untouched — a monitoring stack
  that fails closed lies to you during an outage, which is exactly when
  you believe it.
- **No token configured:** the hub starts anyway, with a warning on
  every startup and a banner on every dashboard page. Refusing to start
  turns a forgotten variable into an outage; starting silently is the
  failure this decision exists to prevent.
- **Browser access:** a small login page with a remember-me cookie and
  a logout button, not browser-native basic auth (no way to log out of
  that without clearing browser data).
- **App management:** a dashboard section to register an app and
  generate a token for it, and to revoke one. Tokens are stored
  **encrypted** (AR11 amendment) so the dashboard can always reproduce
  a working command. The encryption key (`KYU_SECRET_KEY`) is
  mandatory alongside the token, so rotating a leaked bootstrap token
  never silently orphans every app token.
- **Snippets:** the copy-paste commands carry a real, working token. It
  is masked on screen; a reveal button shows it for 10 seconds and a
  copy button puts the whole command on the clipboard without ever
  displaying it — protection against someone glancing at the screen,
  not against someone who already logged in.

**Plaintext-scan test is mandatory** (carried over from the original
rating): no token may appear in logs, metric labels or any rendered
page except behind the reveal control.

### W12 · Graceful shutdown on SIGTERM *(added at the 2026-09-02 mini-round)*

Did not exist at the Phase 2 freeze and was never missed, because
nothing about durability needs it: K12 ↳ *crash-safe storage* is proven
by killing the process with SIGKILL ten times in a row and restarting
without repair. kyu caught no signals at all, so `systemctl stop kyu`
ended the process where it stood.

Two things brought it back. The homelab's nightly file-level backup of
CT 109 ↳ *the container kyu runs on* failed with
`tar: kyu.db-wal: file changed as we read it`, because a running hub
keeps rewriting the write-ahead log and there was no moment when the
files stood still. And Kenny made a graceful stop the norm for every
Rust service in this ecosystem on 2026-09-02 (homelab D93): kyu was the
only one of four without it, and the only one with a database.

- **Both signals:** SIGTERM (systemd, `docker stop`) and Ctrl-C.
- **In-flight requests are answered**, not cut off — a long-poll
  consumer is this hub's normal state, so a reset connection would be
  the common case rather than the rare one.
- **The store is settled** with `PRAGMA wal_checkpoint(TRUNCATE)`, so
  the log is empty and a plain `tar` of the data directory restores.
- **Bounded and configurable:** `KYU_SHUTDOWN_TIMEOUT_MS`, default
  10000. Blowing the budget logs one loud line naming what was still
  open and exits **0** anyway — a stop that hangs is worse than a stop
  that is incomplete, and an exit code of 1 would make systemd report a
  clean stop as a crash.
- **Idempotent:** further signals during shutdown change nothing. The
  escape hatch stays systemd's `TimeoutStopSec`, set above kyu's own
  budget.

Proven by `tests/w12_shutdown.rs` (5 tests, real signals against the
real binary): a clean exit code, a truncated log, the backlog intact
across the stop, three SIGTERMs in a row still exiting 0, and an
in-flight long poll answered rather than dropped.

## Desired (7)

### W1 · Prometheus metrics
`/metrics`: per-subscription backlog, delivery/ack rates, DLQ counts,
lease expirations, store size. **Proven by:** scrape-format test with
seeded state.

### W4 · Delayed / scheduled delivery
`?delay=30m` or `?at=<timestamp>` on publish; durable immediately,
deliverable at due time; visible on dashboard with due time.
**Proven by:** mocked-clock due-time tests; restart-preserves-schedule.

### W5 · Nack
`POST /t/<topic>/nack/<id>?as=<sub>`: immediate requeue (counts as
retry) or `?dead=true` straight to DLQ (poison pill).
**Proven by:** requeue-with-counter; straight-to-DLQ test.

### W7 · Structured JSON logging
JSON lines (event, topic, subscription, message id, outcome);
human-readable in dev. **Proven by:** log-shape assertions in E2E
tests.

### W8 · Online backup & restore
Consistent snapshot while running (e.g. `VACUUM INTO`), via endpoint or
schedule; documented, tested restore. **Proven by:**
backup-under-load → restore → invariants-hold E2E test.

### W9 · Dashboard test-publish
Per-topic form, payload prefilled with last real payload, send button.
**Proven by:** UI route test → message lands on topic.

### W13 · The house themes *(added at the 2026-09-02 mini-round)*

Kenny asked for the seven themes from `@kp-soft/themes` — the shared
package born out of JobTracker's Phase 3 (T17) and Phase 5 (P0) decisions
— with *the same picker and the same way of storing the choice in the
browser*. Not a lookalike: the same contract, so a theme chosen in one of
his apps behaves identically in the next.

- **The seven:** formal (the default), light, dark, cyberpunk, pastel,
  terminal, topo. Three of them are dark: dark, cyberpunk, terminal.
- **The contract that must not drift**, because three projects share it:
  `localStorage` key `theme`, the seven names as values, applied as
  `data-theme` on `<html>` plus the class `dark`, default `formal`.
- **kyu cannot use the package.** It has no npm and no build step, and the
  package ships a React hook and a JSX component. `css/themes.css` is
  therefore vendored verbatim with its version and commit recorded above
  it, and the switcher's behaviour is reimplemented in ~130 lines of
  plain JavaScript.
- **One list, not three.** The themes live once in `dashboard.rs`, are
  rendered server-side into the picker, and the browser script reads what
  it needs off that markup — no `THEME_META` copy in JavaScript.
- **The bridge is kyu's own file.** `static/theme-bridge.css` maps the
  package's tokens onto Bootstrap's `--bs-*` variables, and sets
  `data-bs-theme` for the dark themes so Bootstrap's own components follow
  instead of staying light under dark tokens. Keeping it separate means
  re-copying the upstream file never overwrites it.
- **Deliberately not taken:** `cyberpunk-register.css` (much of it targets
  shadcn `data-slot` attributes and the React fx, so it would be dead
  weight here) and the four `fx/` components (React).

Proven by `tests/w13_themes.rs`: all seven offered with their swatches and
dark flags, the storage contract pinned literally, the assets served with
the right types while the traversal guard still holds, and every theme the
picker offers actually defined in the stylesheet.

**Keeping the copy honest** (decided 2026-09-02). A vendored file goes
stale in silence — the colours still work, the page still renders, and
nothing says the package moved on. `.claude/hooks/gates.sh` therefore
compares `static/themes.css` against `~/Projects/kp-themes/css/themes.css`
whenever that repository is on the machine, and refuses the commit when they
differ, naming the exact lines. Where it is absent (CI) it says so out loud
rather than passing quietly. Proven by changing one character and watching
the gate refuse, then reverting.

**No contrast gate here, on purpose** (Kenny, 2026-09-02). Both JobTracker
and the almanac session proposed vendoring `check-contrast.mjs` and running
it in kyu's CI. It was declined with its reason written into the copy's own
header: kp-themes runs that gate before it tags, so this file has already
passed it, and re-running it would mean pulling Node into a Rust pipeline to
re-answer an answered question. It would only ever catch someone editing
this copy, which is forbidden anyway. The risk a copy actually runs is
staleness, and contrast says nothing about that.

## Later (2)

- **W3 · Companion CLI** — `kyu send/tail/ls`; only if the
  curl+dashboard bet proves insufficient.
- **W10 · Public peek endpoint** — `GET /t/<topic>/peek?n=`; dashboard
  uses the capability internally regardless.

## Don't do (0)

Nothing rejected in Phase 2. (Whole-product alternatives were rejected
in Phase 1 — see SCOPE.md build-vs-buy record.)
