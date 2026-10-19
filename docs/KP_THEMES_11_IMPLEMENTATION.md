# kp-themes 11.0.0 implementation contract

Kenny approved the five review parts on 2026-10-19 02:38 (formal; one
section, the overview graph's loading, goes to a second round). This file
is the contract between the backend work and the four page builds, so they
can run side by side. The approved demos are the visual source and are
implemented exactly: `~/Projects/kp-themes/research/kyu-11-<part>/`.

| Part | Demo | kyu page | Template | Own files |
|---|---|---|---|---|
| 1 across every page | kyu-11-overview | every page | `templates/_kp.html` (new, included by every page) | `static/common.js`, `static/kyu.css` |
| 2 Topics | kyu-11-topics | `/topics` | `templates/topics.html` | `static/topics.js`, `static/topics.css` |
| 3+4 Topic | kyu-11-topic-subs, kyu-11-topic-messages | `/t/{topic}/dashboard` | `templates/topic.html` | `static/topic.js`, `static/topic.css` |
| 5 Subscription | kyu-11-subscription | `/t/{topic}/dashboard/subs/{name}` | `templates/subscription.html` | `static/subscription.js`, `static/subscription.css` |

Defaults Kenny changed: the replay refusal is explained in a toast (every
theme); the subscriptions table uses Comfortable rows and the Chips filter
look (formal). Every other option keeps the demo's default and stays a
switchable option where the component offers one.

kp-themes modules are served by the kit at `/static/kp/js/<name>.js`
(chassis-rs 3.8.0, `webapp` feature: `dashboard` alone serves only 7 modules); a page loads
`/static/kp/js/auto.js` as a module and calls the module's own attach or
set functions for data it fetches. kyu's own files are on
`/assets/<file>?v={{ kyu_assets }}`.

## Data the pages fetch (dashboard router, behind the login)

All JSON. Times are Unix milliseconds; the page formats them
dd/mm/yyyy HH:mm in Europe/Brussels. Errors are `{ "error", "remedy" }`
with a 4xx/5xx status, the shape kyu's `ApiError` already writes.

| Route | Answer | Serves |
|---|---|---|
| `POST /t/{topic}/dashboard/bulk/requeue` body `{ "subscription", "ids": [..] }` | `{ "done": [id], "refused": [{ "id", "reason" }] }` | topic dead letters, selection bar |
| `POST /t/{topic}/dashboard/bulk/delete` body `{ "subscription", "ids": [..] }` | same | topic dead letters, subscription backlog |
| `POST /t/{topic}/dashboard/messages/{id}/replay` | `{ "id": "<new id>" }` or error | topic recent messages |
| `GET /t/{topic}/dashboard/subs/{name}/attempts?id=<message id>` | `{ "message", "attempts": [{ "n", "claimed_at", "ended_at", "outcome": "acked"\|"nacked"\|"expired"\|"dead"\|"in_flight", "app", "reason" }] }` | subscription timeline + request inspector |
| `GET /t/{topic}/dashboard/subs/{name}/daily?days=90` | `{ "days": [{ "day": "YYYY-MM-DD", "delivered", "dead" }] }` | subscription uptime strip |
| `GET /dashboard/stats/hourly?hours=24` | `{ "hours": [ms..], "topics": [{ "topic", "published": [n..], "dead": [n..] }] }` | topics heat map |
| `GET /dashboard/stats/totals?hours=24` | `{ "samples": [{ "at", "messages", "backlog", "dead", "in_flight" }] }` | topics KPI trend tiles |
| `GET /t/{topic}/dashboard/retention` | `{ "dead_letter_days" }` (or what kyu already keeps) | dead-letter problem line |

The existing form posts (requeue, delete, archive, publish) keep working as
the no-script path; the pages may call them with `fetch` and
`Accept: application/json`, in which case they answer `{ "ok": true }` or
the error shape instead of a redirect.

## Storage (backend)

Schema 7 adds, with a migration that keeps the store's existing data:

- `attempts`: one row per delivery attempt (topic, subscription, message
  id, attempt number, claimed_at, ended_at, outcome, app name, reason),
  written where a claim, ack, nack, lease expiry and dead-lettering already
  happen; pruned with the message it belongs to.
- `stats_hourly` (topic, hour, published, dead) and `stats_daily` (topic,
  subscription, day, delivered, dead): counters bumped on publish, ack and
  dead-lettering; hourly kept 30 days, daily 400 days, pruned by the sweeper.
- `stats_samples` (at, messages, backlog, dead, in_flight): the hub totals,
  sampled by the sweeper every 5 minutes, kept 48 hours.

Every growing table has its cap from the start.
