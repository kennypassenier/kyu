# mailbox — Architecture decisions

Decisions T1–T9 (tech choice) were taken at the Phase 3 gate on
2026-08-12; AR1–AR11 (architecture) were **FROZEN 2026-08-12** at the
Phase 4 gate after an architecture-critic attack (12 objections: 3
blockers, 7 serious, 2 minor — all resolved by amendment or explicit
decision below; AR4/AR6/AR8 survived unchanged). Changes only via
mini-rounds, recorded as dated amendments.

## T1 · Web framework: axum

axum 0.8.x (tokio team, MSRV 1.80, API stable since Dec 2024).
Handlers are plain async fns, so K2 long-polling is an awaited
notification with a timeout — no framework ceremony. Same maintainers
as the runtime (T3) keeps the async stack one family.
Rejected: actix-web 4.14 (healthy, marginally faster, no edge at
homelab load, aggressive MSRV).

## T2 · Storage: SQLite via rusqlite

rusqlite 0.40.x with the `bundled` feature (compiles SQLite 3.53 into
the binary; clean musl static builds). WAL mode with
`synchronous=FULL` — explicitly set, never default-trusted — is the
K12 contract: confirmed means fsynced. All engine state (messages,
cursors, leases, dead letters) lives in one database so every delivery
transition is a single SQL transaction. Confirms the Phase 1 lean.
Rejected: redb 4.1 (pure-Rust supply chain, but key-value only —
hand-rolled indexes and claim logic for no functional gain).

## T3 · Async runtime: tokio

tokio 1.x. async-std is discontinued (Mar 2025); smol has a minimal
ecosystem; axum requires tokio anyway. Recorded nuance: rusqlite is
synchronous, so store access runs on the blocking pool
(`spawn_blocking`) behind the storage module boundary — a deliberate
pattern, to be tested, not an accident.

## T4 · Dashboard rendering: minijinja + htmx 2.x

*(Kenny's pick over the askama recommendation.)* minijinja 2.x
(runtime Jinja2 templates, very active) renders the K10 dashboard
server-side; htmx 2.0.x (stability-only line, "supported in
perpetuity"; the 4.0 rewrite is explicitly ignored) provides
auto-refreshing counts and the W9 test-publish form. Template files
ship embedded in the binary (via include/embed) so the container
stays one artifact. Trade-off accepted: template errors surface at
runtime rather than compile time → Phase 7 must include
render-every-template-with-seeded-state tests to compensate.
Rejected: askama (compile-time safety, but Kenny prefers template
iteration without recompiles); embedded SPA (second toolchain, npm
churn — the re-entry tax this project exists to avoid).

## T5 · Logging: tracing

tracing + tracing-subscriber 0.3.x. Structured fields (topic,
subscription, message id) on every event; W7's JSON output is the
built-in `fmt().json()` layer; pretty output in dev.
Rejected: log + env_logger (string-based; W7 would be hand-rolled).

## T6 · Dependency policy: reluctant, policed

New direct dependencies need a one-line justification in the commit
that adds them; prefer std/existing deps; `Cargo.lock` committed;
cargo-deny in CI (advisories, license allowlist, duplicate bans) plus
a weekly scheduled advisory job. Expected direct-dep count for the
Essential set: order of ten.

## T7 · License: MIT OR Apache-2.0, public repo

*(Kenny's pick over the MIT-only recommendation.)* Dual license per
Rust-ecosystem convention (Apache adds the patent grant). Public
GitHub repo and public ghcr.io image — the LXC pulls with zero
credentials (no PAT to rotate; standing rule 10 favours fewer
standing secrets).

## T8 · Toolchain: edition 2024, track stable

Edition 2024 (settled since 1.85). Develop and CI on latest stable
(1.95 at decision time). `rust-version` declared in Cargo.toml at
whatever stable is at L0; bumped freely with a changelog note — we
build our own container, nobody else's compiler matters.
Rejected: N-2 MSRV window (buys nothing for a self-deployed binary).

## T9 · Container base: distroless/static

Multi-stage build → static musl binary → `gcr.io/distroless/static`
(~2 MB: CA certs + nonroot user included). The W6 Docker healthcheck
uses a `--healthcheck` flag on the mailbox binary itself, since the
image has no shell. Rejected: scratch (hand-rolled certs/nonroot for
2 MB), alpine (a shell and package manager the container doesn't
need).

---

# Architecture (AR1–AR11) — frozen 2026-08-12

## AR1 · Module layout: core/shell split

One Cargo package: `mailbox` lib + thin `main.rs`.

- `engine/` — ALL delivery semantics (publish, claim, ack, nack,
  redelivery, DLQ, TTL/retention/idle/due-time transitions). Pure
  logic: no tokio, no HTTP, no ambient wall clock — time enters via
  the `Clock` trait (AR7), storage via the `store` API.
- `store/` — SQLite: schema, migrations, all SQL. Module boundary, not
  a swappable-engine trait (swappability lives at the HTTP contract,
  C2); tests build in-memory databases through it.
- `http/` — axum routes; HTTP ⇄ engine translation only. Long-poll
  wait loops live here.
- `dashboard/` — minijinja rendering + htmx fragment endpoints;
  read-only over engine/store, plus the W9 publish call.
- `events/` — W11: engine emits typed events; this module publishes
  them onto `mailbox.*` topics through the normal publish path.

**Loop-breaker (amendment, critic objection 12):** events *about*
`mailbox.*` topics are logged only, never re-published as events —
otherwise a broken consumer of `mailbox.events` dead-letters, which
emits a dead-letter event onto the same topic, which dead-letters, ad
infinitum. Dedicated test required.

Rationale: the mocked-clock suites (K5/K7/K9/K11) require injected
time; core/shell keeps tokio and axum out of semantics tests.

**Interpretation ratified 2026-08-12 (L0 gate).** The five modules above
are the *domain* modules. Shell concerns may live beside them: `config`
is its own module (so AR6 parsing is testable without mutating the
process environment) and tracing setup lives in `main.rs`. Domain
primitives stay inside the frozen modules — the `Clock` (AR7) and the id
generator live under `engine`, the schema and migrations under `store`.

## AR2 · HTTP contract: raw body default + JSON envelope opt-in

*(Decided in the AR2 deep-dive round.)*

- `POST /t/{topic}` — publish; body stored verbatim. 201 →
  `{"id":"<ulid>"}`. Query `delay`/`at` (W4).
- `GET /t/{topic}/next?as={sub}` — long-poll claim.
  - Default: 200 with the payload as the response body, verbatim,
    metadata in headers (`Mailbox-Id`, `Mailbox-Topic`,
    `Mailbox-Attempt`, `Mailbox-Published-At`, stored `Content-Type`).
  - `&envelope=json`: 200 with
    `{"id","topic","attempt","published_at","content_type","payload"}`;
    JSON payloads embed as JSON, non-JSON/binary payloads appear as
    `"payload_base64"` — flagged, never silently mangled (G8).
  - 204 after `wait` seconds (default 30, max 300), identical in both
    modes. `from=beginning` on first call (K8).
- `POST /t/{topic}/ack/{id}?as={sub}`; `POST /t/{topic}/nack/{id}?as=
  {sub}[&dead=true]` (W5).
- Admin/observability: `GET /api/topics`, `GET /api/t/{topic}`,
  `GET|PUT /api/t/{topic}/subs/{sub}/policy`, DLQ list/requeue,
  archive/unarchive, `GET /healthz` (W6), `GET /metrics` (W1).

Payload-as-body is the contract: `curl | jq` on the payload works and
binary survives. The envelope exists because header-parsing in shell
(`-D-`, CR stripping, case-insensitive match behind Traefik/HTTP-2) is
real friction against S1 — so the K10 dashboard renders BOTH blessed
snippets per topic (raw two-liner and envelope one-liner), and both are
tested. Rejected: envelope-always (permanent base64 for binary, kills
payload fidelity); headers-only (leaves the friction unaddressed).

**Content-type rule (amendment, objection 5):** `curl -d` silently
sends `application/x-www-form-urlencoded`; mailbox stores what it is
sent, verbatim, so every rendered example and doc snippet carries an
explicit `-H 'content-type: …'`. The dashboard sniffs content for
*display* only, never rewriting stored metadata.

## AR3 · Storage: materialized fan-out, backlogs win over retention

Tables (all times integer unix-millis UTC):

- `topics(id, name UNIQUE, retention_ms, created_at)`
- `subscriptions(id, topic_id, name, state
  [active|flagged|archived], lease_ms, max_attempts, backoff_ms,
  ttl_ms NULL, created_at, last_poll_at, UNIQUE(topic_id, name))`
- `messages(rowid INTEGER PK AUTOINCREMENT, id TEXT UNIQUE /*ULID*/,
  topic_id, payload BLOB, content_type, published_at, due_at /*W4*/)`
- `deliveries(msg_id, sub_id, state
  [pending|claimed|acked|dead|expired|lapsed], attempts,
  lease_expires_at, next_attempt_at, dead_at, expired_at,
  PRIMARY KEY(msg_id, sub_id))`

Fan-out is MATERIALIZED: publish inserts one `deliveries` row per
active subscription in the same transaction as the message. Claim is
one state-guarded `UPDATE … RETURNING` on the oldest deliverable row
(ordered by `messages.rowid`, AR7). Every transition is a single
transaction; invariants are checkable with one `SELECT`.

**Amendment (blocker 1):** `expired` (+`expired_at`) and `lapsed` are
part of the enum — AR9's TTL transition had no representable state.
The past-max-attempts → `dead` check happens on the claimed→pending
transition and in the sweeper, so no row can idle in `pending` with
attempts > max.

**Amendment (blocker 2):** `from=beginning` (K8) requires a named
**backfill** operation: on a subscription's first poll with that flag,
deliveries rows are retro-inserted for retained messages in bounded
batches (AR5), skipping messages the retention sweep removes mid-run
(no FK dangling, no delivery for a vanished message). K8's boundary
test asserts "replay sees exactly what retention kept".

**Decision (blocker 3) — backlogs win:** retention (K9) never deletes a
message that still has a pending or claimed delivery on an **active**
subscription. The pressure valve is K11: archiving a subscription
settles its outstanding deliveries as `lapsed` (recorded, counted,
dashboard-visible), after which retention may collect the message.
Unbounded growth is therefore bounded by the idle-archive timeline, and
the dashboard shows oldest-unacked age per subscription. Rejected:
"retention wins" (predictable disk, but a slow consumer silently loses
real backlog on short-retention topics — G4/G8 violation in spirit).

Schema versioning: `PRAGMA user_version`, forward-only numbered
migrations embedded in the binary, applied in a transaction at startup
(AR10). Pragmas set explicitly at every open — never inherited from
build defaults: WAL, `synchronous=FULL`, `foreign_keys=ON`,
`busy_timeout`.

## AR4 · Error model *(critic-cleared, unchanged)*

Typed errors (`thiserror`) in engine/store; `anyhow` with context at
binary edges; no panics on reachable paths (a panic is a bug and
S4-relevant). Every HTTP error body is `{"error":"…","remedy":"…"}` —
remedy mandatory (standing rule 11), e.g. *"subscription 'pritner'
unknown on topic notify.kenny; existing: [ha-forwarder]; subscriptions
are created by polling"*. 4xx = caller mistake with the exact fix; 5xx
= genuine internal failure, always logged at ERROR.

## AR5 · Concurrency: one writer, notify-after-commit, bounded work

- Single process (N1). Reader pool + ONE dedicated writer connection
  (SQLite WAL), all store calls via `spawn_blocking`.
- Long-poll wakeups: per-subscription `tokio::sync::Notify`, fired
  after commit. Correctness never depends on wakeups — claim races are
  resolved by the store transaction; a missed wakeup costs latency
  only.
- ONE sweeper task: lease expiry, TTL, retention, idle-flagging, W4
  due-time promotion — through the same engine functions the
  mocked-clock tests exercise.

**Amendments (objections 6, 7):** every sweep and backfill runs in
bounded batches (`LIMIT N` per transaction, N≈500) so the single writer
is never blocked for seconds and a SIGKILL mid-sweep cannot replay a
giant rollback; sweeper tick ≤ 1 s; poller re-check interval ≤ 5 s; the
sweeper fires a subscription's `Notify` whenever it re-pends or
promotes a message, so redelivery does not wait for the next poll.

## AR6 · Configuration: environment only *(critic-cleared)*

`MAILBOX_DATA_DIR`, `MAILBOX_LISTEN`, `MAILBOX_MAX_BODY_BYTES`, log
level/format, plus global *defaults* (idle-flag/archive thresholds,
default retention). Only what the process needs before it can open the
store; all per-topic/per-subscription policy (K7, K9) lives in the
database, set via API/dashboard. Rejected: a TOML file (a second place
to look).

## AR7 · Time and identifiers

- Public message id: ULID, generated monotonic-clamped (never behind
  the previous one).
- **Amendment (objection 8):** delivery and claim ORDER use
  `messages.rowid` (AUTOINCREMENT insertion order), not the ULID —
  after a power cut a host can boot with a skewed RTC and NTP steps the
  clock backwards, which would make new ULIDs sort before pre-outage
  messages and break S3's in-order drain.
- API timestamps RFC3339 UTC; DB integer unix-millis.
- `Clock` trait injected into engine and sweeper: `SystemClock` in
  production, `MockClock` in the K5/K7/K9/K11 suites.

## AR8 · Names, limits, reserved space *(critic-cleared, unchanged)*

Topic and subscription names `^[a-z0-9._-]{1,64}$`; dots namespace
(`notify.kenny`, `jobs.transcode`). `mailbox.*` reserved for system
topics (W11); external publish there → 403 with remedy. Payload cap 1
MiB default (env-overridable) → 413 with remedy. `wait` ≤ 300 s,
default 30 s.

## AR9 · Delivery state machine (pinned artifact)

```
pending  --claim-->        claimed
claimed  --ack-->          acked
claimed  --lease expiry | nack-->  pending  (attempts+1,
                                    next_attempt_at = now + backoff)
pending  --attempts > max_attempts-->  dead      (K6)
pending  --past TTL-->                 expired   (recorded, K7)
dead     --manual requeue-->           pending   (attempts reset)
active sub archived (K11) --> outstanding deliveries lapsed (AR3)
```

**Decision (objection 9) — expire on re-pend:** whenever a delivery
returns to `pending` (lease expiry or nack), it is TTL-checked against
the message's publish time first; past TTL → `expired`. Without this, a
10-minute-TTL TTS subscription whose worker hangs 25 minutes would
still speak a half-hour-old doorbell announcement. Rejected: "deliver
anyway" (a message that consumed retry effort deserves delivery —
honest, but breaks "relevant now or never").

Every transition above gets a test named after it. Illegal transitions
are unreachable by construction: each is one UPDATE guarded by the
current state in its WHERE clause.

## AR10 · Updates and migrations

No self-updater: updates are image pulls via compose (K13). At startup
forward-only migrations apply inside a transaction; opening a schema
NEWER than the binary knows is refused with a remedy.

**Amendment (objection 10):** before applying any migration the binary
snapshots the store (`VACUUM INTO mailbox.pre-v{N}.db`, last two kept)
— otherwise a bad image migrates the schema, misbehaves, and rolling
the image tag back hits "refuse newer schema" with no snapshot to
return to. Rollback = previous image + snapshot file, documented as a
numbered runbook procedure (Phase 8). The Docker healthcheck
start-period must exceed worst-case migration time so a restart loop
cannot kill a migration mid-flight.

## AR11 · Security model and payload display

- LAN threat model (N3); no auth in v1 (W2 = Later, additive). Binding
  and exposure are compose/LXC concerns.
- Payloads are UNTRUSTED wherever rendered: minijinja autoescape ON,
  payloads rendered as text only, htmx fragments escaped — a malicious
  message must not be able to script the dashboard (stored XSS via
  queue).
- **Amendment (objection 11):** display = lossy UTF-8 decode with a
  hard cap (4 KiB shown) and a visible marker — *"truncated — N of M
  bytes"* / *"binary payload (N bytes)"* (G8: no silent truncation).
  W9 prefills text payloads only.
- mailbox stores no secrets today; if W2 lands, token via env only,
  never logged, with a mandatory plaintext-scan test.
- SQL exclusively parameterized (rusqlite params); no string-built SQL
  anywhere, enforced by review plus a grep gate in CI.
