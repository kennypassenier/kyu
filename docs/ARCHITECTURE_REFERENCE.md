# kyu — architecture reference

The system as built, in Phase 8. `ARCHITECTURE_DECISIONS.md` records what was
decided and why, including the roads not taken; this file records what is
actually there, so you can navigate the code without reading all of it.

---

## Shape

```
HTTP (axum)                 src/http/
  ├── auth.rs               the door: bearer header or session cookie
  ├── csrf.rs               refuse cross-origin state changes
  ├── handlers.rs           verbs, dashboard pages, apps, login, static
  └── error.rs              every failure becomes {error, remedy}
        │
Engine                      src/engine/
  ├── mod.rs                publish / receive / ack / nack / policy / apps
  ├── clock.rs              injected time, so leases are testable
  ├── ids.rs                ULIDs that never go backwards
  ├── names.rs              what a topic or subscription may be called
  └── policy.rs             lease, attempts, backoff, ttl — resolved
        │
Store                       src/store/
  ├── mod.rs                one writer + a 4-connection reader pool
  ├── migrations.rs         forward-only, snapshot before migrating
  └── queries.rs            every SQL statement in the project
        │
SQLite (bundled, WAL, synchronous=FULL)

Background                  src/sweeper.rs    leases, TTL, retention, idle
Self-reporting              src/events.rs     kyu.events
Presentation                src/dashboard.rs + templates/ + static/
```

**The rule that keeps it navigable:** SQL lives only in `store/queries.rs`,
business logic only in `engine/`, HTTP shape only in `http/`. A handler that
starts reasoning about delivery is in the wrong file.

---

## The data model

Four tables plus one for apps. Times are integer milliseconds since the Unix
epoch. `STRICT` throughout, so SQLite enforces column types instead of
coercing them.

| Table | Holds | Notes |
|---|---|---|
| `topics` | name, retention override, created_at | created on first publish |
| `subscriptions` | name, state, policy overrides, last_poll_at | unique per (topic, name) |
| `messages` | `seq` (rowid), public ULID `id`, payload blob, content type, due_at | **`seq` is the ordering**, never the id |
| `deliveries` | (msg_seq, sub_id), state, attempts, lease/next-attempt/dead/expired stamps | one row per fan-out target |
| `apps` | name, **encrypted** token, created_at, revoked_at | unique index over live rows only |

**Fan-out is materialized.** Publishing writes one `deliveries` row per active
subscription, inside the same transaction as the message. Fan-out is therefore
not a query performed at read time but a fact recorded at write time — which
is what makes "ack settles one subscription only" a single guarded `UPDATE`.

**Ordering comes from the rowid, not the id.** ULIDs sort by time and the
generator clamps them monotonic, but a clock stepping backwards after a power
cut must not reorder anything — so delivery order reads `seq`.

**Revocation keeps the row.** An app is turned off by stamping `revoked_at`,
not by deleting it: "this existed and I turned it off" is what you want to see
six months later. The unique index covers live rows only, so a revoked name is
free again.

---

## The delivery state machine

```
                  publish
                     │
                     ▼
   ┌───────────► pending ──────────► claimed ──────► acked
   │                │  ▲                │
   │  lease expiry  │  │  nack          │
   │  or nack       │  └────────────────┘
   │                │
   │                ├──► dead      (attempts exhausted)
   │                ├──► expired   (past the message TTL)
   │                └──► lapsed    (subscription archived)
   │
   └── requeue from dead (attempts reset)
```

`acked`, `dead`, `expired` and `lapsed` are settled: every further transition
is refused rather than silently ignored. The TTL is re-checked whenever a
delivery returns to pending, not only while it waits — otherwise a worker that
claims a message and hangs for half an hour could resurrect it past its
deadline.

**Proven by:** the `l4_ar9_*` tests, one per transition, driven by the mock
clock; `p7_g5_settled_deliveries_refuse_every_further_transition`.

---

## Concurrency

One writer connection, four readers. Every write runs inside a transaction on
the single writer, called from `spawn_blocking` so the async runtime never
blocks on SQLite. Reads take a pooled connection and fall back to the writer
when all four are busy.

That single writer is not a bottleneck to apologise for — it is what makes
"retention cannot delete a message mid-replay" true by construction rather
than by locking discipline. The two operations are serialised because there is
only one place writes happen.

Waiting polls are woken by a per-subscription `tokio::sync::Notify`, but
**correctness never depends on the wakeup**: every waiter also re-checks on an
interval, so a lost notification costs latency, never a message.

Sweeps and backfills run in bounded batches (≈500 rows per transaction), so
the writer is never held for seconds and a hard kill mid-sweep cannot replay a
giant rollback.

**Proven by:** `p7_g14_nothing_is_lost_or_duplicated_under_concurrent_load`
(120 messages, five consumers, a publisher and a live sweeper with a 400 ms
lease), `l4_a_sweep_never_exceeds_its_batch_bound`,
`p7_g15_an_ack_at_the_lease_boundary_wins_against_the_live_sweeper`.

---

## Durability

WAL journaling with `synchronous=FULL`, both set explicitly at every open and
asserted by test — build defaults vary, and the promise cannot rest on how
SQLite happened to be compiled.

Migrations are forward-only, keyed on `PRAGMA user_version`. A populated store
is snapshotted with `VACUUM INTO` before migrating, so a bad upgrade is
reversible. A schema newer than the binary is refused with a remedy rather
than guessed at.

**Proven by:** `l1_pragmas_are_set_explicitly_not_assumed`, the `l5_s4_*`
crash suite (SIGKILL under traffic, both a short outage where leases outlive
the downtime and a long one where they do not), `p7_g2_a_hard_kill_at_startup_leaves_a_migratable_store`.

---

## The door (W2)

Two credentials reach the same check: an `Authorization: Bearer` header, or a
`kyu_session` cookie for browsers. The bootstrap token comes from the
environment; app tokens live in the store, **encrypted** with
ChaCha20-Poly1305 under `KYU_SECRET_KEY`.

Encrypted rather than hashed, deliberately: a hash cannot be turned back into
a working command, and printing a working command is what the dashboard is
for. The trade is stated in AR11's amendment — store file alone is useless,
store file plus compose file is total compromise, which on a single-admin LAN
hub changes nothing an attacker with the compose file did not already have.

**Fail-closed by construction.** The router is split in two: an open router
(`/healthz`, `/metrics`, `/static/*`, `/login`, `/logout`) and a protected one
carrying the auth layer. A route added to the protected half is guarded
whether or not anyone remembered to think about it.

No caching of accepted tokens, so revocation is immediate.

---

## Presentation

minijinja templates and Bootstrap 5.3.3, both compiled into the binary, so the
container stays one artifact with nothing mounted beside it. Asset URLs carry
an FNV-1a fingerprint of their contents, which is what lets the cache header
be a year and mean it.

Payloads are untrusted wherever rendered: autoescape on, payloads as text
only, binary announced with its size rather than mangled, truncation always
saying how much is hidden. A script tag in a payload renders inert.

htmx was chosen in T4 and never actually wired up — the tag pointed at a file
that did not exist. It is gone; the reveal and copy controls are one small
first-party script.

**Proven by:** `l7_a_payload_cannot_script_the_dashboard`,
`l7_binary_and_oversized_payloads_are_marked_not_mangled`,
`p7_g10_the_awkward_dashboard_states_all_render` (the compensation owed for
choosing runtime templates: every page rendered with seeded state).

---

## What is deliberately absent

Single node, no clustering. No exactly-once. No routing rules or message
transformation. No user management or per-topic permissions. Never exposed to
the internet. Each of these is a scope decision (N1–N6), not an omission — see
`SCOPE.md`.
