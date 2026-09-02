# kyu — operations runbook

Numbered procedures for the things you do to a running hub. Each one says
what it changes and how you know it worked. Written in Phase 8; every
procedure was either executed against a real container or is derived from a
test that executes it.

Assumes `HUB=http://hub.lan:8080` and, on a protected hub,
`TOKEN=<your KYU_TOKEN>` with `-H "authorization: Bearer $TOKEN"` on every
call.

---

## 1 · First deployment

**Through the homelab** (supported route):

1. In the homelab wizard, pick the `kyu` preset.
2. Before deploying, write the stack's `.env`:
   ```bash
   echo "KYU_TOKEN=$(openssl rand -hex 24)" >> stacks/<name>/kyu/.env
   echo "KYU_SECRET_KEY=$(openssl rand -hex 32)" >> stacks/<name>/kyu/.env
   ```
   Both or neither. One without the other refuses to start, by design.
3. Deploy. The preset binds `/appdata/<stack>/kyu-config`, which is what
   restic backs up, and carries `com.homelab.backup.pause=true`.
4. Verify: `curl -sf $HUB/healthz` returns `{"status":"ok",…}`.
5. Open `/login`, paste the token, tick "stay logged in".

**As a plain binary under systemd** — what Kenny actually runs, on LXC 109
(`109-app-kyu`, `10.10.10.9`). No Docker on the box at all:

1. Take the binary out of the published image on a machine that has Docker,
   and copy it over. It is statically linked, so it needs nothing installed:
   ```bash
   id=$(docker create ghcr.io/kennypassenier/kyu:1.0.1)
   docker cp "$id":/usr/local/bin/kyu ./kyu && docker rm "$id"
   scp kyu root@<proxmox>:/tmp/kyu
   ssh root@<proxmox> 'pct push 109 /tmp/kyu /usr/local/bin/kyu --perms 755'
   ```
2. A system user that owns only its data, and a config file only it can read:
   ```bash
   adduser --system --group --home /var/lib/kyu --no-create-home kyu
   install -d -o kyu -g kyu -m 0750 /var/lib/kyu
   install -d -o root -g kyu -m 0750 /etc/kyu
   umask 077
   printf 'KYU_TOKEN=%s\nKYU_SECRET_KEY=%s\nKYU_LISTEN=0.0.0.0:8080\nKYU_DATA_DIR=/var/lib/kyu\nKYU_LOG=info\n' \
     "$(openssl rand -hex 24)" "$(openssl rand -hex 32)" > /etc/kyu/kyu.env
   chown root:kyu /etc/kyu/kyu.env && chmod 0640 /etc/kyu/kyu.env
   ```
3. `kyu.service` with `Restart=always` and `StartLimitIntervalSec=0` — the
   second matters as much as the first, because systemd otherwise gives up
   after a few restarts in a short window and turns a transient fault into a
   permanent outage. The unit also carries the namespace hardening
   (`ProtectSystem=strict` and friends), which in an **unprivileged LXC needs
   `features: nesting=1` on the container** or every start fails with
   `Failed to set up mount namespacing`. That is measured, not guessed.
4. `systemctl enable --now kyu`, then `curl -sf http://<host>:8080/healthz`.

**Standalone with Docker:**

```bash
docker compose up -d
curl -sf localhost:8080/healthz
```

Data lands in the `kyu-data` named volume. Do not "fix" that into a bind
mount without reading the comment in `compose.yml` first — the image runs as
uid 65532 and a bind mount makes the hub refuse to start until you chown the
directory. That refusal was reproduced on 2026-08-28, not theorised.

---

## 2 · Cut a release (how a new image comes into existence)

Kenny asked for this written down rather than remembered, and it is short on
purpose: **tagging is the whole trigger.** There is no button to press on
GitHub and no image to build by hand.

1. **Decide the version.** Semver over the HTTP contract — the three verbs,
   their parameters, their response shapes, and the environment variables.
   Breaking any of those is a major. Adding an endpoint or a parameter is a
   minor. Fixing behaviour without changing the contract is a patch. The
   dashboard's HTML and the on-disk schema are explicitly *not* part of it.
2. **Bump it in two places, in one commit:** `Cargo.toml`'s `version`, and a
   new section at the top of `CHANGELOG.md`. They drift the moment you do
   them separately.
3. **Push, and wait for CI to go green.** Tagging a red commit produces a
   release you then have to withdraw.
4. **Tag and push the tag:**
   ```bash
   git tag v1.2.3
   git push origin v1.2.3
   ```
   The `v` prefix is what `.github/workflows/release-image.yml` triggers on
   (`tags: ["v*"]`). Nothing else starts it — not a push to main, not a
   release created by hand.
5. **Watch it:** `gh run list --workflow=release-image` — it takes a few
   minutes. It publishes two tags of the same image:
   `ghcr.io/kennypassenier/kyu:1.2.3` and `:latest`.
6. **Write the GitHub Release yourself:**
   ```bash
   gh release create v1.2.3 --title "v1.2.3" --notes-file <(sed -n '/## \[1.2.3\]/,/## \[/p' CHANGELOG.md)
   ```
   The workflow deliberately does not do this. Release notes are the one part
   a human adds something to, and automating them from commit subjects
   produces a list nobody reads.
7. **Verify the image exists and is pullable:**
   ```bash
   docker pull ghcr.io/kennypassenier/kyu:1.2.3
   ```
   The package is linked to this repository and takes its visibility, so a
   public repo yields a package the homelab host can pull anonymously.

**Deviation from the procedure, recorded rather than forgotten:** Phase 9 of
the dev procedure asks for tag → build → *checksum manifest* → GitHub Release.
kyu ships no self-updating binary — updates arrive as a new image, whose
integrity Docker already verifies by digest — so a checksum manifest would be
a file with no reader. See AR12.

## 2b · Upgrade a running hub

1. Cut the release as in §2, so the new image exists.
2. **Deployed via the homelab:** nothing to do. The nightly run updates it and
   rolls back on a failed health check (`com.homelab.update.policy=auto`).
   Immediate instead: `homelab update stacks/<name>`.
3. **Standalone with Docker:** `docker compose pull && docker compose up -d`.
4. **Plain binary under systemd** — extract the new binary as in §1, then:
   ```bash
   ssh root@<proxmox> '
     pct exec 109 -- systemctl stop kyu
     pct push 109 /tmp/kyu /usr/local/bin/kyu --perms 755
     pct exec 109 -- systemctl start kyu
     pct exec 109 -- /usr/local/bin/kyu --version'
   ```
   Walked for real on 2026-08-28 going from 1.0.0 to 1.0.1: an unacknowledged
   message was still waiting afterwards.
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

### Stopping the hub (2.1.0 and later)

`systemctl stop kyu`, `docker stop` and Ctrl-C all send a signal kyu now
catches (W12). It finishes the requests in flight — a long-poll consumer is
the normal state here, so that matters — folds the write-ahead log back into
the database, truncates it, and exits **0**. After that the files in the data
directory stand still, so a plain `tar` or `cp` of it is a restorable copy.

`KYU_SHUTDOWN_TIMEOUT_MS` (default 10000) bounds it. Blowing the budget logs
one loud line naming what was still open and exits 0 anyway: a stop that
hangs is worse than one that is incomplete, and a non-zero code would make
systemd report a clean stop as a crash.

**Give the unit a `TimeoutStopSec` above that budget**, so systemd's own
patience is never the thing that ends a shutdown:

```ini
# in kyu.service, alongside Restart=always
TimeoutStopSec=30
```

Before 2.1.0 there was no handler at all and the process died where it
stood. That was never a data risk — `l5_crash.rs` kills the real binary with
SIGKILL ten times in a row and it restarts without repair — but the log was
always mid-write, which is why the homelab's nightly `tar` of CT 109 failed
with `kyu.db-wal: file changed as we read it`.

---

### Where the deployment files live (2026-09-02)

`deploy/` in this repository holds the unit files, the backup script and the
alerting hook. Before that date they existed only on LXC 109 and in no
repository, which is how a broken one went unnoticed for two nights: nothing
versioned them, so no test, gate or review could look at them (F179).

**A timer firing says nothing about whether the work succeeded.** Ask about
the service, never the timer:

```bash
systemctl status kyu-backup.service          # not kyu-backup.timer
ls -lt "$KYU_DATA_DIR"/kyu.backup-*.db | head -1
```

Since 2026-09-02 a failure also announces itself: `OnFailure=` on the backup
service runs `kyu-alert`, which publishes the unit name, the host and the
last journal lines to the `ops.alerts` topic on this same hub. Proven by
reproducing the original fault — pointing the unit at an env file that does
not exist — and watching the message arrive.

Its limit, stated rather than hidden: if the hub itself is down there is
nowhere to publish and the alert only reaches the journal. That case belongs
to Uptime Kuma, which watches `/healthz`.

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
2. Put the backup file in place of `kyu.db` in the data directory.
   Remove any `kyu.db-wal` and `kyu.db-shm` beside it — they belong
   to the old file and will confuse SQLite about what it is looking at.
3. Start the hub.
4. Verify: `curl -sf $HUB/healthz`, then poll a subscription you know had a
   backlog and check a message comes out.

**Through the homelab** this is a first-class operation — choose snapshot,
quiesce, restore, restart, verify — with a restore drill scheduled quarterly.

### Moving or renaming a store

The database is three files, not one. In WAL mode most recent writes live in
`kyu.db-wal` until a checkpoint folds them back, so the main `kyu.db` can be a
few kilobytes while the log beside it holds everything. SQLite derives the log's
name from the database's, so the three names must stay in step.

- **Moving a store to another path:** move `kyu.db`, `kyu.db-wal` and
  `kyu.db-shm` together.
- **Renaming the database file:** rename all three to match
  (`x.db`, `x.db-wal`, `x.db-shm`). SQLite then finds the log and replays it on
  the next open.
- **Restoring a backup** is the one case where the log is deleted rather than
  carried: a `VACUUM INTO` backup is already complete, and a stale log beside it
  describes a different database. That is what §4 step 2 says.

Learned the hard way on 2026-08-29, during the rename from `mailbox` to `kyu`:
`mailbox.db` was moved to `kyu.db` and its 800 KB log was left behind, so the
hub came up on a 4 KB database — six topics and fourteen messages short, the
Home Assistant backlog among them. Nothing was lost, because a `VACUUM INTO`
backup taken minutes earlier and the orphaned log independently reconstructed
the identical state, and both were checked against each other before either was
installed. Take the backup first; it is what makes a mistake here survivable.
Use that rather than these steps when the hub is deployed there.

**On LXC 109** there are two things to restore from, and which one you want
depends on what broke:

- **The container is gone or unbootable** → the Proxmox backup job
  `kyu-109` (nightly 03:30, 7 daily + 4 weekly, on `local`). Restore the
  container; everything comes back with it.
- **The data is wrong but the container is fine** — a bad migration, a
  message you should not have deleted → the newest
  `/var/lib/kyu/kyu.backup-*.db`, written nightly at 03:00 by
  `kyu-backup.timer` and integrity-checked by the hub before it was
  reported. Follow step 2 above: stop the service, put it in place of
  `kyu.db`, delete the `-wal` and `-shm`, start again.

The 03:00 backup runs half an hour before the snapshot on purpose: whatever
state the live database is caught in, the snapshot always contains one file
the hub itself opened and checked.

---

## 5 · Rotate the tokens

**The login token (`KYU_TOKEN`) — safe.** Change it in the `.env` or
compose file and restart. App tokens keep working; you just log in with the
new value.

**The encryption key (`KYU_SECRET_KEY`) — destructive.** Every stored app
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

Set `KYU_LOG_FORMAT=json` and restart. One JSON object per line; the
human-readable format stays the default. `KYU_LOG` takes a `tracing`
filter (`info`, `kyu=debug`, …).

Payloads never appear in a log line, in either format. Asserted by
`p7_g9_payloads_never_reach_the_logs_or_the_metrics`.

## 10 · Prove the whole thing still works

```bash
docker build -t kyu:smoke .
bash scripts/container-smoke.sh kyu:smoke
```

Walks the three verbs through the real image, restarts the container, runs the
old volume against a freshly built image, then starts a second protected
container and checks that a tokenless publish is refused, a good token works,
monitoring stays open, the login page's assets are inside the image, and a
half-configured door refuses to boot.

Takes a few minutes and needs Docker. This is the check to run before you
believe anything about a deployment.
