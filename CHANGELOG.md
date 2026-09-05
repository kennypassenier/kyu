# Changelog

Notable changes per release. Format loosely follows Keep a Changelog;
versions follow semver, where the **public interface** means the HTTP
contract (AR2) — the three verbs, their parameters and their response
shapes — plus the environment variables. The dashboard's HTML is not part of
it, and neither is the on-disk schema, which migrates forward on its own.

Releasing as **1.0.0** rather than 0.x is a deliberate promise, made by Kenny
at the Phase 9 gate: that interface is settled, and breaking it means 2.0.0.

## [Unreleased]

Nothing since 2.4.0.

## [2.4.0] — 2026-09-05

### Changed

- **Bootstrap is gone; the dashboard wears @kp-soft/themes' own components**
  (W13). kp-themes v3.0.0 ships twenty-two framework-free components,
  themed natively — the eighteen kyu's templates needed (button, badge,
  card, alert, table, nav, form field) replace `bootstrap.min.css`
  (233 KB) and the 4 KB bridge that translated between the two. Nothing
  in `templates/` still names a Bootstrap class.
- **Eight files are vendored now, not six**: `components.js` carries the
  DI10/DI4 contract enforcement kyu used to hand-roll (a destructive
  control needs an undo or a confirmation; a semantic colour needs
  words too) and the skip-link behaviour; `strings.js` is a hard
  dependency both `theme-picker.js` and `components.js` import since the
  package's own 2.0.0. `static/kyu-init.js` is new and kyu's own: since
  3.0.0 every `js/*.js` import is pure, so something has to call
  `attachThemePickers()`, `enforceContracts()`, `attachConfirmations()`
  and `attachSkipLinks()` once the markup exists — kyu calls only those
  four, not the package's own `js/auto.js`, which would attach twelve
  behaviours this dashboard has no markup for (a data table, a date
  picker, drag-reorder…).
- **Revoking an app token now arms before it acts** (DI10): a first
  click changes the button to "Really revoke?", a second click within
  four seconds does it, and the button disarms itself if nothing
  follows. The same mechanism that used to be kyu's own thirty-line
  script is now three lines calling the vendored one.
- **A skip link.** The first thing a keyboard user meets on every page is
  now "Skip to content", not the theme picker.
- **Theme labels are English**, matching the rest of this dashboard and
  the package's own default since its 2.0.0 — they were Dutch before,
  left over from a package that was Dutch-by-default when this list was
  first written.
- **The login page joined the theme system.** Until this release it was
  the one page on kyu with no theme picker, no dark mode and no
  no-flash snippet — plain Bootstrap regardless of what a visitor had
  chosen everywhere else. It still offers no picker of its own (nothing
  to switch to before logging in), but it now renders in whatever theme
  is already stored.

### Fixed

- **A required field no longer renders red before anyone has touched
  it.** `components.css`'s own `input:invalid` rule matches an empty
  required field from the moment the page loads, which is correct CSS
  and wrong UX for a page not using the package's `attachForms()` (kyu
  does not — see below). `static/kyu.css` narrows this to `:user-invalid`,
  which only matches once a field has been interacted with and left
  invalid.

### Not carried over, on purpose

- **`js/forms.js`** (the package's accessible validation summary) is not
  vendored. kyu's forms are simple enough that the server's own
  `{% if error %}` banner already covers it, and — see below — the
  v3.0.0 release's own `SHA256SUMS` does not list this file, so it
  cannot be verified against the release the way the other eight are.
- **TH63's light/dark theme-menu grouping** is not adopted. It needs
  each theme's `dark` flag, which kyu deliberately removed from its own
  side in 1.0.0 (a hand-kept copy of that flag is exactly how kyu once
  came to believe in four dark themes when there are three); reading it
  from the JS registry to build a grouped menu would mean building the
  menu in JavaScript, which this dashboard's server-rendered picker
  exists to avoid.

### Worth knowing, not kyu's to fix

- **kp-themes' own v3.0.0 release `SHA256SUMS` omits `js/strings.js` and
  `js/forms.js`.** `strings.js` is not optional here — both
  `theme-picker.js` and `components.js` import it, so kyu could not
  serve either without it. Its hash in `KP_THEMES.sha256` is computed
  from the v3.0.0 git tag rather than lifted from the release manifest,
  which is the offline-verifiable next best thing but proves less: it
  catches an edited or truncated copy, not that the copy is genuinely
  the tagged release.

Nothing about the HTTP contract, the environment variables or the
deployment changed. Upgrading is replacing the binary.

## [2.3.2] — 2026-09-04

### Changed

- **The vendored kp-themes files move from v1.0.0 to v1.2.0** (W13). Nothing
  visible changes, and that was measured rather than assumed before the copy
  was made: `themes.css` differs in exactly one line — the version banner —
  so the eleven palettes are byte-identical; `theme-picker.js` is identical;
  `theme-core.js` and `theme-registry.js` gained only the JSDoc types of
  v1.1.0, which kyu does not read. `components.css` grew from 21 KB to 43 KB
  with v1.2.0's twenty-two components, purely by addition — the old file is a
  byte-exact prefix of the new one, and the five classes kyu uses sit
  untouched.

- **The no-flash snippet comes from the package now.** kyu hand-wrote the six
  lines that put the visitor's stored theme on `<html>` before first paint;
  v1.2.0 ships them as `NO_FLASH_SNIPPET`, so the copy is gone. kyu still
  inlines the text rather than importing the module — a module arrives too
  late to prevent the flash it exists to prevent — but the text now has one
  source, and a test compares what the document head inlines against the
  vendored file. It was shown red twice before being trusted: once on a
  single stray character, once with the snippet moved below the stylesheet
  link, where it is the right text at the wrong moment.

  The package's own comment on that file names kyu as the consumer whose
  home-grown copy grew a list of which themes are dark and had it wrong.

Nothing about the HTTP contract, the environment variables or the deployment
changed. Upgrading is replacing the binary.

## [2.3.1] — 2026-09-04

### Fixed

- **`data-bs-theme` is set before first paint, not after it.** The script
  that makes Bootstrap's own components follow the active theme sits after
  the stylesheet links — where the theme's `color-scheme` is already
  readable — but waited for `DOMContentLoaded` anyway. Under a dark theme
  that left a flash of light Bootstrap chrome on load.

  Found by the almanac session hitting the mirror image while adopting the
  same package: their read ran *before* the links and got `normal`. Between
  the two, the whole window is accounted for.

## [2.3.0] — 2026-09-04

### Changed

- **The themes come from the package's own picker now** (W13). kp-themes
  v1.0.0 ships the framework-free channel kyu asked for, so the switcher
  behaviour kyu hand-wrote in 2.2.0 is gone: five files are vendored verbatim
  and the module attaches to markup kyu's server writes. One implementation
  of that behaviour instead of two, in the project that owns it.

- **Eleven themes instead of seven.** high-contrast, sepia, blueprint and
  solstice join formal, light, dark, cyberpunk, pastel, terminal and topo.

- **Swatches wear the theme they preview.** v1.0.0 removed the colour copies
  that used to live in JavaScript, so a swatch reads the live custom
  properties instead of a duplicate that drifts when a palette is adjusted.
  kyu's own theme list shrank to names and labels for the same reason.

Nothing about the HTTP contract, the environment variables or the deployment
changed. Upgrading is replacing the binary.

## [2.2.0] — 2026-09-02

### Added

- **The house themes** (W13). The seven themes from `@kp-soft/themes` —
  formal, light, dark, cyberpunk, pastel, terminal, topo — with a picker in
  the navbar and the choice remembered in the browser. Same contract as
  JobTracker and kp-soft: the `theme` key in `localStorage`, `data-theme` on
  `<html>` plus the `dark` class, `formal` as the default. A theme chosen in
  one of these apps behaves the same way in the next.

  kyu has no npm and no build step, so `css/themes.css` is vendored verbatim
  with its version and commit recorded at the top, and the React switcher's
  behaviour is reimplemented in plain JavaScript. The mapping onto Bootstrap
  lives in a separate file so re-copying the upstream one never overwrites
  it.

### Fixed

- **The Generate token button lines up with the field again.** The row was
  bottom-aligned while the field's column also held its hint text, so the
  button sat level with the bottom of the hint rather than with the input.

## [2.1.0] — 2026-09-02

Both items come from the homelab, which is adopting CT 109 as a supervised
native service and found kyu the odd one out of four.

### Added

- **A graceful stop** (W12). kyu caught no signals at all, so
  `systemctl stop kyu` ended the process where it stood. Nothing about
  durability depended on that — ten SIGKILLs in a row still need no repair,
  which is what `l5_crash.rs` proves — but it cost three things worth
  having: in-flight requests were cut off mid-response, systemd recorded
  every stop as "killed by signal", and the files on disk never stood still,
  so the homelab's nightly `tar` of the data directory failed with
  `kyu.db-wal: file changed as we read it`.

  SIGTERM and Ctrl-C now stop the hub politely: requests in flight are
  answered, the write-ahead log is folded back and truncated, and the
  process exits 0. Bounded by **`KYU_SHUTDOWN_TIMEOUT_MS`** (default
  10000); blowing the budget logs one loud line and exits 0 anyway, because
  a stop that hangs is worse than one that is incomplete. Further signals
  during shutdown change nothing.

- **The release carries the binary.** Every `v*` tag now attaches `kyu` and
  a `SHA256SUMS` to its GitHub Release alongside the image, so
  `homelab install-native` can fetch it, verify the checksum on the desktop
  and push only verified bytes into the container. Before this, updating the
  native deployment on LXC 109 meant copying a file by hand.

  The binary is **extracted from the image the same workflow just built**
  rather than compiled a second time: cargo runs exactly once, so what you
  download is byte-for-byte what runs in the container. Two build paths
  drift; one cannot.

### Upgrading from 2.0.x

Nothing is required — the new variable has a working default and the HTTP
contract is untouched. Two things are worth doing on a systemd host:

1. Add `TimeoutStopSec` to the unit, comfortably above
   `KYU_SHUTDOWN_TIMEOUT_MS`, so systemd's patience is never what ends a
   shutdown.
2. If a backup tars the data directory, it can now stop the service first
   and get files nothing is rewriting. Backing up `POST /api/backup` output
   while the hub runs remains the alternative that needs no downtime.

## [2.0.0] — 2026-08-29

### The project is now called kyu

It was called `mailbox`, and that name said email. This has never been
email: nothing is sent anywhere, nothing is forwarded, no mail protocol is
spoken. What it does is hold a message until a consumer comes asking and
confirms it handled it — which is a queue, not a postbox.

**kyu** (級) is the Japanese word for a rank or grade: the position that
says when your turn comes. It is pronounced *queue*, which is what the
thing is.

Everything moved, deliberately. A half-renamed system asks you to remember
two names instead of one, and the old name would have outlived every
document explaining it.

- Crate, binary, repository, image and container: `mailbox` → `kyu`.
- **Environment variables** — every `MAILBOX_*` is now `KYU_*`
  (`KYU_TOKEN`, `KYU_SECRET_KEY`, `KYU_DATA_DIR`, `KYU_LISTEN`, …).
- **Response headers** — `mailbox-id`, `mailbox-topic`, `mailbox-attempt`,
  `mailbox-published-at` and `mailbox-notice` are now `kyu-*`.
- **Metrics** — `mailbox_messages`, `mailbox_deliveries`, `mailbox_topics`,
  `mailbox_subscriptions`, `mailbox_store_bytes` and
  `mailbox_sweeper_age_ms` are now `kyu_*`.
- **Session cookie** — `mailbox_session` → `kyu_session`.
- **The hub's own events** — the topic `mailbox.events` is now
  `kyu.events`, and the reserved topic prefix is `kyu.` instead of
  `mailbox.`.
- **On disk** — `/var/lib/mailbox` → `/var/lib/kyu`, `/etc/mailbox/` →
  `/etc/kyu/`.

### Upgrading from 1.x

The stored data needs no migration: the schema never carried the name, so
the same SQLite file opens unchanged under the new binary. What has to
change is everything that speaks to the hub.

1. Move the store and the configuration: `/var/lib/mailbox` →
   `/var/lib/kyu`, `/etc/mailbox/mailbox.env` → `/etc/kyu/kyu.env`.
2. Rename the variables inside that env file (`MAILBOX_` → `KYU_`).
3. In every consumer, rename the response headers it reads and the topic
   `mailbox.events` if it subscribes to it.
4. Point monitoring at the new metric names before you retire the old
   dashboards, or the graphs go quiet without anything being wrong.

### Why this is 2.0.0 and not 1.2.0

The definition at the top of this file counts the environment variables as
part of the public interface, and the Phase 9 promise was that breaking it
means a major version. Renaming every one of them breaks it. The three
verbs, their parameters and their response *shapes* are unchanged — only
the names on the outside moved.

## [1.0.1] — 2026-08-28

### Fixed

- **The command line no longer fails open.** Every argument except
  `--healthcheck` was ignored, so `kyu --version` printed nothing and
  started the hub instead — found while deploying 1.0.0 onto its LXC, where
  the invocation simply never returned. Unknown flags and stray positional
  arguments are now refused with exit code 2 and a remedy, and `--version`
  and `--help` exist. The risk this closes is not the missing flag: it is a
  typo in a unit file or deploy script starting a second hub on the same
  store instead of complaining (standing rules 11 and 12).

`--help` documents the environment variables, because someone reaching for
`--help` is usually looking for the configuration and there is no config
file to find.

## [1.0.0] — 2026-08-28

First release. Everything below was built between 2026-08-12 and 2026-08-28
under the staged procedure in `~/Projects/dev-procedure`; the reasoning lives
in `docs/`, not here.

### The contract

- **Publish, long-poll receive, acknowledge** over plain HTTP (K1, K2, K3).
  Payload and content type stored verbatim; a confirmed publish survives a
  hard kill.
- **Subscription names carry both delivery patterns** (K4): different names
  each receive every message, the same name from several processes competes.
- **Two response shapes** (AR2): raw body with `Kyu-*` headers by default,
  `?envelope=json` for scripts — JSON embeds as JSON, text as `payload_text`,
  anything else as `payload_base64`, never silently mangled.
- **Every failure answers `{error, remedy}`** (AR4), including the ones a
  framework would otherwise reject with bare text.

### Reliability

- **Leases, redelivery and dead letters** (K5, K6): at-least-once delivery, a
  background sweeper that returns expired claims with a counted attempt and a
  linear backoff, and a visible dead-letter list that survives restarts and
  can be requeued with a fresh set of attempts.
- **Per-subscription policy** (K7): lease, attempts, backoff, TTL and the idle
  thresholds, each defaulted so a fresh subscription needs no configuration.
- **`nack`** (W5), with `?dead=true` to send a poison pill straight to the
  dead-letter list.
- **Crash safety** (K12): WAL with `synchronous=FULL`, asserted rather than
  assumed; ten hard kills in a row need no manual repair to restart.

### History and housekeeping

- **Retention** (K9) that never collects a backlog an active subscription
  still needs, per topic or hub-wide, with `never` to keep forever.
- **Replay** (K8) via `?from=beginning`, idempotent and in bounded batches.
- **Idle lifecycle** (K11): a quiet subscription is flagged, then archived,
  settling what it held as `lapsed` rather than dropping it silently.
- **`kyu.events`** (W11): the hub publishes its own events as ordinary
  messages, so consuming them needs no special integration.
- **Delayed delivery** (W4) via `?delay=` or `?at=`, durable on acceptance.

### Operating it

- **Dashboard** (K10) that doubles as the documentation: every topic page
  prints working commands built from its own data.
- **Health** (W6) that reports what can actually be broken while the process
  still answers, and **metrics** (W1) including the sweeper age, which is what
  makes a stalled sweeper alertable instead of invisible.
- **JSON logs** (W7) and an **online backup** (W8) that is opened and
  integrity-checked before it is reported as written.
- **Shared-token auth** (W2): a door over the verbs and the dashboard, per-app
  tokens managed from the dashboard and encrypted at rest, a login page, and
  copy-paste commands that carry a real token while showing a masked one.
  `/healthz` and `/metrics` stay open so monitoring keeps working.
- **Distribution** (K13): a `v*` tag publishes a Docker image to GHCR;
  `presets/kyu/` in the homelab repo deploys it like any other app.

### Deliberately not included

No exactly-once, no routing or transformation, no clustering, no user
management, never exposed to the internet. See `docs/SCOPE.md` (N1–N6).

### Known limitations at release

- ~~The tag → image path has **never run**.~~ It ran on this tag and was
  verified before the release was published: the image pulls anonymously
  after `docker logout ghcr.io`, answers `/healthz`, refuses a tokenless
  publish with 401, and carries a message through publish → receive → ack.
- The dashboard's copy button is not covered by an automated test — both
  clipboard paths need a real click in a focused window. What is tested is
  that the command it copies is the real working one.
- `/metrics` is open by design, so anyone who can reach the hub can learn
  topic and subscription **names** and their counts. Never a payload.

### Verified after release, on real hardware

Deployed from the published image into a temporary LXC on the Proxmox host
(2026-08-28), then destroyed: publish → receive → ack round-trips, an
unacknowledged message comes back after a container restart, `/healthz` and
`/metrics` answer without a token while the dashboard redirects to its login
page, the static assets are inside the image, and `/static/../etc/passwd`
resolves to nothing. Resident memory under that load: **1.9 MiB**.
