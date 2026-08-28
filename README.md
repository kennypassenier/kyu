# mailbox

A self-documenting durable message hub for a homelab. Any script can
send with one `curl`, any worker can receive and acknowledge with two,
nothing is silently lost, and the dashboard doubles as the
documentation.

> **Status: feature-complete, not yet released.** Every frozen feature is
> built and under test (190 tests, CI green on every push). What has *not*
> happened: no version has been tagged, so no image exists on GHCR yet, and
> the hub has not run anywhere but a test container. This README claims only
> what the code does — the honesty pass this line used to promise was done on
> 2026-08-28, and it found this very block claiming the message API did not
> exist.

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

## The door

Out of the box mailbox has **no authentication**, which is a real choice for
a hub on a network nothing else reaches — and one it will not let you make
by accident: it warns on every startup and puts a banner on every dashboard
page saying so.

To put a token on it, set both of these and restart:

```bash
MAILBOX_TOKEN=$(openssl rand -hex 24)        # what you log in with
MAILBOX_SECRET_KEY=$(openssl rand -hex 32)   # encrypts per-app tokens
```

One without the other refuses to start, and the error prints a generated key
you can paste. With them set:

- **Scripts** send `-H 'authorization: Bearer <token>'`. The dashboard prints
  the whole command for you, token included.
- **You** get a login page with a remember-me box and a logout button.
- `/healthz` and `/metrics` stay **open**, so Uptime Kuma and Grafana keep
  working without changes. Neither exposes a payload.

### Per-app tokens

The dashboard's **Apps** page registers an app and generates a token for it.
Giving each program its own means you can revoke one without touching the
others, and revocation takes effect on the very next request. On a topic page
you can switch which app's token the printed commands carry.

Tokens are shown masked. **Copy** puts the whole working command on your
clipboard without ever displaying it; **Reveal** shows it for ten seconds.
That protects against someone glancing at your screen — not against someone
who has already logged in, which is by design.

### Which value to rotate

`MAILBOX_TOKEN` and `MAILBOX_SECRET_KEY` do different jobs, and the
difference bites exactly once:

- Rotating **`MAILBOX_TOKEN`** is safe. App tokens keep working; you just log
  in with a new value.
- Rotating **`MAILBOX_SECRET_KEY`** makes every stored app token unreadable.
  The apps page will show them as `unreadable`; revoke and re-issue.

That is precisely why the key is a separate variable rather than derived from
the token: so that rotating a leaked password does not silently take every
integration down with it.

## Releases and updates

Pushing a version tag publishes a Docker image to GHCR:

```bash
git tag v0.1.0 && git push origin v0.1.0
```

`.github/workflows/release-image.yml` then builds and pushes
`ghcr.io/kennypassenier/mailbox:0.1.0` and `:latest`. The workflow is taken
from the homelab's `templates/rust-service/`, so every one of these Rust
repos ships the same way — one shape to remember instead of four.

Two things that are deliberately *not* automatic:

- **The GitHub Release itself.** The workflow publishes the image, not a
  release. Writing release notes is a human act; `gh release create` at the
  moment you mean it.
- **Deployment.** The LXC pulls `:latest` through compose. Deployed via the
  homelab preset it also carries `com.homelab.update.policy=auto`, so the
  nightly run updates it and rolls back on a failed health check.

One unverified detail, carried over from the homelab guide and repeated here
so nobody has to rediscover it: that guide says the GHCR package is created
**private** even on a public repo and has to be flipped to public once. It
has never been checked against a real package. The first tag settles it.

## Backups

The store is one SQLite file. Two routes, and they are not equivalent:

**Through the homelab** (the supported route). `presets/mailbox/` in the
homelab repo binds the data directory under `/appdata/<stack>/mailbox-config`,
which is what restic walks: encrypted, off-site, 7 daily / 4 weekly / 3
monthly, with a restore drill scheduled quarterly. The preset also carries
`com.homelab.backup.pause=true`, which stops the container while the snapshot
is taken — SQLite copied mid-write is not a database.

**Standalone** (this repo's `compose.yml`). It uses a named volume, on
purpose: the image is distroless and runs as uid 65532, and a bind mount does
not inherit that ownership, so `- ./data:/data` makes the hub refuse to start
until you chown the directory. The cost is that restic cannot see a named
volume — so on the standalone route, backups are yours to arrange:

```bash
curl -X POST -H "authorization: Bearer $MAILBOX_TOKEN" http://hub.lan:8080/api/backup
```

That writes a consistent copy beside the store while the hub keeps serving,
opens it, integrity-checks it, and answers with the path and the restore
procedure. A backup that will not restore is not a backup, which is what its
test asserts: take one under load, restore it into a fresh directory, deliver
a message out of it.

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
| `MAILBOX_TOKEN` | *(none)* | The token you log in with and that scripts send. Unset means no door at all. |
| `MAILBOX_SECRET_KEY` | *(none)* | 64 hex characters. Encrypts per-app tokens. Required whenever `MAILBOX_TOKEN` is set. |
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

**If you are using mailbox**, these are the ones you want:
[USER_GUIDE.md](docs/USER_GUIDE.md) (every feature with a command you can
paste, and where each claim is proven),
[OPERATIONS_RUNBOOK.md](docs/OPERATIONS_RUNBOOK.md) (numbered procedures:
deploy, upgrade, back up, restore, rotate tokens),
[DEBUGGING_GUIDE.md](docs/DEBUGGING_GUIDE.md) (symptom → cause → fix).

**If you are changing mailbox**, add
[ARCHITECTURE_REFERENCE.md](docs/ARCHITECTURE_REFERENCE.md) (the system as
built) and [TEST_PLAN.md](docs/TEST_PLAN.md) (what is proven where, and what
is deliberately not covered).

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

The Rust version is pinned in `rust-toolchain.toml` and CI asks for that same
version rather than for "stable". Without that, a green gate here did not
predict a green build there — which stopped being theoretical the day 1.98
added a lint 1.97 had never heard of.

## License

MIT OR Apache-2.0, at your option.

`static/bootstrap.min.css` is Bootstrap 5.3.3, copyright 2011-2024 The
Bootstrap Authors, MIT licensed. It is vendored rather than loaded from a CDN
so the hub works on a network with no route to the internet, and so opening
the dashboard tells nobody outside that you did. Verified on download against
the published integrity hash
`sha384-QWTKZyjpPEjISv5WaRU9OFeRpok6YctnYmDr5pNlyT2bRjXh0JMhjY6hW+ALEwIH`.
Note that `cargo-deny` does not police it — it is not a crate — so bumping it
is a manual, deliberate act.
