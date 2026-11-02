# kyu — architecture reference

The system as built, in Phase 8. `ARCHITECTURE_DECISIONS.md` records what was
decided and why, including the roads not taken; this file records what is
actually there, so you can navigate the code without reading all of it.

---

## Shape

```
HTTP (axum)                 src/http/
  ├── mod.rs                three routers: router() (machine API),
  │                         pages() (dashboard), assets() (open files)
  ├── handlers/             one module per area: publish, receive, settle,
  │   │                     policy, dead, retention, backup, assets, wire,
  │   │                     engine_call
  │   └── dashboard/        the pages, their form actions and the JSON
  │                         their scripts fetch (topics, topic,
  │                         subscription, bulk, history, stats, ...)
  ├── dedup.rs              Kyu-Dedup-Key / Kyu-Dedup-Window on publish
  ├── notify.rs             long-poll wakeups
  ├── page_modules.rs       the ES modules the page scripts import
  └── error.rs              every failure becomes {error, remedy}
        │
Engine                      src/engine/
  ├── mod.rs                the Engine and its defaults
  ├── publish.rs            publish, scheduled delivery, replay, damping
  ├── claim.rs              claim (poll) and ack
  ├── settle.rs, dead.rs    nack, backoff, dead-lettering, dead-letter list
  ├── subscriptions.rs      lifecycle, retention, policy
  ├── sweep.rs              the background pass
  ├── dedup.rs, apps.rs     damping rules; the 2.x apps table
  ├── clock.rs, ids.rs      injected time; ULIDs that never go backwards
  ├── names.rs, days.rs     what a topic may be called; Brussels days
  ├── policy.rs             lease, attempts, backoff, ttl: resolved
  └── error.rs, store_access.rs
        │
Store                       src/store/
  ├── mod.rs                file name, reader count, pre-migration copies
  ├── migrations/           the schema as a list, one entry per version
  ├── queries/              every SQL statement, one file per concern
  └── dedup.rs              the SQL behind publish damping
        │
SQLite (the kit's AppDb: WAL, synchronous=FULL), opened by the kit
  (src/storage.rs declares the schema, pool shape and Storage-page kinds)

Background                  src/sweeper.rs    leases, TTL, retention, idle
Self-reporting              src/events.rs     kyu.events
Kit glue                    src/kit/          /healthz subsystems, /metrics
                                              series, status-page section,
                                              2.x token import
Configuration               src/config/       KYU_* environment, the door
                                              pair, the retired KYU_DATA_DIR
Presentation                src/dashboard/ (view models) + templates/ +
                            static/ (page scripts and stylesheets)
Binary                      src/main.rs       assembles all of it on chassis
```

**The rule that keeps it navigable:** SQL lives only under `store/`
(`queries/` and `dedup.rs`), business logic only in `engine/`, HTTP shape only
in `http/`. A handler that starts reasoning about delivery is in the wrong
file. No file of code is over 250 lines (CHANGELOG 6.0.0).

---

## The data model

Four core tables, one for apps, and the history and damping tables. Times
are integer milliseconds since the Unix epoch (`stats_daily.day` is a
Europe/Brussels day number). `STRICT` throughout, so SQLite enforces column
types instead of coercing them. The schema is the list in
`src/store/migrations/schema.rs`, currently eight versions.

| Table | Holds | Notes |
|---|---|---|
| `topics` | name, retention override, created_at | created on first publish; `kyu.events` exists from migration 2 |
| `subscriptions` | name, state, policy overrides, idle thresholds, last_poll_at, expiry-announcement state | unique per (topic, name) |
| `messages` | `seq` (rowid), public ULID `id`, payload blob, content type, published_at, due_at | **`seq` is the ordering**, never the id |
| `deliveries` | (msg_seq, sub_id), state, attempts, lease/next-attempt/dead/expired stamps | one row per fan-out target; a dashboard delete leaves a `removed` row so a replay does not bring it back (migration 6) |
| `apps` | name, **encrypted** token, created_at, revoked_at | the 2.x registry, kept as history; the kit's client store now owns the tokens (see The door) |
| `attempts` | one row per claim: attempt number, outcome, app, reason | migration 7; follows its message, and the sweeper drops rows older than 30 days |
| `stats_hourly`, `stats_daily`, `stats_samples` | published/dead/deduplicated counters per hour, delivered/dead per day, totals sampled every 5 minutes | migration 7 (and 8 for `deduplicated`); kept 30 days, 400 days and 48 hours |
| `dedup_keys` | (topic, key), message id, window end | migration 8; capped at 10,000 rows, pruned by the sweeper |

The kit adds two small tables of its own to `kyu.db` (`chassis_meta`,
`chassis_prefs`); its own data (clients, sessions) is a separate file,
`kyu-kit.db`, beside it (CHANGELOG 5.0.0; `src/storage.rs`).

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

Migrations are forward-only, keyed on `PRAGMA user_version`, and applied by
the kit's runner (`src/store/migrations/mod.rs`). A populated store is copied
to `kyu.pre-v<N>.db` before migrating (the newest two are kept,
`SNAPSHOTS_KEPT` in `src/store/mod.rs`), so a bad upgrade is reversible. A
schema newer than the binary is refused with a remedy rather than guessed at,
and `--check` reports the state without migrating.

**Proven by:** `l1_pragmas_are_set_explicitly_not_assumed`, the `l5_s4_*`
crash suite (SIGKILL under traffic, both a short outage where leases outlive
the downtime and a long one where they do not), `p7_g2_a_hard_kill_at_startup_leaves_a_migratable_store`.

---

## The door (W2)

Since 3.0.0 (step 2, 2026-09-06) the kit owns the door; kyu only says which
routes sit behind which half of it (`src/kit/mod.rs`, `src/http/mod.rs`):

- `router()` is handed to `App::api_routes`: the machine API (the three
  verbs, policy, dead letters, retention, backup) needs
  `Authorization: Bearer <client token>`, or the login token for Kenny's own
  scripts.
- `pages()` is handed to `App::dashboard_routes`: the dashboard pages sit
  behind the admin login, and the kit refuses cross-origin form posts.
- `assets()` (`/assets/{file}`) is open; `/healthz` and `/metrics` are the
  kit's own routes. `KYU_TRUSTED_ADMINS` names addresses that are the admin
  without a login (kit knob, `docs/KIT.md`).

Login, the session cookie, the Apps page and revocation are the kit's; this
repository no longer contains a cookie, a login handler or a CSRF layer.
Client tokens and sessions live sealed in `kyu-kit.db` under
`KYU_SECRET_KEY`. The 2.x `apps` table in `kyu.db` stays as history, and
every start copies the apps whose name the kit's client store lacks into that
store (`src/kit/import.rs`, `tests/door_import.rs`).

Encrypted rather than hashed, deliberately: a hash cannot be turned back into
a working command, and printing a working command is what the dashboard is
for. The trade is stated in AR11's amendment: store file alone is useless,
store file plus the environment file is total compromise, which on a
single-admin LAN hub changes nothing an attacker with the environment file
did not already have. `src/crypto.rs` is kyu's own seal for the 2.x table;
the kit seals its stores with the same key in its own format.

**Fail-closed by construction.** The binary hands the kit only closed
routers, plus the open `/assets`; a route added to `router()` or `pages()` is
guarded whether or not anyone remembered to think about it. The kit refuses
to start without `KYU_TOKEN` and `KYU_SECRET_KEY`.

## Presentation

The pages are templates in `templates/` (a page plus partials, expanded in
`src/http/handlers/dashboard/page.rs`) rendered inside the kit's layout, with
the dashboard's CSS and JavaScript from kp-themes (one bundle,
`/static/kp/dist/kp-themes.min.js`, served by the kit). kyu's own page
scripts and stylesheets in `static/` are compiled into the binary and served
from memory on `/assets/{file}`: the entry files and stylesheets carry a
`?v=<fingerprint>` and a year-long `immutable` cache header, while the page
scripts' part modules (ES modules imported without a fingerprint) are served
`no-cache` with an `ETag`, so a browser revalidates them and gets a 304 when
nothing changed (`src/http/handlers/assets.rs`, CHANGELOG 6.0.0). The
fingerprint is an FNV-1a hash of the embedded files.

Payloads are untrusted wherever rendered: autoescape on, payloads as text
only, binary announced with its size rather than mangled, truncation always
saying how much is hidden. A script tag in a payload renders inert.

htmx was chosen in T4 and never actually wired up — the tag pointed at a file
that did not exist. It is gone; the pages' behaviour (reveal and copy, tables,
charts) is first-party ES-module scripts in `static/`, one entry per page.

**Proven by:** `k2_a_payload_cannot_script_the_dashboard`,
`k2_binary_and_oversized_payloads_are_marked_not_mangled`,
`k2_the_awkward_dashboard_states_all_render` (the compensation owed for
choosing runtime templates: every page rendered with seeded state), all in
`tests/k2_dashboard.rs`, and `k2_the_hub_assets_are_served_open_and_fingerprinted`.

---

## What is deliberately absent

Single node, no clustering. No exactly-once. No routing rules or message
transformation. No user management or per-topic permissions. Never exposed to
the internet. Each of these is a scope decision (N1–N6), not an omission — see
`SCOPE.md`.
