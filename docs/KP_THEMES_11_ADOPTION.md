# kp-themes 11.0.0 adoption map

Kenny, 2026-10-19: every kyu page uses as many kp-themes 11.0.0 components
as make sense, every option to the full; usability and cool-factor decide.
This map is the source for the review demo
(`kp-themes/research/kyu-11/`) and, once approved, for the implementation
after chassis-rs ships kp-themes 11.0.0. Nothing here is decided until the
demo is approved.

Legend: **[now]** works on data kyu already has; **[server]** needs new
kyu backend work (endpoint or stored history) and is a new use case for
Kenny to judge.

## Every page

| Component | Use |
|---|---|
| `.kp-page-header` + `lede` | heading, one-sentence explanation, meta line per page [now] |
| `idmark` | the same coloured mark beside a topic everywhere (list, header, breadcrumbs); `idmarkColours` so marks never collide [now] |
| `freshness` (`data-kp-ago`) | every timestamp as "3 min ago" with dd/mm/yyyy HH:mm on hover; `stale-after` flags a quiet topic [now] |
| `palette` (Ctrl+K) + `intent` | jump to any topic or subscription, "Publish a test message", "Prune dead letters"; options rendered by the server [now] |
| `shortcuts` + `keys` | `g t` topics, `g a` apps, `/` search, `?` sheet, `p` publish box [now] |
| `auto-refresh` (region mode) + `tick` + `update` | live counts swapped in place, theme-native value change; pauses on hover and while a field has focus; remembered [now] |
| `datatable` | every table: sort, multi-sort, search, choice filters, column menu, cards on a phone, remembered [now] |
| `actions` (`data-kp-action`) | row buttons in fixed columns [now] |
| `.kp-empty` + `art` | every empty state full-footprint and centred (`done` art for "nothing dead") [now] |
| `tables` caption | every scroll region named [now] |
| `effects` headline reveal | once per session on the h1 [now] |
| `anchors` | `#` link on headings and on dead-letter / message rows; landing flashes the row, "no longer here" when it was requeued [now] |

## Topics (`/topics`)

| Component | Use |
|---|---|
| `kpi` toggle tiles | strip: topics, messages, backlog, dead; Backlog and Dead tiles filter the table [now] |
| `attention` band | "notify.kenny: 1 dead letter", "desktop flagged", ranked by severity, Fix = link to the row [now] |
| `barcell` | backlog and dead as proportional bars, amber at 75 %, red at 90 % of the largest [now] |
| `stackbar` | per topic: backlog / in flight / dead / lapsed as one bar with legend [now] |
| `command` | the "Start a topic" curl with prompt and Copy [now] |
| `uses-map` | topics → subscriptions as a line diagram ("who reads what") [now] |
| `graph` | alternative showpiece: hub-and-ring topology, link kind = subscription state [now] |
| `heat` | topic × hour publish / dead-letter heat map [server: hourly counts] |
| `chart` sparkline in the KPI tiles | publish rate over 24 h [server: metrics] |

## Topic (`/t/{topic}/dashboard`)

| Component | Use |
|---|---|
| `.kp-secret` (patterns) inside `command` | the four masked curl commands: Reveal with countdown, Copy without showing; deletes kyu's own `app.js` [now] |
| `steplist --journey` | the four commands as numbered steps publish → receive → ack [now] |
| `tour` | first-visit walkthrough of snippets, states, dead letters; `?tour` from a Help button [now] |
| `segmented` (multi, counts) | subscriptions filter All / active / flagged / archived with exact counts [now] |
| `menu-button` + `reason` | per-row More menu; disabled items say why ("archive it first") [now] |
| `undo-toast` (defer, ring) | Archive with undo [now] |
| typed-name confirm (`data-kp-confirm-type`) | Delete subscription [now] |
| `row-select` + `selection-bar` | dead letters: j/k, x, Shift range, "Requeue 5 selected" [server: bulk endpoint] |
| `problems` | "3 messages died on desktop", why, remedy with Requeue [now] |
| `code` cell mode | every payload: Pretty/Raw, fold, Copy, binary and truncated lines [now] |
| `master-detail` + `viewer` | recent messages list left, payload right, resizable, phone sheet [now] |
| `newpill` | "2 new messages" on the live list [now with auto-refresh] |
| `json-field` + `autosize` + `flash` + `forms` | publish box: live JSON check with line/column, Format, grows, Ctrl+Enter, remembered draft, button says Published or the refusal under it [now] |
| replay a message | publish a recent message again from its row [server: small, a new use case] |
| `gantt` / `request-inspector` | delivery attempts per message: claimed, lease expired, nack, dead [server: attempt history] |

## Subscription (`/t/{topic}/dashboard/subs/{name}`)

| Component | Use |
|---|---|
| `.kp-kv` | policy as label/value grid, "(default)" as a muted aside [now] |
| `bullets` | oldest unacked age against the lease, attempts against max attempts [now] |
| `progressbar` | backlog draining [now] |
| `datatable` + `row-select` + `code` | backlog with bulk delete [server: bulk endpoint] |
| `uptime` | 90-day delivered vs dead strip [server: daily stats] |

## Not used, and why

console, contrast, countdown, dated-options, datepicker, diagnostics,
docpreview, focus-steps, gallery, gridlayout, availability, calm-tones,
checklist, claim, compare, colorpicker, quote, rating, timer, week,
weekdays, wheel, upload, transfer, print, thread, unsaved, wizard,
quantity, slider, sentence, runs, snooze, timefield, version-delta: no kyu
page has that shape today. theme-picker, nav-fold, sidenav, tabbar, calm,
passkeys, no-flash, lazy-register: owned by the chassis-rs layout.
