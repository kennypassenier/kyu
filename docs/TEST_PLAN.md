# mailbox — Test plan

What is proven, where, and what is deliberately not covered. Written at
the Phase 7 gate on 2026-08-28 and maintained from here on.

**148 tests.** 116 integration tests across twelve suites, 32 unit tests
inside the modules they belong to. Every suite runs on every commit (the
git hooks refuse a commit whose tests fail) and again in CI.

## Principles these suites follow

- **Real dependencies** (standing rule 9): real HTTP over real sockets,
  real SQLite files on disk, the real compiled binary killed with real
  signals, the real container image. The only mocked thing is the clock,
  because waiting seven days for a retention test is not a strategy.
- **Named after what they protect.** A test called
  `l4_ar9_claimed_to_pending_when_the_lease_expires` says which frozen
  decision fails if it goes red.
- **A bug becomes a test before it becomes a fix** (standing rule 8).
  Every fix recorded below has a test that failed against the old code.

## Suites

| Suite | Tests | What it proves |
|---|---|---|
| `l0_skeleton.rs` | 2 | The server binds a real socket and answers `/healthz` with JSON. |
| `l1_store.rs` | 8 | Migrations apply and are idempotent; a newer schema is refused with a remedy; a populated store is snapshotted before migrating; pragmas (WAL, `synchronous=FULL`, foreign keys) are asserted rather than assumed; foreign keys actually reject an orphan delivery; the AR9 state set is enforced by the schema; **delivery order survives a clock that steps backwards**. |
| `l2_verbs.rs` | 10 | Publish, long-poll receive and ack round-trip in both AR2 response shapes; an acked message never returns, including across a restart; 204 arrives within the wait window; a message published mid-poll is delivered at once; binary payloads survive byte-for-byte; the stored content type is returned unchanged; a new subscription starts from now and says so; **eleven error paths each carry an actionable remedy**. |
| `l3_fanout.rs` | 6 | Two subscriptions each receive every message; an ack settles one subscription only and leaves the sibling's copy pending; a dead subscription accumulates its full backlog without slowing the live one (S5); four concurrent workers on one name handle 40 messages with no duplicates and none lost; an archived subscription receives nothing new; one name on two topics is two subscriptions. |
| `l4_reliability.rs` | 15 | Every AR9 transition, by name, on the mock clock: claim, ack, lease expiry, backoff, dead-lettering at max attempts, TTL expiry, requeue with attempts reset, and the **expire-on-re-pend** rule that stops a half-hour-old doorbell being announced. Also: policy defaults, per-subscription policy, replace semantics, batch bounds, and an ack winning a race against a sweep. |
| `l4_http.rs` | 7 | The reliability endpoints with the real sweeper running: the **S2 crash test** (a consumer killed before acking gets its message back as attempt 2), policy reporting effective vs explicit values, invalid policies refused with remedies, nack, dead-letter listing and requeue, and dead letters surviving a restart. |
| `l5_crash.rs` | 9 | **The S4 suite.** The real binary, killed with SIGKILL: every confirmed publish present and in order; acks still acked; ten kills in a row needing no manual repair; a short outage leaving claimed messages claimed and a long one returning them; `/healthz` reporting a stalled sweeper; the `--healthcheck` flag exiting non-zero when the hub is gone. |
| `l6_history.rs` | 16 | Retention collects what nobody needs and **never a backlog an active subscription still holds**; keep-forever; replay (`?from=beginning`) idempotent and bounded by what retention kept; the idle lifecycle flag → archive → unarchive with lapsed settlement; per-subscription idle thresholds; the hub's own events; and the **loop-breaker**, proven by letting a consumer of `mailbox.events` dead-letter an event and asserting nothing breeds from it. |
| `l7_dashboard.rs` | 9 | Every page rendered with seeded state — the compensation owed for runtime templates (T4). A script tag in a payload renders inert; binary and oversized payloads are announced rather than mangled; a topic nobody polls explains the bootstrap order; **the printed curl snippets are pulled off the page and executed**. |
| `l8_ops.rs` | 7 | Metrics expose per-subscription backlog and the sweeper's age; delayed delivery is durable immediately and survives a restart without firing early; two answers to "when" are refused; **a backup taken under load restores into a working hub that delivers a message**; JSON logs parse as one object per line. |
| `p7_hardening.rs` | 23 | The gaps the Phase 7 audit found: a broken migration rolls back whole; eight kills at startup leave a migratable store; a store that cannot grow refuses publishes loudly and stays up; lapsed deliveries stay lapsed and let retention reclaim; settled deliveries refuse every further transition; replay over HTTP; the retention and unarchive endpoints; payload edges (empty, at-limit, NUL); **payloads never reach the logs or the metric labels**; the awkward dashboard states; `/healthz` at 503; the events topic as an ordinary topic; the dead-letter view and its requeue button; 120 messages through five consumers, a publisher and a live sweeper. |
| `p7_security.rs` | 4 | The security review's findings: a hostile content type cannot escape the copy-paste snippet; cross-origin state-changing requests are refused while scripts are unaffected; a payload cannot render itself in the hub's origin. |
| Unit tests (32) | | Configuration parsing and its remedies (5); payload display and snippet building (8); the write probe (2); the injected clock (3); monotonic ids under a backwards clock (4); name validation (4); policy resolution and validation (6). |

## Defects found by auditing rather than by building

Recorded because they are the argument for auditing as a separate
activity — each passed the whole suite before it was found.

1. **A truncated multi-byte character made a text payload look binary.**
   The dashboard cut payloads at 4096 bytes and then asked whether the
   slice was UTF-8; an ordinary Dutch message long enough to hit the cap
   was displayed as "binary payload". Fixed by backing off to the last
   whole character, and only when the slice ends mid-character.
2. **Retention events fed themselves.** Collecting messages published an
   event onto `mailbox.events`, which became a message retention would
   later collect, emitting another. The topic never reached quiescence.
   Housekeeping is now logged; every remaining event has a subject topic,
   so the loop-breaker always applies.
3. **The dead-letter view was never built.** K6 promises payload, retry
   history and one-click requeue on the dashboard; only a count existed.
   No test caught it, because nothing tests a feature that was never
   written — which is why the Phase 6 registry-coverage item now exists.
4. **Three security findings**, each with a test that failed first: a
   publisher-controlled `Content-Type` escaping the copy-paste shell
   snippet; no cross-origin protection on state-changing requests; and a
   payload able to render itself in the hub's own origin.

## Not covered, by decision

Every gap the Phase 7 audit raised was closed rather than accepted, so
this section records what remains true about the system rather than
choices to skip work.

- **A disk that is full but writable is not visible on `/healthz`.** The
  probe takes SQLite's write lock, which succeeds until a commit needs a
  new page. Publishes then fail loudly with a remedy and the hub stays up
  (proven), but the health endpoint keeps reporting `ok`, so an uptime
  check will not raise it. Watch `mailbox_store_bytes` instead.
- **Cross-origin protection depends on the browser.** Refusing requests
  that announce a foreign `Origin` stops the drive-by case. It is not
  authentication: anything on the LAN that speaks HTTP directly can still
  do anything, which is the documented threat model (N3 LAN-only, W2 auth
  rated Later).
- **Single node, no clustering** (N1). Nothing tests failover because
  there is none.
- **Exactly-once delivery is not offered** (N4). The suites assert
  at-least-once and that duplicates are possible under crash conditions;
  consumers are expected to be idempotent.
