# mailbox — user guide

Everything mailbox does, one feature at a time, with commands you can paste.
Written in Phase 8 from the code and tests, not from intent: every claim here
names where it is proven.

If you have three minutes and no memory of this project, read **K1–K3** and
stop. That is the whole product; the rest is what happens when something goes
wrong.

**Every example assumes a hub at `http://hub.lan:8080`.** If the hub has a
token (W2), add `-H "authorization: Bearer $MAILBOX_TOKEN"` to every command —
the dashboard prints them that way for you, so you never have to remember.

---

## The three verbs

### K1 · Publish

```bash
curl -H 'content-type: application/json' \
     -d '{"title":"Backup done"}' \
     http://hub.lan:8080/t/notify.kenny
```

A topic starts existing the moment something publishes to it. The response is
`201` with the message id. The payload is stored byte for byte and the content
type verbatim — mailbox never rewrites what you sent.

Once the response is in your hand, the message is on disk. Not "queued for
writing": a hard kill one millisecond later still delivers it.

**Proven by:** `l2_round_trip_in_raw_mode`, `l2_the_content_type_is_stored_verbatim`,
`l5_s4_every_confirmed_publish_survives_a_hard_kill`.

### K2 · Receive (long poll)

```bash
curl -s -D- -o message.body \
     "http://hub.lan:8080/t/notify.kenny/next?as=printer"
```

`as=` is the **subscription name**, and it is the only concept in mailbox you
have to understand:

- **Different names** on one topic each get every message, independently.
  That is fan-out.
- **The same name** from several processes competes — each message goes to
  exactly one of them. That is load balancing.

The payload comes back as the raw body; the metadata rides in headers
(`mailbox-id`, `mailbox-topic`, `mailbox-attempt`, `mailbox-published-at`).
Prefer one JSON document?

```bash
curl -s "http://hub.lan:8080/t/notify.kenny/next?as=printer&envelope=json"
```

The call waits up to 30 seconds for a message (`&wait=` to change it, max
300). A message published while you wait arrives at once rather than after the
timeout. Nothing waiting when the window closes: `204`.

**The one trap** (and the hub says so in a `mailbox-notice` header the first
time): a subscription starts existing when it *first polls*, and it only sees
what is published after that. Poll once before publishing the message you want
it to catch — or use `&from=beginning` to pull in what the topic still retains.

**Proven by:** `l2_round_trip_in_envelope_mode`, `l3_two_subscriptions_each_receive_every_message`,
`l3_competing_consumers_share_the_work_without_duplicating_it`,
`l2_a_message_published_mid_poll_arrives_at_once`,
`l2_a_new_subscription_starts_from_now_and_says_so`.

### K3 · Ack

```bash
curl -X POST "http://hub.lan:8080/t/notify.kenny/ack/<id>?as=printer"
```

Until you acknowledge, the message is yours on loan. Ack settles it for **your
subscription only** — a sibling subscription still has its own copy waiting.

**Proven by:** `l3_an_ack_settles_one_subscription_only`,
`l2_an_acked_message_never_returns_after_a_restart`.

---

## When things go wrong

### K5 · Leases and redelivery

Every claimed message carries a lease (30 s by default). Miss it — crash,
hang, power cut — and the message returns to the queue with its attempt count
raised, after a backoff. Delivery is **at-least-once**: build consumers that
tolerate seeing the same message twice.

Backoff is linear on purpose, not exponential: predictable, and needing no cap.

**Proven by:** `l4_ar9_claimed_to_pending_when_the_lease_expires`,
`l4_a_redelivered_message_waits_out_its_backoff_first`,
`l4_s2_a_killed_consumer_gets_its_message_redelivered`.

### K6 · Dead letters

After the attempts run out (5 by default) the message stops being retried and
lands in a dead-letter list. It is not deleted — a dead letter waits for a
human, and survives restarts.

See them on the topic's dashboard page, with their payload, or over the API:

```bash
curl "http://hub.lan:8080/api/t/notify.kenny/subs/printer/dead"
curl -X POST "http://hub.lan:8080/api/t/notify.kenny/subs/printer/dead/<id>/requeue"
```

Requeue resets the attempt count, so a fixed consumer gets a clean run. The
dashboard has a **Requeue** button that does the same thing.

**Proven by:** `l4_dead_letters_are_listed_with_their_payload_and_can_be_requeued`,
`l4_dead_letters_survive_a_restart`, `p7_p1_the_dashboard_shows_dead_letters_and_requeues_them`.

### W5 · Nack

Hand a message back without waiting out the lease:

```bash
curl -X POST "http://hub.lan:8080/t/notify.kenny/nack/<id>?as=printer"
curl -X POST "http://hub.lan:8080/t/notify.kenny/nack/<id>?as=printer&dead=true"
```

`dead=true` is the poison pill: skip the remaining attempts and go straight to
the dead-letter list. Use it when you can see the payload will never work.

**Proven by:** `l4_a_nack_returns_the_message_without_waiting_for_the_lease`,
`l4_a_poison_pill_nack_skips_the_remaining_attempts`.

### K7 · Per-subscription policy

Lease, attempts, backoff and TTL belong to a subscription, not to the hub:

```bash
curl "http://hub.lan:8080/api/t/notify.kenny/subs/printer/policy"

curl -X PUT -H 'content-type: application/json' \
     -d '{"lease_ms":300000,"max_attempts":10}' \
     "http://hub.lan:8080/api/t/notify.kenny/subs/printer/policy"
```

The response tells you the values in force, which of them you set explicitly,
and the retry schedule that results — so no number on the dashboard is
unexplained. A write **replaces every field**: one rule instead of two.

A policy that cannot work is refused with a remedy rather than accepted.

**Proven by:** `l4_the_policy_endpoint_reports_what_is_in_force_and_what_is_explicit`,
`l4_a_policy_write_replaces_every_field`, `l4_a_policy_that_cannot_work_is_refused_with_a_remedy`.

---

## History and housekeeping

### K9 · Retention · K8 · Replay

Messages are kept for 7 days by default, then collected — but **never** one
that an active or flagged subscription is still waiting for. A consumer
offline for a fortnight comes back to a complete backlog.

```bash
curl -X PUT -H 'content-type: application/json' -d '{"retention_ms":"never"}' \
     "http://hub.lan:8080/api/t/notify.kenny/retention"
```

Replay pulls what the topic still retains into a subscription:

```bash
curl "http://hub.lan:8080/t/notify.kenny/next?as=printer&from=beginning"
```

It is idempotent, runs in bounded batches, and cannot race the retention
sweep — both run on the single writer, so they are serialised.

**Proven by:** `l6_retention_never_collects_a_backlog_an_active_subscription_still_needs`,
`l6_replay_is_idempotent`, `l6_replay_sees_exactly_what_retention_kept`.

### K11 · Idle subscriptions

A subscription that stops polling is **flagged** after 7 days and **archived**
after 30. Archiving settles what it was holding as `lapsed` — recorded and
counted, never silently dropped. Polling clears a flag; it will **not** revive
an archive, because whoever comes back needs to learn their backlog lapsed:

```bash
curl -X POST "http://hub.lan:8080/api/t/notify.kenny/subs/printer/unarchive"
```

Both thresholds are hub-wide defaults *and* per-subscription overrides — a
consumer that only runs monthly says so in its own policy rather than dragging
the whole hub's settings with it.

**Proven by:** `l6_an_idle_subscription_is_flagged_then_archived`,
`l6_polling_clears_a_flag_but_will_not_revive_an_archive`,
`l6_a_subscription_can_set_its_own_idle_thresholds`.

### W4 · Delayed delivery

```bash
curl -H 'content-type: application/json' -d '{"job":"nightly"}' \
     "http://hub.lan:8080/t/printer.jobs?delay=3600000"
```

`?delay=` in milliseconds, or `?at=` for a Unix millisecond timestamp — giving
both is refused rather than silently preferring one. The message is durable
the moment it is accepted; only its *delivery* waits, and the due time is a
column rather than a timer in memory, so a restart neither loses the schedule
nor releases it early.

**Proven by:** `l8_a_delayed_message_is_durable_immediately_and_deliverable_later`,
`l8_a_schedule_survives_a_restart`, `l8_two_answers_to_when_are_refused`.

---

## Watching it

### K10 · The dashboard

`http://hub.lan:8080/` lists every topic; each topic has a page with its
subscriptions, backlogs, dead letters and recent messages.

The part that matters for coming back after three months: the topic page
prints the four commands **filled in with your own topic, your own last
payload and a subscription that actually exists**. Generic documentation is
what you skim; your own data is what you trust. One test reads the page the
way a browser shows it, pulls the printed command off it, and runs it.

There is also a box to publish a test message (W9), and a Requeue button per
dead letter.

**Proven by:** `l7_the_snippets_the_dashboard_prints_actually_work`,
`l7_the_topic_page_shows_subscriptions_backlogs_and_policy`,
`l7_the_test_publish_form_puts_a_real_message_on_the_topic`.

### W6 · Health · W1 · Metrics

```bash
curl http://hub.lan:8080/healthz
curl http://hub.lan:8080/metrics
```

`/healthz` answers `200` with `{"status":"ok","store":"writable","sweeper":"alive"}`,
or `503` with an `error` and a `remedy` when the store cannot be written or the
sweeper has stopped. Both endpoints stay open even on a hub with a token, so
Uptime Kuma and Grafana keep working unchanged.

`/metrics` exposes `mailbox_topics`, `mailbox_subscriptions`, `mailbox_messages`,
`mailbox_deliveries` (by state), `mailbox_store_bytes` and
`mailbox_sweeper_age_ms`. The last one is the alertable series: a stalled
sweeper makes messages *hang* rather than fail, which is otherwise invisible.

Payloads and tokens never reach a metric label. That is asserted, not assumed.

**Proven by:** `l5_healthz_reports_the_store_and_the_sweeper`,
`p7_g11_healthz_answers_503_when_the_store_refuses_writes`,
`l8_metrics_expose_the_series_that_reveal_a_silent_failure`,
`p7_g9_payloads_never_reach_the_logs_or_the_metrics`,
`p7_no_token_reaches_the_metrics_or_any_page_in_the_clear`.

### W11 · The hub's own events

mailbox publishes its own events onto the ordinary topic `mailbox.events`, so
consuming them needs no special integration — subscribe exactly the way you
subscribe to anything else:

```bash
curl "http://hub.lan:8080/t/mailbox.events/next?as=ha&envelope=json"
```

Events: `message.dead_lettered`, `message.expired`, `subscription.flagged`,
`subscription.archived`, `subscription.unarchived`.

One rule is load-bearing: an event *about* a `mailbox.*` topic is logged, never
republished. Without it, a broken consumer of `mailbox.events` dead-letters,
which emits an event onto the same topic, which dead-letters — a self-sustaining
message generator.

**Proven by:** `l6_the_hub_publishes_its_own_events`,
`l6_events_about_the_events_topic_are_logged_not_republished`,
`p7_collecting_the_events_topic_does_not_feed_itself`.

---

## W2 · The door

See the README's "The door" section for setup. In daily use:

- Scripts send `-H 'authorization: Bearer <token>'`.
- You log in at `/login` with a remember-me box, and log out from the navbar.
- `/apps` registers an app and generates a token for it; revoking one takes
  effect on the very next request, with no cache to wait out.
- Printed commands carry a **real, working** token, masked on screen. *Copy*
  puts the whole command on your clipboard without displaying it; *Reveal*
  shows it for ten seconds.

A hub with no token configured still starts — that is a legitimate choice for
a hub nothing else can reach — but says so on every startup and on every page,
so it can never be a surprise.

**Proven by:** the thirteen tests in `tests/p7_auth.rs`.

---

## What mailbox deliberately does not do

- **No exactly-once.** At-least-once is the contract; be idempotent (N4).
- **No routing or transformation.** The hub moves bytes; it does not inspect
  or rewrite them (N5).
- **No clustering.** One node, one file (N1).
- **No user management.** A token is admission, not authority — there are no
  roles and no per-topic permissions, ever (N2).
- **Never exposed to the internet.** LAN or VPN only (N3).
