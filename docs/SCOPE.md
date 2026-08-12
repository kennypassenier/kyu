# mailbox — Scope

Approved via the Phase 0 gate on 2026-08-12. Every statement below was
individually approved by Kenny (Klopt), amended items are marked.

## The project in one sentence

A self-documenting post office for the homelab — any script can send
with one curl, any worker can receive and ack with two, nothing is ever
silently lost, and the dashboard teaches you your own system every time
you come back.

## Goals

- **G1 · Core mission.** One self-hosted service ("the hub") that gives
  every homelab service — and all future software — durable,
  topic-based messaging. Anything that speaks HTTP can participate (HA
  automations, bash scripts, cron jobs, future binaries). The hub runs
  24/7; messages handed to it are never silently lost.
- **G2 · Three-verb API.** The complete mental model is
  send / receive / ack over plain HTTP:
  - `curl -d '{"title":"Backup done"}' http://hub.lan/t/notify.kenny`
  - `curl http://hub.lan/t/notify.kenny/next?as=ha-forwarder` (long-poll)
  - `curl -X POST http://hub.lan/t/notify.kenny/ack/<id>?as=ha-forwarder`

  No client library, no AMQP concepts. Publishing to a topic that does
  not exist yet creates it.
- **G3 · Named subscriptions.** A consumer identifies itself with
  `?as=name`; first use auto-creates the subscription. Different names
  on one topic = fan-out (each receives every message, acks
  independently). The same name from multiple processes = load
  balancing (each message to exactly one worker, redelivered if it
  dies). Ack-counting on a shared queue was considered and rejected.
- **G4 · At-least-once delivery with visible dead letters.** A message
  stays in a subscription until acked. Crash or timeout → redelivery.
  Retries exhausted → dead-letter view on the dashboard (payload
  visible, one-click retry), never a log void.
- **G5 · The dashboard is the documentation.** One web page shows every
  topic, its subscriptions (backlog, last ack, alive/idle), recent
  messages, dead letters with retry — and per topic a copy-paste
  send/receive example rendered with a real recent payload.
- **G6 · Delivery policy is per subscription, not per topic.** TTL, max
  retries and backoff belong to the consumer. Example: on one
  `notify.kenny` topic a TTS subscription carries a 10-minute TTL while
  a printer subscription keeps messages forever. Sensible defaults
  apply when unconfigured.
- **G7 · New subscribers start from now.** A new subscription sees
  messages from its first appearance onward; `?from=beginning`
  explicitly opts into replaying retained history.
- **G8 · Nothing happens silently.** Idle subscriptions are flagged on
  the dashboard after a threshold and archived only after a longer one,
  with a notification. No silent caps, drops or fallbacks anywhere.

## Non-goals

- **N1 · No clustering or high availability.** One node, one container.
  Hub uptime is an Uptime Kuma check, not an engineering programme.
- **N2 · No multi-user auth or tenancy.** Single-admin tool. Whether a
  single shared token guards the API is a Phase 2 feature decision;
  user management is permanently out of scope.
- **N3 · Never exposed to the internet.** LAN/VPN only. No port
  forwarding, no public endpoints, no inbound webhooks from outside.
- **N4 · No exactly-once delivery.** The contract is at-least-once;
  consumers must tolerate occasional duplicates (idempotency).
- **N5 · No routing rules or message transformation.** The hub moves
  bytes; filtering/routing logic lives in consumers, as testable code.
- **N6 · No RabbitDispatcher migration.** Both legacy sources are dead;
  the old repo stays untouched as reference. The receipt printer is out
  of scope — if it returns, it is just a future consumer.

## Success criteria

- **S1 · The 5-minute re-entry test.** After ≥1 month away, using only
  the dashboard, a brand-new script sends and a second script
  consumes + acks in under 5 minutes. Flagship criterion.
- **S2 · A crashing consumer loses nothing.** Automated test: consumer
  killed before acking → message redelivered on next receive.
- **S3 · An offline consumer catches up in order.** Automated test: a
  subscription accumulates a simulated week of messages; the returning
  consumer drains the full backlog in publish order, without gaps.
- **S4 · The hub survives power loss.** Automated test: hub killed hard
  (SIGKILL) mid-traffic → after restart every confirmed publish is
  present and everything acked stays acked. Short- and long-outage
  cases are tested separately.
- **S5 · Fan-out subscriptions are truly independent.** Automated test:
  two subscriptions each receive every message and ack independently; a
  dead or slow subscription does not delay or affect the other.

## Hard constraints

- **C1 · Deployment target** *(amended at the gate)*: runs as a Docker
  container inside a Proxmox LXC. Development and testing run against
  scratch resources; touching the real container is always an agreed,
  explicit step.
- **C2 · The three-verb HTTP API is the contract; the engine is
  swappable.** Clients only ever see the G2 API. The engine underneath
  (RabbitMQ, NATS JetStream, embedded store) is chosen in Phase 1/3 and
  swappable without client changes.
- **C3 · Language** *(amended at the gate)*: **Rust**. Phase 3 decides
  libraries, dependency policy, MSRV — not the language. Standing rules
  apply throughout: English code/comments/docs, no paid services, no
  credit-billed tooling.

## Build-vs-buy record (Phase 1, decided 2026-08-12)

Answers Q1 from the Phase 0 gate ("does this lean on an existing
RabbitMQ server, or is the engine built in?"). Web research ran on
2026-08-12; all versions/sources verified then. Kenny's decision per
alternative:

| Alt | Candidate | Decision | Deciding factors |
|---|---|---|---|
| A1 | ntfy v2.27 (use as-is) | **Reject** | No acks, redelivery, DLQ or per-consumer state — fails G3/G4 |
| A2 | NSQ v1.3 (use as-is) | **Reject** | TCP-only consumption (fails G2), no server-side DLQ, memory-first durability (fails S4) |
| A3 | ElasticMQ (use as-is) | **Reject** | No fan-out (fails G3), SQS request plumbing (fails S1), JVM ~250–500 MB |
| A4 | RabbitMQ 4.3 as engine (hybrid) | **Reject** | ~150–250 MB Erlang appliance; no replay-from-beginning on queues (fails G7); lapin lacks auto-reconnect |
| A5 | NATS JetStream 2.14 as engine (hybrid) | **Reject** | No native DLQ (advisories are lossy — collides with G8), no per-subscription TTL (fails G6), Jepsen 2025: default fsync posture loses acked writes on power cut (fails S4) |
| A6 | Embedded engine in the Rust binary | **Build our own** | One binary fits S1; every scope feature implementable as specified; S4 durability fully in our control; SQLite-as-queue prior art is deep |

Also dismissed without a form item: beanstalkd (TCP-only, no fan-out,
dormant), Nchan (transport only), betterMQ (immature), sled
(abandoned).

Consequences: delivery logic (cursors, leases, redelivery, DLQ
transitions, TTL sweeps, long-poll wakeups) is ours to build and test —
S2–S5 are our tests, not a broker's. The store library (SQLite via
rusqlite vs redb) is a Phase 3 decision; early lean is SQLite in WAL
mode with `synchronous=FULL`. C2 keeps the engine swappable behind the
HTTP contract regardless.
