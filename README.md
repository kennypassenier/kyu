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

Configuration is environment-only: `MAILBOX_LISTEN`,
`MAILBOX_DATA_DIR`, `MAILBOX_MAX_BODY_BYTES`, `MAILBOX_LOG`. Everything
per-topic or per-subscription is policy and lives in the database.

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
