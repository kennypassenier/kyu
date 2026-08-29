# kyu — debugging guide

Symptom first, because that is what you have when something is wrong. Written
in Phase 8 from the code and the tests; the "how you confirm" column is a
command you can run, not a suggestion to think about it.

**Where the evidence lives.** In order of how quickly it tells you something:

| Source | Command | What it settles |
|---|---|---|
| Health | `curl -s $HUB/healthz` | is the store writable, is the sweeper alive |
| Metrics | `curl -s $HUB/metrics` | backlogs, dead counts, store size, sweeper age |
| Dashboard | `$HUB/t/<topic>/dashboard` | per-subscription state, dead letters with payload |
| Hub events | `curl "$HUB/t/kyu.events/next?as=debug&envelope=json"` | what the hub decided and when |
| Logs | `docker logs kyu` | startup, migrations, errors, refused logins |

Payloads and tokens are in none of them except the dashboard, on purpose.

---

## Nothing arrives

| Symptom | Likely cause | How you confirm | Fix |
|---|---|---|---|
| Poll returns `204` forever, publisher says `201` | The subscription was created **after** the message was published. It only sees what comes next (G7). | First poll carried a `kyu-notice` header saying so. Dashboard shows the subscription with backlog 0 while the topic has messages. | Publish again, or `&from=beginning` to pull in what is retained. |
| `404` on receive, `topic … does not exist` | Nothing has ever published there. Topics are created by publishing, not by polling. | `curl -s $HUB/ ` — the topic is absent from the index. | Publish once. Check the spelling against the index. |
| `404` on ack, `subscription … does not exist` | Typo in `as=`, or you acked under a different name than you received with. | The dashboard lists the subscriptions that do exist. | Use the exact name from the dashboard. |
| Consumer sees nothing, siblings do | This subscription is **archived** (30 quiet days). | Dashboard shows state `archived`. `kyu.events` has a `subscription.archived` for it. | `POST …/unarchive`, then raise its idle thresholds so it does not recur. The old backlog is gone — it lapsed, by design. |
| Messages appear then vanish | Retention collected them (7 days default) — but only ones no active or flagged subscription still needed. | `kyu.events` has `message.expired`; `kyu_messages` dropped. | Raise `retention_ms` for that topic, or `"never"`. |

## The same message keeps coming back

| Symptom | Likely cause | How you confirm | Fix |
|---|---|---|---|
| Redelivery every ~30 s | The consumer never acks, or crashes before acking. The lease expires and the message returns — that is at-least-once working. | `kyu-attempt` header climbs. Dashboard shows it "in flight". | Ack after processing. If processing is genuinely slow, raise `lease_ms` for that subscription. |
| Attempts climb then stop, message gone | It ran out of attempts and was dead-lettered. | Dashboard's dead-letter section; `kyu.events` has `message.dead_lettered`. | Fix the consumer, press **Requeue**. |
| Two consumers both handle it | Two *different* subscription names, which is fan-out working as designed. | Dashboard lists two subscriptions on the topic. | Use the **same** `as=` name in both processes to compete instead. |
| Duplicates under one name | At-least-once. A lease expired mid-processing and the message was re-handed out. | `kyu-attempt` > 1 on the duplicate. | Make the consumer idempotent. There is no exactly-once and there will not be (N4). |

## Messages hang rather than fail

| Symptom | Likely cause | How you confirm | Fix |
|---|---|---|---|
| Nothing redelivers, nothing dead-letters, backlog frozen | **The sweeper has stopped.** This is the failure that is invisible from the outside: without it, expired leases never return. | `curl -s $HUB/healthz` → `"sweeper":"stalled"`, HTTP 503. `kyu_sweeper_age_ms` climbing. | Restart the container. Alert on `kyu_sweeper_age_ms` so you learn this from monitoring rather than from a user. |

**Proven by:** `l5_healthz_goes_unhealthy_when_the_sweeper_stops`.

## Publishing is refused

| Symptom | Likely cause | How you confirm | Fix |
|---|---|---|---|
| `503`, remedy mentions free space | The store cannot grow. kyu refuses rather than confirming a publish it cannot keep. | `curl -s $HUB/healthz` → `"store":"unwritable"` with the error. `docker logs` has `a store write failed`. | Free space on the data volume, then check it is still mounted and writable. The hub recovers by itself once writes succeed. |
| `413` | Payload over `KYU_MAX_BODY_BYTES` (1 MiB default). | The error names the limit. | Raise the limit, or put the bulk somewhere else and send a reference. kyu is a message hub, not a file store. |
| `403`, "reserved for kyu's own events" | You published to a `kyu.*` topic. | The topic name starts with `kyu.`. | Pick another name. You can still *subscribe* to `kyu.*`. |
| `400`, name not allowed | Uppercase, spaces or slashes in a topic or subscription name. | The remedy lists the allowed characters. | Lowercase letters, digits, `.`, `_`, `-`. Dots namespace: `notify.kenny`. |

**Proven by:** `p7_g3_a_full_store_refuses_publishes_loudly_and_stays_up`,
`l2_every_error_carries_a_remedy`.

## The door

| Symptom | Likely cause | How you confirm | Fix |
|---|---|---|---|
| `401` on every call | The hub has a token and your call does not carry it. | The remedy shows the exact header. | `-H "authorization: Bearer $KYU_TOKEN"`. The dashboard prints the whole command for you. |
| Browser bounces to `/login` in a loop | Cookies are being dropped, or you are on a different host than the one you logged in on. | The login POST returns `303` with a `set-cookie`; if the next page still redirects, the cookie is not coming back. | Check you are using the same origin. The cookie is `HttpOnly; SameSite=Lax`, deliberately. |
| Hub refuses to start, names `KYU_SECRET_KEY` | A token without a key. Half-configured is not allowed. | The error prints a freshly generated key to paste. | Paste it. Keep both values together. |
| Apps page lists tokens as `unreadable` | `KYU_SECRET_KEY` changed. The old tokens cannot be decrypted. | The page says so in place. | Revoke each and generate replacements. See runbook §5. |
| Warning banner on every page | No token configured at all. | `docker logs` has the same warning at startup. | Deliberate? Fine. Otherwise set both variables and restart. |
| Copy button says "Copy failed — use Reveal" | The browser refused clipboard access — no user gesture, or an unfocused window. | Reveal works and shows the same command. | Use Reveal and select the text. Not a hub fault; both clipboard paths need a real click in a focused window. |

**Proven by:** the thirteen tests in `tests/p7_auth.rs`;
`p7_a_token_without_a_key_refuses_to_start_and_hands_over_a_key`.

## Startup and upgrades

| Symptom | Likely cause | How you confirm | Fix |
|---|---|---|---|
| `cannot open the store at /data/kyu.db` | The data directory is not writable by uid 65532. Almost always a bind mount, which does not inherit the image's ownership. | `docker logs` shows exactly that line with its remedy. | Use a named volume, or `chown 65532:65532` the host directory. See the comment in `compose.yml`. |
| Refuses to start, "newer schema" | The volume was written by a newer kyu than this image. | The error says which versions. | Roll forward to the newer image, or restore the pre-migration snapshot beside the store. |
| Container restart loop right after an upgrade | The healthcheck fired during a long migration. | `docker inspect` shows health `starting` then `unhealthy`. | The Dockerfile's start-period already exceeds worst-case migration; if you overrode it, put it back. |

**Proven by:** `l1_a_newer_schema_is_refused_with_a_remedy`,
`l1_a_snapshot_is_written_before_migrating_a_populated_store`,
`p7_g2_a_hard_kill_at_startup_leaves_a_migratable_store`.

---

## Things that look like bugs and are not

- **A restart does not lose messages, and never needs repair.** Ten hard kills
  in a row are a test (`l5_a_hard_kill_never_needs_manual_repair_to_restart`).
  If data is missing after a restart, suspect the volume, not the hub.
- **Order is by insertion, not by timestamp.** A clock stepping backwards
  after a power cut cannot reorder delivery — ids are ULIDs but ordering comes
  from the rowid (`l1_delivery_order_survives_a_clock_that_steps_backwards`).
- **A binary payload shows as "binary payload (N bytes)"** rather than
  mangled text. A payload truncated at the display cap says how much is
  hidden. Neither is data loss; the store has the bytes.
- **`kyu.events` never fills up from its own housekeeping.** Retention
  collecting it is logged, not republished — that loop was a real defect,
  found and fixed, and is now pinned by
  `p7_collecting_the_events_topic_does_not_feed_itself`.
