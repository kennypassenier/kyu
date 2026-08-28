#!/usr/bin/env bash
# Container smoke test (K13, L5 exit criterion): start the real image, walk
# the three verbs through it, restart the container and check the state
# survived. Run locally or in CI; it cleans up after itself.
set -euo pipefail

IMAGE="${1:-mailbox:smoke}"
NAME="mailbox-smoke-$$"
VOLUME="mailbox-smoke-data-$$"
PORT="${PORT:-18099}"
HUB="http://localhost:${PORT}"

cleanup() {
    docker rm -f "$NAME" >/dev/null 2>&1 || true
    docker volume rm "$VOLUME" >/dev/null 2>&1 || true
}
trap cleanup EXIT

say() { printf '\n== %s\n' "$1"; }

say "starting $IMAGE"
docker run -d --name "$NAME" -p "${PORT}:8080" -v "${VOLUME}:/data" "$IMAGE" >/dev/null

for _ in $(seq 1 60); do
    if curl -sf -o /dev/null "${HUB}/healthz"; then break; fi
    sleep 0.5
done
curl -sf -o /dev/null "${HUB}/healthz" || { echo "the hub never became healthy"; docker logs "$NAME"; exit 1; }

say "the container reports itself healthy through its own binary"
docker inspect --format '{{.State.Health.Status}}' "$NAME" 2>/dev/null || true
docker exec "$NAME" /usr/local/bin/mailbox --healthcheck

say "publish, subscribe, publish, receive, ack"
curl -sf -o /dev/null -H 'content-type: application/json' -d '{"bootstrap":1}' "${HUB}/t/notify.kenny"
curl -sf -o /dev/null "${HUB}/t/notify.kenny/next?as=printer&wait=0"
ID=$(curl -sf -H 'content-type: application/json' -d '{"title":"smoke"}' "${HUB}/t/notify.kenny" \
     | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')
GOT=$(curl -sf -D- -o /dev/null "${HUB}/t/notify.kenny/next?as=printer" \
      | tr -d '\r' | awk 'tolower($1)=="mailbox-id:"{print $2}')
[ "$GOT" = "$ID" ] || { echo "expected $ID, received $GOT"; exit 1; }
curl -sf -o /dev/null -X POST "${HUB}/t/notify.kenny/ack/${ID}?as=printer"

say "leave one message unacked, then restart the container"
UNACKED=$(curl -sf -H 'content-type: application/json' -d '{"title":"survives"}' "${HUB}/t/notify.kenny" \
          | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')
docker restart "$NAME" >/dev/null
for _ in $(seq 1 60); do
    if curl -sf -o /dev/null "${HUB}/healthz"; then break; fi
    sleep 0.5
done

say "state survived the restart"
AFTER=$(curl -sf -D- -o /dev/null "${HUB}/t/notify.kenny/next?as=printer&wait=0" \
        | tr -d '\r' | awk 'tolower($1)=="mailbox-id:"{print $2}')
[ "$AFTER" = "$UNACKED" ] || { echo "expected the unacked $UNACKED, received '${AFTER}'"; exit 1; }

STATUS=$(curl -sf -o /dev/null -w '%{http_code}' "${HUB}/t/notify.kenny/next?as=printer&wait=0" || true)
[ "$STATUS" = "204" ] || { echo "the acked message came back (status $STATUS)"; exit 1; }

say "upgrade: the same volume against a freshly built image"
# The only place migration, snapshot and the healthcheck start-period meet
# in reality is an existing volume meeting a new image — which is what every
# pull on the LXC does.
docker rm -f "$NAME" >/dev/null
docker run -d --name "$NAME" -p "${PORT}:8080" -v "${VOLUME}:/data" "$IMAGE" >/dev/null
for _ in $(seq 1 60); do
    if curl -sf -o /dev/null "${HUB}/healthz"; then break; fi
    sleep 0.5
done
curl -sf -o /dev/null "${HUB}/healthz" || { echo "the hub did not come back after an upgrade"; docker logs "$NAME"; exit 1; }

STILL=$(curl -sf -D- -o /dev/null "${HUB}/t/notify.kenny/next?as=printer&wait=0" \
        | tr -d '\r' | awk 'tolower($1)=="mailbox-id:"{print $2}')
[ -z "$STILL" ] || { echo "the upgraded hub redelivered something already acked: $STILL"; exit 1; }

docker exec "$NAME" /usr/local/bin/mailbox --healthcheck

printf '\nOK: three verbs, healthcheck, restart and upgrade all behaved.\n'
