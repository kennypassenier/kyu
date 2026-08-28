# mailbox — operations runbook

Numbered procedures for the things you do to a running hub. Each one says
what it changes and how you know it worked. Written in Phase 8; every
procedure was either executed against a real container or is derived from a
test that executes it.

Assumes `HUB=http://hub.lan:8080` and, on a protected hub,
`TOKEN=<your MAILBOX_TOKEN>` with `-H "authorization: Bearer $TOKEN"` on every
call.

---

## 1 · First deployment

**Through the homelab** (supported route):

1. In the homelab wizard, pick the `mailbox` preset.
2. Before deploying, write the stack's `.env`:
   ```bash
   echo "MAILBOX_TOKEN=$(openssl rand -hex 24)" >> stacks/<name>/mailbox/.env
   echo "MAILBOX_SECRET_KEY=$(openssl rand -hex 32)" >> stacks/<name>/mailbox/.env
   ```
   Both or neither. One without the other refuses to start, by design.
3. Deploy. The preset binds `/appdata/<stack>/mailbox-config`, which is what
   restic backs up, and carries `com.homelab.backup.pause=true`.
4. Verify: `curl -sf $HUB/healthz` returns `{"status":"ok",…}`.
5. Open `/login`, paste the token, tick "stay logged in".

**Standalone:**

```bash
docker compose up -d
curl -sf localhost:8080/healthz
```

Data lands in the `mailbox-data` named volume. Do not "fix" that into a bind
mount without reading the comment in `compose.yml` first — the image runs as
uid 65532 and a bind mount makes the hub refuse to start until you chown the
directory. That refusal was reproduced on 2026-08-28, not theorised.

---

## 2 · Upgrade

1. Tag the release in this repo: `git tag v0.2.0 && git push origin v0.2.0`.
   GitHub Actions publishes `ghcr.io/kennypassenier/mailbox:0.2.0` and
   `:latest`.
2. Write the GitHub Release yourself — `gh release create v0.2.0` — because
   the workflow publishes the image, not the release.
3. **Deployed via the homelab:** nothing to do. The nightly run updates it and
   rolls back on a failed health check (`com.homelab.update.policy=auto`).
   Immediate instead: `homelab update stacks/<name>`.
4. **Standalone:** `docker compose pull && docker compose up -d`.
5. Verify: `curl -sf $HUB/healthz`, and check the log line `schema migrated`
   if the schema moved.

**What happens to your data.** A populated store is snapshotted with
`VACUUM INTO` *before* any migration runs, so a bad upgrade stays reversible:
roll the image tag back and restore the snapshot. A newer schema than the
binary understands is refused with a remedy rather than guessed at — which is
why the rollback needs the snapshot rather than just the old image.

**Proven by:** `l1_a_snapshot_is_written_before_migrating_a_populated_store`,
`l1_a_newer_schema_is_refused_with_a_remedy`, and `scripts/container-smoke.sh`,
which runs an existing volume against a freshly built image because that is
the only place migration, snapshot and the healthcheck start-period meet the
way a real pull does.

---

## 3 · Take a backup by hand

```bash
curl -X POST -H "authorization: Bearer $TOKEN" $HUB/api/backup
```

Writes a consistent copy beside the store while the hub keeps serving, opens
it, integrity-checks it, and answers with the path and the restore procedure.
It refuses to overwrite an existing file.

A backup that will not restore is not a backup, which is exactly what its test
asserts: take one under concurrent load, restore it into a fresh directory,
deliver a message out of it.

**Proven by:** `l8_a_backup_taken_under_load_restores_to_a_working_store`,
`l8_a_backup_never_overwrites_an_existing_file`,
`p7_g16_a_corrupt_backup_target_is_not_reported_as_a_backup`.

## 4 · Restore

1. Stop the hub. `docker compose down`, or the homelab's own restore flow,
   which quiesces for you.
2. Put the backup file in place of `mailbox.db` in the data directory.
   Remove any `mailbox.db-wal` and `mailbox.db-shm` beside it — they belong
   to the old file and will confuse SQLite about what it is looking at.
3. Start the hub.
4. Verify: `curl -sf $HUB/healthz`, then poll a subscription you know had a
   backlog and check a message comes out.

**Through the homelab** this is a first-class operation — choose snapshot,
quiesce, restore, restart, verify — with a restore drill scheduled quarterly.
Use that rather than these steps when the hub is deployed there.

---

## 5 · Rotate the tokens

**The login token (`MAILBOX_TOKEN`) — safe.** Change it in the `.env` or
compose file and restart. App tokens keep working; you just log in with the
new value.

**The encryption key (`MAILBOX_SECRET_KEY`) — destructive.** Every stored app
token becomes unreadable. The apps page will list them as `unreadable`.
Recovery: revoke each one there and generate a replacement, then update the
apps that used them.

That asymmetry is the entire reason these are two variables rather than one
derived from the other: rotating a leaked password must not silently take
every integration down with it.

**Proven by:** `p7_another_key_cannot_open_it_and_says_what_to_do` (the error
names the variable and says to revoke and regenerate),
`p7_a_token_without_a_key_refuses_to_start_and_hands_over_a_key`.

## 6 · Add or revoke an app

1. Open `/apps` (you must be logged in).
2. Type a name — lowercase letters, digits, dots, underscores, hyphens — and
   press **Generate token**.
3. *Copy* puts the whole working command on your clipboard without showing
   the token; *Reveal* shows it for ten seconds.
4. Revoking takes effect on the **very next request**. There is no cache to
   wait out, because "revoked but still working for another minute" is not
   something anyone wants to reason about mid-incident.

A revoked name becomes available again; a live one is refused with `409`.

**Proven by:** `p7_an_app_token_works_and_stops_working_the_moment_it_is_revoked`,
`p7_two_apps_cannot_share_a_name_and_a_revoked_name_can_be_reused`.

---

## 7 · Deal with dead letters

1. Open the topic's dashboard page. Dead letters are listed with their
   payload, when they died, and how many attempts they burned.
2. Fix whatever made the consumer fail.
3. Press **Requeue**. Attempts reset, so the fixed consumer gets a clean run.

Nothing is lost while you take your time: a dead letter waits, and survives
restarts. If the payload can never work, leave it dead — that is what the list
is for.

Over the API instead:

```bash
curl "$HUB/api/t/<topic>/subs/<sub>/dead"
curl -X POST "$HUB/api/t/<topic>/subs/<sub>/dead/<id>/requeue"
```

## 8 · Bring back an archived subscription

```bash
curl -X POST "$HUB/api/t/<topic>/subs/<sub>/unarchive"
```

The response says whether it actually changed anything. What it does **not**
do is give back the backlog: archiving settled those deliveries as `lapsed`,
and that is deliberate — whoever comes back needs to learn their backlog
lapsed rather than silently believing they saw everything.

To stop it happening again, give that subscription its own thresholds:

```bash
curl -X PUT -H 'content-type: application/json' \
     -d '{"idle_flag_ms":2592000000,"idle_archive_ms":7776000000}' \
     "$HUB/api/t/<topic>/subs/<sub>/policy"
```

**Proven by:** `p7_g7_the_unarchive_endpoint_reports_whether_it_changed_anything`,
`p7_g4_lapsed_deliveries_stay_lapsed_after_an_unarchive`,
`l6_a_subscription_can_set_its_own_idle_thresholds`.

---

## 9 · Ship logs to Loki

Set `MAILBOX_LOG_FORMAT=json` and restart. One JSON object per line; the
human-readable format stays the default. `MAILBOX_LOG` takes a `tracing`
filter (`info`, `mailbox=debug`, …).

Payloads never appear in a log line, in either format. Asserted by
`p7_g9_payloads_never_reach_the_logs_or_the_metrics`.

## 10 · Prove the whole thing still works

```bash
docker build -t mailbox:smoke .
bash scripts/container-smoke.sh mailbox:smoke
```

Walks the three verbs through the real image, restarts the container, runs the
old volume against a freshly built image, then starts a second protected
container and checks that a tokenless publish is refused, a good token works,
monitoring stays open, the login page's assets are inside the image, and a
half-configured door refuses to boot.

Takes a few minutes and needs Docker. This is the check to run before you
believe anything about a deployment.
