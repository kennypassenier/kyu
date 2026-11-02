# kyu — Test plan

What is proven, where, and what is deliberately not covered. Written at
the Phase 7 gate on 2026-08-28 and maintained from here on.

**229 tests** (counted 2026-11-02 by `#[test]` and `#[tokio::test]`
attributes): 178 integration tests across twenty suites in `tests/`, and 51
unit tests inside the modules they belong to. *(Was 148 at the Phase 7 gate;
the 2026-08-28 W2 mini-round added the door and its suite, and later releases
moved the door and the dashboard shell onto the kit, which retired `p7_auth.rs`
and `l7_dashboard.rs` and brought `k2_dashboard.rs`, `kp11_history.rs`,
`dedup_damping.rs` and the others below.)* Every suite runs on every commit
(the git hooks refuse a commit whose tests fail) and again in `chassis release`'s
gate before every release (there is no CI since 2026-09-29).

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
| `l1_store.rs` | 10 | Migrations apply and are idempotent; a newer schema is refused with a remedy; a populated store is snapshotted before migrating; pragmas (WAL, `synchronous=FULL`, foreign keys) are asserted rather than assumed; foreign keys actually reject an orphan delivery; the AR9 state set is enforced by the schema; **delivery order survives a clock that steps backwards**; the kit's write probe passes on a healthy store and fails when writes are refused. |
| `l2_verbs.rs` | 10 | Publish, long-poll receive and ack round-trip in both AR2 response shapes; an acked message never returns, including across a restart; 204 arrives within the wait window; a message published mid-poll is delivered at once; binary payloads survive byte-for-byte; the stored content type is returned unchanged; a new subscription starts from now and says so; **eleven error paths each carry an actionable remedy**. |
| `l3_fanout.rs` | 6 | Two subscriptions each receive every message; an ack settles one subscription only and leaves the sibling's copy pending; a dead subscription accumulates its full backlog without slowing the live one (S5); four concurrent workers on one name handle 40 messages with no duplicates and none lost; an archived subscription receives nothing new; one name on two topics is two subscriptions. |
| `l4_reliability.rs` | 15 | Every AR9 transition, by name, on the mock clock: claim, ack, lease expiry, backoff, dead-lettering at max attempts, TTL expiry, requeue with attempts reset, and the **expire-on-re-pend** rule that stops a half-hour-old doorbell being announced. Also: policy defaults, per-subscription policy, replace semantics, batch bounds, and an ack winning a race against a sweep. |
| `l4_http.rs` | 7 | The reliability endpoints with the real sweeper running: the **S2 crash test** (a consumer killed before acking gets its message back as attempt 2), policy reporting effective vs explicit values, invalid policies refused with remedies, nack, dead-letter listing and requeue, and dead letters surviving a restart. |
| `l5_crash.rs` | 9 | **The S4 suite.** The real binary, killed with SIGKILL: every confirmed publish present and in order; acks still acked; ten kills in a row needing no manual repair; a short outage leaving claimed messages claimed and a long one returning them; `/healthz` reporting a stalled sweeper; the `--healthcheck` flag exiting non-zero when the hub is gone. |
| `l6_history.rs` | 17 | Retention collects what nobody needs and **never a backlog an active subscription still holds**; keep-forever; replay (`?from=beginning`) idempotent and bounded by what retention kept; the idle lifecycle flag → archive → unarchive with lapsed settlement; per-subscription idle thresholds; the hub's own events; the **loop-breaker**, proven by letting a consumer of `kyu.events` dead-letter an event and asserting nothing breeds from it; and the **expiry window** (W11 amendment, 2026-09-18): three expiries one sweep apart produce one `message.expired`, and the next carries `count` 2 only after the window — the CT 109 flood, replayed on the mock clock. |
| `k2_dashboard.rs` | 23 | The hub's side of the kit-owned door and dashboard (3.0.0 onward; it replaces `l7_dashboard.rs` and the dashboard half of `p7_auth.rs`): the kit owns the door; `--check` refuses to start without a token; apps issued by 2.x keep working after the import; no login token leaks into `/metrics` or the topic list; the topics page lists every topic and the status page counts them; every page rendered with seeded state; a script tag in a payload renders inert; binary and oversized payloads are announced rather than mangled; a hostile content type cannot escape the printed snippet; **the printed curl snippets are pulled off the page and executed**; the test-publish form; dead letters shown, requeued, deleted one subscription at a time, and pruned in one confirmed sweep; the hub's assets are served open and fingerprinted. |
| `kit_smoke.rs` | 1 | The kit's own smoke test (it is the kit's file, rewritten by `chassis sync`): the service starts, the admin logs in, a client token is issued and the API accepts it, the dashboard is served. |
| `kp11_history.rs` | 20 | The dashboard history (schema 7): one attempt row per claim with the app that took it, nacks and exhaustion, a lease that runs out, requeue continuing the numbering, retention taking attempts with the message, the sweeper's 5-minute samples kept 48 hours, the caps, Brussels days, migration 7 keeping every row of a schema-6 store, and the JSON routes the pages fetch (attempts, daily, hourly, totals, retention, bulk requeue/delete, replay, form posts answering JSON when asked). |
| `dedup_damping.rs` | 12 | Damping of repeated notifications (schema 8): no key behaves as before; a repeat inside the window returns the original id; after the window the key rearms; another key or topic passes; a key alone defaults to 24 hours; windows take milliseconds and units; bad headers get a remedy; a suppressed repeat delivers nothing and is counted; the sweeper prunes expired keys and orphans; the table is capped by evicting the oldest; `kyu_dedup_keys` and `kyu_duplicates_suppressed` on `/metrics`. |
| `l8_ops.rs` | 7 | Metrics expose per-subscription backlog and the sweeper's age; delayed delivery is durable immediately and survives a restart without firing early; two answers to "when" are refused; **a backup taken under load restores into a working hub that delivers a message**; JSON logs parse as one object per line. |
| `w12_shutdown.rs` | 4 | Graceful shutdown with real signals against the real binary: SIGTERM exits cleanly and leaves the files standing still, the backlog survives the stop, a second SIGTERM during shutdown changes nothing, and an in-flight long poll is answered rather than cut off. |
| `f179_backup.rs` | 4 | `deploy/kyu-backup` run for real against a live hub: it takes its paths from the environment and writes a backup; a missing variable is refused loudly and never guessed; a wrong token fails instead of reporting success; pruning keeps the newest and the count is configurable (`KYU_BACKUP_KEEP`). |
| `door_import.rs` | 2 | K2-1 amended (3.4.0): the 2.x `apps` table is merged into the kit's client store at every start, by name — a store with two apps and a door holding one of them (under a UUID id, as the kit writes them) gains exactly the missing one and a second start adds nothing; a revoked row with the same name and no token does not block re-adoption. Real files, the kit's own `FileClientStore`. |
| `fix_check_1.rs` | 2 | `--check` on the real binary against a store one schema behind: exit 0, says "schema N … migrat…", leaves `user_version` unchanged and no `kyu.pre-v*.db`; and a state directory with no store passes without creating one (fix-check-1, 2026-09-20; red first). |
| `p7_hardening.rs` | 19 | The gaps the Phase 7 audit found: a broken migration rolls back whole; eight kills at startup leave a migratable store; a store that cannot grow refuses publishes loudly and stays up; lapsed deliveries stay lapsed and let retention reclaim; settled deliveries refuse every further transition; replay over HTTP; the retention and unarchive endpoints; payload edges (empty, at-limit, NUL); a corrupt backup target is not reported as a backup; **payloads never reach the logs or the metric labels**; `/healthz` at 503; the events topic as an ordinary topic; 120 messages through five consumers, a publisher and a live sweeper. |
| `p7_cli.rs` | 5 | What the binary does when it is called wrong (1.0.1): `--version` and `--help` answer and exit rather than starting the hub; an unknown flag and a stray positional argument are refused with exit code 1 (3.0.0, the kit's; 2.x used 2) and a remedy naming what was not understood; and no arguments still serves. Each spawns the real binary under a timeout, because the bug being pinned turns a question into a running process. |
| `p7_security.rs` | 3 | The security review's findings about payloads: a payload cannot render itself in the hub's origin, an ordinary payload still opens in a browser, and every executable content type is forced to download. (The cross-origin and hostile-snippet findings are now covered by the kit and by `k2_a_hostile_content_type_cannot_escape_the_dashboard_snippet`.) |
| Unit tests (51) | | `src/crypto.rs` 12 (a sealed token comes back intact, the plaintext never appears in the sealed bytes, another key cannot open it and says what to do, tampering is refused, key parsing, generated tokens, constant-time compare); `src/config/auth.rs` 7 (the door pair, including a token without a key); `src/dashboard/payload.rs` 5 and `snippets.rs` 3 (payload display, snippet building); `src/engine/policy.rs` 6, `names.rs` 4, `ids.rs` 4, `clock.rs` 3, `dedup.rs` 3; `src/config/retired.rs` 2 (the retired `KYU_DATA_DIR` refuses to start), `src/config/mod.rs` 1 (the expiry window refuses `never`), `src/dashboard/mod.rs` 1. |

## Defects found by auditing rather than by building

Recorded because they are the argument for auditing as a separate
activity — each passed the whole suite before it was found.

1. **A truncated multi-byte character made a text payload look binary.**
   The dashboard cut payloads at 4096 bytes and then asked whether the
   slice was UTF-8; an ordinary Dutch message long enough to hit the cap
   was displayed as "binary payload". Fixed by backing off to the last
   whole character, and only when the slice ends mid-character.
2. **Retention events fed themselves.** Collecting messages published an
   event onto `kyu.events`, which became a message retention would
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

- ~~A disk that is full but writable is not visible on `/healthz`.~~
  **Closed at the Phase 7 gate (2026-08-28).** Rather than predicting a
  failure the write lock cannot see, the store now remembers that a write
  actually failed and health reports it for a minute afterwards, then
  recovers by itself. Only storage failures count — an unknown topic is
  the caller being wrong, and a hub that called itself unhealthy over a
  404 would be crying wolf. Proven by
  `p7_g3_a_full_store_refuses_publishes_loudly_and_stays_up`, which caps
  the store, publishes until it refuses, and asserts a 503 whose remedy
  names free space first.
- **Cross-origin protection depends on the browser.** Refusing requests
  that announce a foreign `Origin` stops the drive-by case, and on a
  protected hub it also stops a foreign page riding your session cookie.
  It is not authentication in itself.
  *Amended 2026-08-28:* the sentence that used to close this entry —
  "anything on the LAN that speaks HTTP directly can still do anything" —
  is now true only of a hub deliberately run without a token. With
  `KYU_TOKEN` set, the verbs and the dashboard need one (W2). What
  stays open by design is `/healthz` and `/metrics`, so a LAN observer can
  still learn topic and subscription *names* and their counts, never a
  payload. Proven by `k2_no_login_token_leaks_into_metrics_or_the_topic_list`.
  *Amended 2026-11-02:* since 3.0.0 there is no hub "deliberately run without a
  token": the kit refuses to start without `KYU_TOKEN` and `KYU_SECRET_KEY`.
- **The copy button is not covered by an automated test.** Both clipboard
  paths — `navigator.clipboard.writeText` and the `execCommand` fallback —
  require a focused document and a real user gesture, by specification. A
  headless or background tab has neither (`document.hasFocus() === false`,
  `navigator.userActivation.hasBeenActive === false`), so an automated click
  always fails there and would prove nothing either way. What *is* covered:
  the command placed in `data-secret` is the real, working one
  (`k2_the_snippets_the_dashboard_prints_actually_work`), and
  the reveal/hide path was exercised end to end against the then-shipped `app.js`
  in a real browser on 2026-08-28 (that script was removed in 6.0.0; the
  pages' scripts are now the ES modules in `static/`). The button itself needs a human click.
- **Single node, no clustering** (N1). Nothing tests failover because
  there is none.
- **Exactly-once delivery is not offered** (N4). The suites assert
  at-least-once and that duplicates are possible under crash conditions;
  consumers are expected to be idempotent.
