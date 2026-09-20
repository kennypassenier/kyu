# Corrections

Live-found faults and what was decided about them (standing rule 29,
FORM_PROTOCOL §8). One record per fault, the nine fields in order; the
form Kenny answers is the summary, this is the record.

## fix-events-1 · `message.expired` flooded its consumer (found 2026-09-18)

1. **What went wrong.** 27,991 `message.expired` events for one
   subscription (`notify.kenny`/`desktop`) between 2026-09-04 16:18 and
   2026-09-10 18:45, 27,969 of them with `count` 1; measured with
   `sqlite3 -readonly /appdata/kyu/kyu-config/data/kyu.db` on CT 109.
   Each became a Home Assistant notification, and Home Assistant publishes
   its notifications for Kenny back onto `notify.kenny` — a loop that grew
   from 314 events on the first day to 6,592 on the sixth, and left 15,690
   items in `todo.notifications`.
2. **Which gate let it through.** W11's test bar said "event-emitted
   assertions inside the K6/K11 suites": it checked that an event exists,
   never how many. Phase 7's audit asked "could this assertion ever fail",
   not "what does this emit per hour when the consumer is gone".
3. **Where else the same fault sits.** The property: a per-sweep bundle of
   something a subscription can produce every sweep. Searched with
   `grep -n "events::emit" src/engine/mod.rs` — five emitters. Dead
   letters are bounded by attempts, flag/archive/unarchive fire once per
   state change; only `Expired` had no bound. Nowhere else.
4. **How we prevent recurrence.** One announcement per subscription per
   `KYU_EXPIRED_EVENT_WINDOW_MS` (a day), carrying the count since the
   previous one, state kept in the store (migration 5).
5. **What the remedy costs.** Two columns, one environment variable, and a
   consumer that wants finer granularity than a day sets the variable.
6. **Who or what enforces it.** Code-enforced:
   `l6_w11_expiries_are_announced_once_per_window_with_their_count`
   (integration test in `tests/`, runs with the suite at every commit that
   touches Rust, per this project's gates) drove red with `[1, 1]` before
   the fix.
7. **How we measure that it works, and when.** At the first day on CT 109
   after 3.3.0 is live with a polled-by-nobody subscription: the hub
   journal shows at most one `hub event published` with
   `event=message.expired` per subscription per day. Queued in
   `CLAUDE.md` under mini-rounds. 3.3.0 went live 2026-09-20 19:23 UTC;
   the window starts at 19:30 UTC (Homelab Rust measured both runner and
   switchboard `degraded` for seconds right after the restart — a sample
   taken then describes the restart, not the steady state) and is read
   after 2026-09-21 19:30 UTC. One `message.expired` was published at the
   restart itself (the backlog of the `desktop` subscription), which is the
   first-after-a-quiet-spell case, not a count against the window.
   Live so far (verified 2026-09-20 20:00 UTC): exactly one announcement at
   19:32:20 UTC, `expired_announced_at` set on `notify.kenny`/`desktop`,
   `expired_unannounced` 2 — the hub is counting, not announcing.
8. **Fallback if the measurement fails.** `KYU_EXPIRED_EVENT_WINDOW_MS` is
   read at start; the HA-side throttle (24 h per topic in
   `automation.hub_kyu_events_webhook`) holds the to-do list regardless.
9. **When we review the measure.** At the next W11 change, or when a
   second consumer of `kyu.events` exists.

## fix-state-1 · The 3.x unit opened an empty store (found 2026-09-18)

1. **What went wrong.** `deploy/kyu.service` sets
   `Environment=KYU_STATE_DIR=/appdata/kyu/kyu-config`; the 2.x
   `kyu.env` on CT 109 still said `KYU_DATA_DIR=/appdata/kyu/kyu-config/data`.
   3.x let the new name win silently and started on an empty store on
   2026-09-10 18:52 (`ls -la /appdata/kyu/kyu-config`: `kyu.db` 61 KB
   beside `data/kyu.db` 50 MB). Every client token except kyu-runner's
   went with it: 55 refused publishes (401) and 152,738 polls answering
   404 in the 1.5 days of journal that exist.
2. **Which gate let it through.** The 3.0.0 migration's "still honoured
   with a warning" was tested for the alias alone, never for both names
   set; the CT 109 rollout checked `/healthz` and the sweeper, not the
   topic count. Rule 15a (a store is three files, moved together and its
   rows counted afterwards) was written for a move and nobody saw this
   as one.
3. **Where else the same fault sits.** The property: a renamed setting
   whose old and new name can both be set. Searched with
   `grep -rn "the 2.x name\|still honoured" src/ docs/ README.md` —
   `KYU_DATA_DIR` is the only alias the hub keeps. Nowhere else in kyu;
   the same property in chassis-rs consumers is theirs to search.
4. **How we prevent recurrence.** Both names set and different → refuse
   to start, naming both directories and the three files to move.
5. **What the remedy costs.** One `match`; an operator with a stale env
   file sees a refusal instead of a silent empty hub.
6. **Who or what enforces it.** Code-enforced: unit test beside the code
   (`fix_state_1_two_state_roots_that_disagree_refuse_to_start`, commit
   subset), drove red before the fix.
7. **How we measure that it works, and when.** Restated 2026-09-20: the
   `KYU_DATA_DIR` line left `kyu.env` on 2026-09-19 (verified from this
   session on 2026-09-20: the file carries only `KYU_TOKEN`,
   `KYU_SECRET_KEY`, `KYU_LISTEN`, `KYU_LOG`), so the original moment can
   no longer occur by itself. The measurement is now a deliberate drill at
   the 3.3.0 deploy on CT 109: run `kyu --check` once with
   `KYU_DATA_DIR=/appdata/kyu/kyu-config/data` added to the unit's
   environment and expect the refusal naming both directories; then the
   normal `--check` must pass. Queued in `CLAUDE.md`.
8. **Fallback if the measurement fails.** The deploy stops there; the old
   store at `data/` is untouched either way.
9. **When we review the measure.** At 4.0, when the alias is removed and
   the guard goes with it.

**Measurement DONE 2026-09-20 19:21 UTC (Homelab Rust, at the 3.3.0 deploy
on CT 109; verified from this session at 19:40).** With
`KYU_DATA_DIR=/appdata/kyu/kyu-config/data` added to the unit's
environment, `kyu --check` exited 1 naming both directories and the three
files to move; without it: `store OK at /appdata/kyu/kyu-config/kyu.db`,
`configuration ok`, exit 0. After the swap: `/healthz` 3.3.0 ok, 8 clients
in the door, journal since the restart 134× 204, 56× 200, 5× 201, zero
401. Loop closed. Two things seen on the way, neither this correction's:
the deploy first failed on a root-owned binary in the kyu-owned
`/opt/kyu/bin` (`cannot keep the previous binary … Operation not
permitted`) with a kit message that blames directory permissions — a
chassis-rs finding, queued for relay in CLAUDE.md; and the drill's
`--check` as root migrated the store to schema 5 and left
`kyu.pre-v4.db` root-owned in the state dir (removal later works, the
directory is kyu's — but a `--check` that writes is worth a look at 4.0).

**Follow-up 2026-09-20 — the door, closed by hand.** The restored store
did not restore the door (kyu 3.x reads `clients.json.enc`, and
`import_app_tokens` runs only when that file is absent). Homelab Rust
adopted the four missing apps (ha, newsflash, radarr, sonarr) into the
kit's client store with a one-off tool on CT 109 on 2026-09-20 18:57 UTC;
homelab-host and alertmanager had been issued through the door on
2026-09-19 22:15. Verified from this session: `GET /api/clients` lists 8,
zero 401 in the hub journal after 16:30 UTC, the file is owned by
`kyu:kyu`. Three things Homelab Rust measured that any future import into
the kit's store must respect: (1) match on the client's NAME with a live
token, never on an `app-<name>` id — clients the door issues itself carry
UUIDs, and an id-based dedup offered to re-add the two working services;
(2) stop kyu while writing — the running service rewrites
`clients.json.enc` about once a minute (`last_used_at`/`uses`), so an
outside write is clobbered; (3) a write as root leaves the file root-owned
and kyu refuses to start with `cannot read clients store … Permission
denied` until `chown kyu:kyu`. Decided by Kenny on 2026-09-20 ("ja,
merge-import"): kyu 3.4.0 imports the table at every start, by name,
skipping names that already hold a live token — `src/kit.rs`, proven by
`tests/door_import.rs` (red first).

## fix-check-1 · `kyu --check` migrated the live store (found 2026-09-20)

1. **What went wrong.** During the fix-state-1 drill on CT 109, Homelab
   Rust ran `kyu --check` (3.3.0) as root against the live store: it
   opened the store, applied migration 5 and left `kyu.pre-v4.db`
   (35,368,960 bytes, root:root, stamped 2026-09-20T19:21:59Z) in the state
   dir. Measured there: `pragma user_version` 5 before the service itself
   had started. `src/main.rs:117-122` does this on purpose ("`--check`
   opens it too: a store that will not open is exactly what a pre-start
   check exists to catch"), and opening migrates.
2. **Which gate let it through.** The 3.0.0 migration to the kit added
   `--check` as `ExecStartPre` and reused the serve path's `Store::open`;
   no test asks what `--check` writes, and its help line ("opens no
   socket") let the operator read "no socket" as "no writes".
3. **Where else does the same fault sit.** The property: a command that
   presents itself as a check and opens the store through the migrating
   path. `Store::open` is called once in the binary (shared by `--check`
   and serve); the kit's other early flags return before it. Nowhere else.
   Searched with: `grep -n "Store::open\|app.run()" src/main.rs`.
4. **How we prevent recurrence.** `--check` opens the store read-only:
   it reports "schema N, this binary knows M — migration runs at start,
   snapshot first" and applies nothing; a store it cannot read still fails
   the check. The help line says what `--check` reads and that it writes
   nothing.
5. **What the remedy costs.** A read-only open path in `Store` and one
   more line of output; the pre-start check loses nothing it exists for.
6. **Who or what enforces it.** Code: an integration test that runs the
   real binary with `--check` against a version-4 store and asserts
   `user_version` is still 4 afterwards and no `kyu.pre-v*.db` appeared
   (full suite). Discipline until then: Homelab Rust drills on a copy
   (`--state-dir` at a copy), recorded there as fix-22.
7. **How we measure that it works, and when.** At the next kyu release
   after this lands: the drill on CT 109 runs `--check` against the live
   store and `ls` shows no new `kyu.pre-v*.db`; the journal shows the
   migration happening at the service start, not before.
8. **Fallback if the measurement fails.** The migration's own snapshot
   keeps the store recoverable either way; Homelab Rust keeps drilling on
   a copy.
9. **When we review the measure.** At 4.0, when the migration set is
   revisited with the alias removal.

**Kenny: Klopt (2026-09-20). Built the same evening** — `Store::inspect`
(read-only open, `quick_check(1)`, `migrations::pending`), `--check` returns
before the migrating path in `src/main.rs`; `tests/fix_check_1.rs` drove red
first (the check migrated and created the store) and is green. Ships as
3.5.0. Measurement (field 7) stays OPEN until the first CT 109 deploy of a
release carrying it, where Homelab Rust runs `--check` against the live
store and no `kyu.pre-v*.db` appears.

