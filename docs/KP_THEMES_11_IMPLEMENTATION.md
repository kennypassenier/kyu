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

## Chassis 3.11.0 - 3.13.0 adoption

- **Storage page (3.11.0)**: `kyu::storage::declare` calls `App::app_db` on
  the SAME `kyu.db` with kyu's own `MIGRATIONS` list as the kit's
  migrations. `Store::open` migrates first (snapshot included), so the kit
  finds the file at the current version and applies nothing; no table moves,
  the schema and `/api/backup` (VACUUM INTO) are untouched, and kyu's
  sweeper caps stay the only automatic pruning. Kinds: `messages`,
  `attempts`, `stats_hourly`, `stats_samples` (no `finished_by_definition`).
  `stats_daily` is not a kind: its `day` is a day number the kit cannot
  compare. Cost to know: two openers share the file (WAL handles it) and the
  kit adds `chassis_meta` and `chassis_prefs` outside the numbered schema.
- **Preferences (3.12.0)**: `App::sync_browser_keys` declares the prefix
  `kp-remember:` (every `data-kp-remember` choice, the tour-done flag
  included). The theme key stays in the browser (read before login).
- **Trusted admins (3.13.0)**: no code. The CT 109 environment needs
  `KYU_TRUSTED_ADMINS=10.10.10.10` (Kenny's PC); that is the homelab's deploy
  config, not kyu's code, and must not name the Traefik address that is
  `KYU_TRUSTED_PROXIES` (10.10.10.4).

## Next kp-themes version

kp-themes main (860e46ec) adds `.kp-formgrid`, the required layout for
label | value | control rows, and a 1 px alignment check. When chassis-rs
ships a kp-themes release with it, move kyu's hand-made grids onto it:
the phone card label | value grid in `topics.css` and `subscription.css`,
the topic cell grid, the palette rows and the who-reads-what tracks in
`kyu.css`, and the Topic page's command and publish grids in `topic.css`.
The same release carries formal's graph loading (no kyu change needed).

With kp-themes 11.1.0 (kp-themes main 612cf357), replace kyu's card-failed
fallbacks with the components' own failed plates and Retry events:
`setHeat(el, {state})` → `kp-heat-retry`, `setMatrix(el, {state})` →
`kp-matrix-retry`, `setGanttState(el, state, words)` → `kp-gantt-retry`,
`setGraphState(el, state, words)` → `kp-graph-retry`,
`setUptime(el, {state, error})` → `kp-uptime-retry`,
`setTrendData(figure, null | {points: []} | {error})` → `kp-chart-retry`,
`setProgressFailed(bar, {title})` → `kp-progressbar-retry`. Heat, matrix and
uptime need `attachFootprints()`, which auto.js already runs.

Also with 11.1.0: a `data-kp-submit` form no longer wipes a flash button's
stack (kp-themes c3c26699), so the Topic page's publish box gets its
approved flash words back ("Published" / the refusal line under the
button) next to the busy submit, instead of the toast and form summary
kyu uses on 11.0.0.
