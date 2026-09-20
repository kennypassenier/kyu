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
   `CLAUDE.md` under mini-rounds.
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
denied` until `chown kyu:kyu`. Whether kyu itself gains an idempotent
merge-import is Kenny's open choice.
