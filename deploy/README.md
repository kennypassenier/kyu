# deploy/ — what actually runs on the machine

These files are the source of truth for a native systemd deployment of
kyu. They existed only on LXC 109 until 2026-09-02, in no repository at all,
which is how a broken one went unnoticed for two nights: nothing versioned
them, so no test, no gate and no review could ever look at them (F179).

| File | What it is |
|---|---|
| `kyu.service` | the hub itself (`Type=notify`, `ExecStartPre=/opt/kyu/bin/kyu --check`, hardened, `RuntimeDirectory=kyu`) |
| `kyu-backup` | script: asks the running hub for a consistent copy (`POST /api/backup`) and prunes old ones |
| `kyu-backup.service` | one-shot unit that runs `kyu-backup`; `OnFailure=` starts `kyu-alert@%n` |
| `kyu-backup.timer` | runs that service nightly at 03:00 (`Persistent=true`) |
| `kyu-alert` | script: publishes a message to a topic (default `ops.alerts`) when a helper unit fails |
| `kyu-alert@.service` | template unit that runs `kyu-alert %i` |
| `service.yml` | the homelab's stack file for the native service (vmid 109, binary `/opt/kyu/bin/kyu`, `update_cmd`) |
| `journald.conf` | journal budget for the LXC (`SystemMaxUse=200M`, one month); install as `/etc/systemd/journald.conf.d/50-service.conf` |
| `compose.example.yml` | running the container image with docker compose instead (named volume, log rotation) |

## The one thing that is deployment-specific

`EnvironmentFile=` in the three `.service` files (`kyu.service`,
`kyu-backup.service`, `kyu-alert@.service`). That path is owned by whoever
deploys kyu — on LXC 109 the homelab's adoption put it at
`/appdata/kyu/kyu-config/kyu.env`, which is what the files in this directory
say; elsewhere it may be `/etc/kyu/kyu.env`.

**Nothing else may name a path.** The scripts read `KYU_STATE_DIR`,
`KYU_LISTEN` and `KYU_TOKEN` (`kyu-alert` needs only the last two) from the
environment systemd hands them, and refuse loudly if any is missing rather
than falling back on a guess. That is standing rule 28 — state has an address
and Kenny owns it — applied to the helpers as well as to the hub, which is
the lesson F179 actually taught: `kyu-backup` hardcoded `/etc/kyu/kyu.env`
and `/var/lib/kyu`, the deployment moved both, and every nightly run failed
with `grep: /etc/kyu/kyu.env: No such file or directory` while the timer went
on reporting that it had fired.

## Optional environment variables the scripts read

| Variable | Default | Read by | What it does |
|---|---|---|---|
| `KYU_BACKUP_KEEP` | `7` | `kyu-backup` | How many `kyu.backup-*.db` copies to keep in `KYU_STATE_DIR`; older ones are deleted after a successful backup. A count rather than an age, so a week of failed runs cannot delete the last good backup. |
| `KYU_ALERT_TOPIC` | `ops.alerts` | `kyu-alert` | The topic the failure message is published to. |

## The backup-pause socket

`kyu.service` sets `RuntimeDirectory=kyu` and `RuntimeDirectoryMode=0700`.
That creates `/run/kyu/`, where the running service listens on
`/run/kyu/backup.sock`. The kit's `kyu backup-pause --for <secs>` and
`kyu backup-resume` talk to it: pause holds the service's writes until the
files stand still, so a plain `tar` of the state directory is a restorable
copy (`--for` is 1 to 21600 seconds, and the pause ends by itself after that
even if `backup-resume` never comes). The socket sits outside the state
root, so it is never archived. `kyu-backup` does not use it: it asks the hub
for `VACUUM INTO` over `/api/backup` instead.

## Installing

```bash
install -m 755 deploy/kyu-backup deploy/kyu-alert /usr/local/bin/
install -m 644 deploy/*.service deploy/*.timer /etc/systemd/system/
# set EnvironmentFile= to wherever this host keeps kyu.env, then:
systemctl daemon-reload
systemctl enable --now kyu kyu-backup.timer
```

## Checking that it still works

The backup is proven by `tests/f179_backup.rs`, which runs this exact script
against a real hub. On the machine, the honest check is the service rather
than the timer:

```bash
systemctl status kyu-backup.service     # the timer firing says nothing
ls -lt <KYU_STATE_DIR>/kyu.backup-*.db | head -1
```
