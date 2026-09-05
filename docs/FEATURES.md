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

**The apps page always exists now** *(fixed 2026-09-05, found live by Kenny
in the 2.4.0 preview)*. Until this fix the nav link was gated on
`protected` and `GET /apps` on an unprotected hub answered a bare
`{"error", "remedy"}` JSON body — so a hub started without `KYU_TOKEN` had
no visible way to discover that app management exists at all, only an
error nobody would see without already knowing to look for it. AR11's real
guarantee is unchanged: `POST /apps/create` and `/apps/revoke` still refuse
outright without a bootstrap token, because a per-app token only means
something once something already decides who may in. What changed is the
**page**: on an unprotected hub it now explains that, and hands over a
freshly generated `KYU_TOKEN`/`KYU_SECRET_KEY` pair — the same way the CLI
already prints one when refusing to start on a token without a key — ready
to paste into the compose file. Proven in `tests/p7_auth.rs`, shown red
against the pre-fix code before being trusted.

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

### W13 · The house themes *(added 2026-09-02, moved to the package's own picker 2026-09-04, Bootstrap replaced 2026-09-05)*

Kenny asked for the themes from `@kp-soft/themes` with *the same picker and
the same way of storing the choice in the browser*. Not a lookalike: the
same contract, so a theme chosen in one of his apps behaves identically in
the next.

**Since v1.0.0 that is no longer kyu's code.** The first implementation
(kyu 2.2.0) hand-wrote the switcher's behaviour, because the package only
shipped a React hook and a JSX component and kyu has no npm and no build
step. kyu and almanac were rebuilding the same thing separately; that was
raised with the package, and v1.0.0 ships a framework-free channel. kyu now
vendors it instead of maintaining a second implementation of someone else's
behaviour.

- **Eleven themes**: formal (the default), light, dark, cyberpunk, pastel,
  terminal, topo, high-contrast, sepia, blueprint, solstice.
- **The contract three projects share**: `localStorage` key `theme`, the
  theme names as values, `data-theme` on `<html>`, default `formal`.
- **Eight files are vendored verbatim** — `themes.css`, `components.css`,
  `theme-core.js`, `theme-picker.js`, `theme-registry.js`, `no-flash.js`,
  `components.js`, `strings.js` — byte-for-byte copies of the v3.0.0 tag,
  never edited here.
- **The no-flash snippet is the package's**, not kyu's. `no-flash.js` is the
  only vendored file the browser never fetches: kyu inlines its
  `NO_FLASH_SNIPPET` into `<head>`, because a module arrives too late to
  prevent the flash it exists to prevent. It is vendored so a test can compare
  what the head inlines against the package's own text.
- **kyu writes the markup, the package writes the behaviour.** The menu is
  rendered server-side because a menu built by JavaScript is an empty box on
  first paint, and this dashboard is server-rendered HTML. The vendored
  module attaches to the package's contract attributes
  (`data-kp-theme-picker`, `data-kp-theme`, `.kp-swatch`,
  `data-kp-theme-status`).
- **No colour copies anywhere.** v1.0.0 removed them on purpose: a swatch
  wears the theme it previews, reading that theme's live custom properties.
  The dark flag is gone from kyu's side too — the package derives it from
  each theme's own `color-scheme`, which is how kyu came to believe in four
  dark themes when there are three.
- **Bootstrap is gone (2.4.0).** The dashboard now wears the package's own
  components — button, badge, card, alert, table, nav, form field — which
  are themed natively, so `bootstrap.min.css` (233 KB) and the 4 KB
  `theme-bridge.css` that translated its `--bs-*` variables onto the
  package's tokens both left with it. `static/kyu.css` is kyu's own file
  now: layout glue, three badge-tone modifiers the package's `.kp-badge`
  deliberately does not ship (colour is only ever badges' second signal;
  DI4 says the word must already be there), and a `:user-invalid` override
  — see "Two things kp-themes' own 3.0.0 release left for kyu to solve"
  below.
- **DI10 in kyu's own markup**: revoking an app token now arms before it
  acts (`data-kp-destructive`, `data-kp-confirm`), using the vendored
  `components.js` instead of a hand-rolled confirmation. A skip link
  (`.kp-skip-link`) is new too, from the same file.
- **`static/kyu-init.js` is new and kyu's own.** Every vendored `js/*.js`
  import became pure at 3.0.0 — importing one attaches nothing. The
  package's own answer, `js/auto.js`, attaches sixteen behaviours; kyu's
  dashboard has markup for four of them (the theme picker, contract
  enforcement, confirmations, the skip link), so `kyu-init.js` calls only
  those, rather than paying for a data table and a date picker nothing on
  this dashboard uses.

**Keeping the copies honest.** Two checks in `.claude/hooks/gates.sh`, because
a copy fails in two ways and one check cannot see both.

- *Are they what we claim?* `static/KP_THEMES.sha256` holds the release's own
  checksums, mapped onto kyu's flat paths; a mismatch **refuses** the commit.
  This holds offline and pins the tag, so an edited copy and a copy taken from
  a working tree that had drifted past its tag both fail. Proven by appending
  one line to `no-flash.js` and watching it refuse. `strings.js`'s hash is the
  one exception: it is computed from the v3.0.0 git tag rather than lifted
  from the release's own `SHA256SUMS`, which omits it (see below) — still
  offline-verifiable, but it cannot prove tag provenance the other seven do.
- *Has the package moved on?* A comparison against `~/Projects/kp-themes`
  whenever that repository is on the machine — a **notice**, not a refusal,
  because being behind a release is a decision to make rather than a broken
  commit. Where it is absent (CI) it says so out loud rather than passing
  quietly.

The tests carry the third case, the one no checksum can see: that kyu's own
server-rendered side still agrees with the copies. `tests/w13_themes.rs`
compares the rendered menu's names *and* labels against the package's
generated registry, and the inlined snippet against `no-flash.js`. Each was
proven red on a deliberate injection before being trusted.

**No contrast gate here, on purpose** (Kenny, 2026-09-02). kp-themes runs
`check-contrast.mjs` before it tags, so these files have already passed it;
re-running it would mean pulling Node into a Rust pipeline to re-answer an
answered question, and it would only catch someone editing a vendored copy.
The risk a copy actually runs is staleness, and contrast says nothing about
that — the gate above guards the risk that is there.

Proven by `tests/w13_themes.rs`: the picker offers exactly the themes the
vendored registry defines and no others, the contract attributes are
present, the storage contract is read from the served registry rather than
from a literal, the whole ES module chain is reachable while the traversal
guard still holds (and Bootstrap and its bridge are confirmed gone, not
merely unreferenced), and every theme offered is defined in the stylesheet.

**Two things kp-themes' own 3.0.0 release left for kyu to solve, not the
other way round.**

1. The release's `SHA256SUMS` lists `theme-core.js`, `theme-registry.js`,
   `theme-picker.js`, `components.js`, `overlays.js` and `no-flash.js`, but
   not `strings.js` — and both `theme-picker.js` and `components.js` import
   it since the package's own 2.0.0. Without it neither module loads at
   all, which a browser console catches instantly and a Rust test never
   would (kyu's tests fetch the module text, not execute it). Vendored
   anyway, hashed from the tag rather than the manifest.
2. `components.css`'s `input:invalid` rule paints a destructive border on
   an empty required field from the moment the page renders — correct for
   a consumer using the package's `attachForms()` (which sets `novalidate`
   and reports on blur instead), wrong for one that is not. kyu does not
   vendor `js/forms.js` (also missing from the same `SHA256SUMS`, though
   kyu never needed it), so every required field on this dashboard would
   have rendered red on first paint without `kyu.css`'s `:user-invalid`
   override.

Neither is kyu's copy going stale; both are gaps in what the release itself
ships. Kenny decides whether either is worth raising with the project.

## Later (2)

- **W3 · Companion CLI** — `kyu send/tail/ls`; only if the
  curl+dashboard bet proves insufficient.
- **W10 · Public peek endpoint** — `GET /t/<topic>/peek?n=`; dashboard
  uses the capability internally regardless.

## Don't do (0)

Nothing rejected in Phase 2. (Whole-product alternatives were rejected
in Phase 1 — see SCOPE.md build-vs-buy record.)
