# deploy/ — what actually runs on the machine

These four files are the source of truth for a native systemd deployment of
kyu. They existed only on LXC 109 until 2026-09-02, in no repository at all,
which is how a broken one went unnoticed for two nights: nothing versioned
them, so no test, no gate and no review could ever look at them (F179).

| File | What it is |
|---|---|
| `kyu.service` | the hub itself |
| `kyu-backup` | asks the running hub for a consistent copy and prunes old ones |
| `kyu-backup.service` + `.timer` | runs that nightly at 03:00 |
| `kyu-alert` + `kyu-alert@.service` | publishes a message when a helper unit fails |

## The one thing that is deployment-specific

`EnvironmentFile=` in the two `.service` files. That path is owned by whoever
deploys kyu — on LXC 109 the homelab's adoption put it at
`/appdata/kyu/kyu-config/kyu.env`; elsewhere it may be `/etc/kyu/kyu.env`.

**Nothing else may name a path.** The scripts read `KYU_STATE_DIR`,
`KYU_LISTEN` and `KYU_TOKEN` from the environment systemd hands them, and
refuse loudly if any is missing rather than falling back on a guess. That is
standing rule 28 — state has an address and Kenny owns it — applied to the
helpers as well as to the hub, which is the lesson F179 actually taught:
`kyu-backup` hardcoded `/etc/kyu/kyu.env` and `/var/lib/kyu`, the deployment
moved both, and every nightly run failed with
`grep: /etc/kyu/kyu.env: No such file or directory` while the timer went on
reporting that it had fired.

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
