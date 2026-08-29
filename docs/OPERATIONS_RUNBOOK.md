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

**As a plain binary under systemd** — what Kenny actually runs, on LXC 109
(`109-app-mailbox`, `10.10.10.9`). No Docker on the box at all:

1. Take the binary out of the published image on a machine that has Docker,
   and copy it over. It is statically linked, so it needs nothing installed:
   ```bash
   id=$(docker create ghcr.io/kennypassenier/mailbox:1.0.1)
   docker cp "$id":/usr/local/bin/mailbox ./mailbox && docker rm "$id"
   scp mailbox root@<proxmox>:/tmp/mailbox
   ssh root@<proxmox> 'pct push 109 /tmp/mailbox /usr/local/bin/mailbox --perms 755'
   ```
2. A system user that owns only its data, and a config file only it can read:
   ```bash
   adduser --system --group --home /var/lib/mailbox --no-create-home mailbox
   install -d -o mailbox -g mailbox -m 0750 /var/lib/mailbox
   install -d -o root -g mailbox -m 0750 /etc/mailbox
   umask 077
   printf 'MAILBOX_TOKEN=%s\nMAILBOX_SECRET_KEY=%s\nMAILBOX_LISTEN=0.0.0.0:8080\nMAILBOX_DATA_DIR=/var/lib/mailbox\nMAILBOX_LOG=info\n' \
     "$(openssl rand -hex 24)" "$(openssl rand -hex 32)" > /etc/mailbox/mailbox.env
   chown root:mailbox /etc/mailbox/mailbox.env && chmod 0640 /etc/mailbox/mailbox.env
   ```
3. `mailbox.service` with `Restart=always` and `StartLimitIntervalSec=0` — the
   second matters as much as the first, because systemd otherwise gives up
   after a few restarts in a short window and turns a transient fault into a
   permanent outage. The unit also carries the namespace hardening
   (`ProtectSystem=strict` and friends), which in an **unprivileged LXC needs
   `features: nesting=1` on the container** or every start fails with
   `Failed to set up mount namespacing`. That is measured, not guessed.
4. `systemctl enable --now mailbox`, then `curl -sf http://<host>:8080/healthz`.

**Standalone with Docker:**

```bash
docker compose up -d
curl -sf localhost:8080/healthz
```

Data lands in the `mailbox-data` named volume. Do not "fix" that into a bind
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
   `ghcr.io/kennypassenier/mailbox:1.2.3` and `:latest`.
6. **Write the GitHub Release yourself:**
   ```bash
   gh release create v1.2.3 --title "v1.2.3" --notes-file <(sed -n '/## \[1.2.3\]/,/## \[/p' CHANGELOG.md)
   ```
   The workflow deliberately does not do this. Release notes are the one part
   a human adds something to, and automating them from commit subjects
   produces a list nobody reads.
7. **Verify the image exists and is pullable:**
   ```bash
   docker pull ghcr.io/kennypassenier/mailbox:1.2.3
   ```
   The package is linked to this repository and takes its visibility, so a
   public repo yields a package the homelab host can pull anonymously.

**Deviation from the procedure, recorded rather than forgotten:** Phase 9 of
the dev procedure asks for tag → build → *checksum manifest* → GitHub Release.
mailbox ships no self-updating binary — updates arrive as a new image, whose
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
     pct exec 109 -- systemctl stop mailbox
     pct push 109 /tmp/mailbox /usr/local/bin/mailbox --perms 755
     pct exec 109 -- systemctl start mailbox
     pct exec 109 -- /usr/local/bin/mailbox --version'
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

**On LXC 109** there are two things to restore from, and which one you want
depends on what broke:

- **The container is gone or unbootable** → the Proxmox backup job
  `mailbox-109` (nightly 03:30, 7 daily + 4 weekly, on `local`). Restore the
  container; everything comes back with it.
- **The data is wrong but the container is fine** — a bad migration, a
  message you should not have deleted → the newest
  `/var/lib/mailbox/mailbox.backup-*.db`, written nightly at 03:00 by
  `mailbox-backup.timer` and integrity-checked by the hub before it was
  reported. Follow step 2 above: stop the service, put it in place of
  `mailbox.db`, delete the `-wal` and `-shm`, start again.

The 03:00 backup runs half an hour before the snapshot on purpose: whatever
state the live database is caught in, the snapshot always contains one file
the hub itself opened and checked.

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
