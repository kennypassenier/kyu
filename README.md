# mailbox

A self-documenting durable message hub for a homelab. Any script can
send with one `curl`, any worker can receive and acknowledge with two,
nothing is silently lost, and the dashboard doubles as the
documentation.

> **Status: in development (L0 of 9).** The walking skeleton builds,
> serves `/healthz` and ships as a container; the message API itself does
> not exist yet. This README will only ever claim what the code
> actually does — the honest pass happens in Phase 8.

## The idea

Publishing goes to a topic; consuming happens under a *subscription
name*. That one parameter covers both delivery patterns:

- **different names** on a topic each receive every message and
  acknowledge independently (fan-out)
- **the same name** from several processes competes, so each message goes
  to exactly one worker (load balancing)

Delivery is at-least-once: an unacknowledged message comes back, and one
that exhausts its retries lands in a visible dead-letter list instead of
disappearing.

```bash
# publish
curl -H 'content-type: application/json' \
     -d '{"title":"Backup done"}' http://hub.lan/t/notify.kenny

# receive (long-poll) and acknowledge
curl -s "http://hub.lan/t/notify.kenny/next?as=printer&envelope=json"
curl -sX POST "http://hub.lan/t/notify.kenny/ack/<id>?as=printer"
```

## Running it

```bash
docker compose up -d
curl localhost:8080/healthz
```

## Configuration

Two layers, and the distinction matters: the **environment** configures the
process and sets hub-wide defaults, while **policy** lives in the database
and belongs to one topic or one subscription. Nothing that varies per
consumer is an environment variable.

### Environment — the process and its defaults

| Variable | Default | What it does |
|---|---|---|
| `MAILBOX_LISTEN` | `0.0.0.0:8080` | Address to bind. |
| `MAILBOX_DATA_DIR` | `/data` | Where the store lives. |
| `MAILBOX_MAX_BODY_BYTES` | `1048576` | Largest accepted payload; bigger ones are refused with 413, never trimmed. |
| `MAILBOX_LOG` | `info` | Log filter (`tracing` syntax). |
| `MAILBOX_LOG_FORMAT` | human | Set to `json` for one JSON object per line. |
| `MAILBOX_RETENTION_MS` | `604800000` (7 days) | Default retention. `never` keeps messages indefinitely. |
| `MAILBOX_IDLE_FLAG_MS` | `604800000` (7 days) | Unpolled for this long → flagged on the dashboard. |
| `MAILBOX_IDLE_ARCHIVE_MS` | `2592000000` (30 days) | Unpolled for this long → archived; outstanding messages are settled as `lapsed`. |

The two idle thresholds are **defaults, not laws** — see below.

### Policy — per subscription

```bash
curl -X PUT -d '{"ttl_ms":600000}' \
     http://hub.lan/api/t/notify.kenny/subs/tts/policy
```

| Field | Default | What it does |
|---|---|---|
| `lease_ms` | 30000 | How long a claimed message stays claimed before it is offered again. |
| `max_attempts` | 5 | Delivery attempts before the message is dead-lettered. |
| `backoff_ms` | 1000 | Base retry delay; the schedule is linear (1s, 2s, 3s…). |
| `ttl_ms` | none | Drop messages older than this — "relevant now or never". |
| `idle_flag_ms` | hub default | This subscription's own flag threshold. |
| `idle_archive_ms` | hub default | This subscription's own archive threshold. |

A policy write **replaces** the whole policy: a field you leave out returns
to its default. The response always states what is now in force, which
fields are explicit, and the resulting retry schedule.

The idle thresholds exist per subscription because the knowledge that a
consumer only polls monthly lives with that consumer, not with the hub:

```bash
# a monthly report consumer that must not be archived after 30 quiet days
curl -X PUT -d '{"idle_flag_ms":5184000000,"idle_archive_ms":15552000000}' \
     http://hub.lan/api/t/reports.monthly/subs/monthly-report/policy
```

### Policy — per topic

```bash
curl -X PUT -d '{"retention_ms":86400000}' http://hub.lan/api/t/notify.kenny/retention
curl -X PUT -d '{"keep_forever":true}'     http://hub.lan/api/t/print.receipt/retention
```

Retention never collects a message an active subscription is still holding,
however old it is — a consumer offline for a fortnight comes back to a
complete backlog.

## Development

This project follows a staged procedure; the paper trail is in `docs/`:
[SCOPE.md](docs/SCOPE.md) (goals and non-goals),
[FEATURES.md](docs/FEATURES.md) (the frozen feature list),
[ARCHITECTURE_DECISIONS.md](docs/ARCHITECTURE_DECISIONS.md) (frozen
decisions with the reasoning and the rejected alternatives), and
[REALIZATION_PLAN.md](docs/REALIZATION_PLAN.md) (milestones and status).

Gates are git-native, so they hold from any session or terminal. After
cloning:

```bash
git config core.hooksPath .githooks
```

A commit is refused unless `cargo fmt --check`, `cargo clippy -D
warnings` and `cargo test --all` pass, no string-built SQL appears in
`src/`, and the message names the feature IDs it implements.

## License

MIT OR Apache-2.0, at your option.
