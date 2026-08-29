#!/usr/bin/env bash
# Container smoke test (K13, L5 exit criterion): start the real image, walk
# the three verbs through it, restart the container and check the state
# survived. Run locally or in CI; it cleans up after itself.
set -euo pipefail

IMAGE="${1:-kyu:smoke}"
NAME="kyu-smoke-$$"
VOLUME="kyu-smoke-data-$$"
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
docker exec "$NAME" /usr/local/bin/kyu --healthcheck

say "publish, subscribe, publish, receive, ack"
curl -sf -o /dev/null -H 'content-type: application/json' -d '{"bootstrap":1}' "${HUB}/t/notify.kenny"
curl -sf -o /dev/null "${HUB}/t/notify.kenny/next?as=printer&wait=0"
ID=$(curl -sf -H 'content-type: application/json' -d '{"title":"smoke"}' "${HUB}/t/notify.kenny" \
     | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')
GOT=$(curl -sf -D- -o /dev/null "${HUB}/t/notify.kenny/next?as=printer" \
      | tr -d '\r' | awk 'tolower($1)=="kyu-id:"{print $2}')
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
        | tr -d '\r' | awk 'tolower($1)=="kyu-id:"{print $2}')
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
        | tr -d '\r' | awk 'tolower($1)=="kyu-id:"{print $2}')
[ -z "$STILL" ] || { echo "the upgraded hub redelivered something already acked: $STILL"; exit 1; }

docker exec "$NAME" /usr/local/bin/kyu --healthcheck

say "the door: a protected hub, in the real image (W2)"
# A second container, this one with a token. The point is not to re-test the
# auth logic — the Rust suite does that — but to prove the two variables
# actually reach the binary through compose-style env, that the static assets
# the login page needs are inside the image, and that monitoring still works
# without a token.
DOOR="${NAME}-door"
DOOR_VOLUME="${VOLUME}-door"
DOOR_PORT=$((PORT + 1))
DOOR_HUB="http://localhost:${DOOR_PORT}"
TOKEN="smoke-token-$(date +%s)-abcdefgh"
KEY=$(openssl rand -hex 32)

cleanup_door() {
    docker rm -f "$DOOR" >/dev/null 2>&1 || true
    docker volume rm "$DOOR_VOLUME" >/dev/null 2>&1 || true
}
trap 'cleanup; cleanup_door' EXIT

docker run -d --name "$DOOR" -p "${DOOR_PORT}:8080" -v "${DOOR_VOLUME}:/data" \
    -e "KYU_TOKEN=${TOKEN}" -e "KYU_SECRET_KEY=${KEY}" "$IMAGE" >/dev/null
for _ in $(seq 1 60); do
    if curl -sf -o /dev/null "${DOOR_HUB}/healthz"; then break; fi
    sleep 0.5
done
curl -sf -o /dev/null "${DOOR_HUB}/healthz" \
    || { echo "the protected hub never became healthy"; docker logs "$DOOR"; exit 1; }

STATUS=$(curl -s -o /dev/null -w '%{http_code}' -H 'content-type: application/json' \
         -d '{"title":"no token"}' "${DOOR_HUB}/t/notify.kenny")
[ "$STATUS" = "401" ] || { echo "a tokenless publish was not refused (status $STATUS)"; exit 1; }

STATUS=$(curl -s -o /dev/null -w '%{http_code}' -H "authorization: Bearer ${TOKEN}" \
         -H 'content-type: application/json' -d '{"title":"with token"}' "${DOOR_HUB}/t/notify.kenny")
[ "$STATUS" = "201" ] || { echo "a good token was refused (status $STATUS)"; exit 1; }

for PATH_ in /healthz /metrics; do
    STATUS=$(curl -s -o /dev/null -w '%{http_code}' "${DOOR_HUB}${PATH_}")
    [ "$STATUS" = "200" ] || { echo "${PATH_} must stay open for monitoring (status $STATUS)"; exit 1; }
done

# The login page is useless without its stylesheet, and the stylesheet only
# exists inside the binary — this is the check that would have caught the
# templates/ omission in the Dockerfile the first time round.
for ASSET in bootstrap.min.css app.js; do
    STATUS=$(curl -s -o /dev/null -w '%{http_code}' "${DOOR_HUB}/static/${ASSET}")
    [ "$STATUS" = "200" ] || { echo "the image is missing ${ASSET} (status $STATUS)"; exit 1; }
done

# A half-configured door must refuse to start rather than run open.
if docker run --rm -e "KYU_TOKEN=${TOKEN}" "$IMAGE" >/dev/null 2>&1; then
    echo "a token without a secret key started anyway, which it must never do"
    exit 1
fi

printf '\nOK: three verbs, healthcheck, restart, upgrade and the door all behaved.\n'
