# mailbox — Features

**FROZEN 2026-08-12** via the Phase 2 gate (rating rounds 1+2, report
form R1–R3 all approved). Changes only via mini-rounds
(FORM_PROTOCOL §5), recorded here as dated amendments.

Tally: 15 Essential · 6 Desired · 3 Later · 0 Don't do.
IDs are permanent: they appear in commits, test names, docs and forms.

Ratings use the canonical English scale (Essential/Desired/Later/Don't
do); the forms rendered them in Dutch (Onmisbaar/Gewenst/Later/Niet
doen) per FORM_PROTOCOL.

## Essential (15)

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
`mailbox.events`. Unarchive is one click.
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
(start → publish → receive → ack → restart container → state intact);
tag→release→ghcr pipeline verified end-to-end with a pre-release tag.

### W6 · Health endpoint
`/healthz` returns 200 + small JSON (store writable, sweeps alive).
Feeds Uptime Kuma and the Docker healthcheck in K13's compose.
**Proven by:** healthy and degraded (read-only store → non-200) tests.

### W11 · `mailbox.events` system topic
The hub publishes its own noteworthy events as ordinary messages:
dead-lettered, subscription flagged/archived, TTL batch expired.
Consumed via K2 like any topic (e.g. HA automation → office light).
Rated Essential by Kenny (above Claude's Desired recommendation) — K6
and K11 announcements are built on it from the start.
**Proven by:** event-emitted assertions inside the K6/K11 suites.

## Desired (6)

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

## Later (3)

- **W2 · Shared-token auth** — optional `Authorization: Bearer`;
  additive, can land any time. Dashboard would render examples with
  token (S1 preserved). Plaintext-scan tests mandatory when built.
- **W3 · Companion CLI** — `mailbox send/tail/ls`; only if the
  curl+dashboard bet proves insufficient.
- **W10 · Public peek endpoint** — `GET /t/<topic>/peek?n=`; dashboard
  uses the capability internally regardless.

## Don't do (0)

Nothing rejected in Phase 2. (Whole-product alternatives were rejected
in Phase 1 — see SCOPE.md build-vs-buy record.)
