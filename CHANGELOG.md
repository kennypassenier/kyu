# Changelog

Notable changes per release. Format loosely follows Keep a Changelog;
versions follow semver, where the **public interface** means the HTTP
contract (AR2) — the three verbs, their parameters and their response
shapes — plus the environment variables. The dashboard's HTML is not part of
it, and neither is the on-disk schema, which migrates forward on its own.

Releasing as **1.0.0** rather than 0.x is a deliberate promise, made by Kenny
at the Phase 9 gate: that interface is settled, and breaking it means 2.0.0.

## [Unreleased]

Nothing since 1.0.1.

## [1.0.1] — 2026-08-28

### Fixed

- **The command line no longer fails open.** Every argument except
  `--healthcheck` was ignored, so `mailbox --version` printed nothing and
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
- **Two response shapes** (AR2): raw body with `Mailbox-*` headers by default,
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
- **`mailbox.events`** (W11): the hub publishes its own events as ordinary
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
  `presets/mailbox/` in the homelab repo deploys it like any other app.

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
