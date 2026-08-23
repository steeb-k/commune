#!/usr/bin/env bash
#
# Stand up a throwaway Matrix homeserver and seed it with the rooms needed to
# exercise the parts of Commune that a normal account cannot reach.
#
# There are two of those, and both are why this script exists:
#
#   * The join rule subpage only offers the room membership rule to a room that
#     is already restricted to a space. Making one by hand means an account that
#     can create a space and a room inside it.
#
#   * Reporting a room, a user or an event sends a real report to a real
#     administrator. Against a public homeserver that is noise someone has to
#     read. Against this one it goes to a server you throw away afterwards.
#
# Everything lives under testing/.homeserver, which is git-ignored. Run
# `./testing/local-homeserver.sh down` to stop the server, or `clean` to also
# delete its data.
#
# Usage:
#   ./testing/local-homeserver.sh up      # start and seed (default)
#   ./testing/local-homeserver.sh check   # verify the endpoints server-side
#   ./testing/local-homeserver.sh reports # show what the admin has received
#   ./testing/local-homeserver.sh down    # stop the server, keep the data
#   ./testing/local-homeserver.sh clean   # stop and delete everything

set -euo pipefail

CONTAINER=commune-test-homeserver
IMAGE=${COMMUNE_TEST_SYNAPSE_IMAGE:-ghcr.io/element-hq/synapse:latest}
PORT=${COMMUNE_TEST_PORT:-8008}
HS=http://localhost:$PORT

# Rootless podman maps the container's user to a subuid, so anything the image
# writes lands in the data directory owned by someone we are not. Run as
# ourselves instead, so the config stays editable and `clean` can delete it.
AS_US=(--userns=keep-id --user "$(id -u):$(id -g)")

HERE=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
DATA=$HERE/.homeserver
STATE=$DATA/seeded.json

ALICE_PASS=alice-is-testing
BOB_PASS=bob-is-testing

log() { printf '\033[1;36m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m!!\033[0m %s\n' "$*" >&2; }
die() { printf '\033[1;31m!!\033[0m %s\n' "$*" >&2; exit 1; }

need() { command -v "$1" >/dev/null || die "$1 is not installed"; }

# ---------------------------------------------------------------- server ----

start_server() {
  if podman container exists "$CONTAINER" 2>/dev/null; then
    if [ "$(podman inspect -f '{{.State.Running}}' "$CONTAINER")" = true ]; then
      log "Homeserver is already running on $HS"
      return
    fi
    log "Starting the existing homeserver container…"
    podman start "$CONTAINER" >/dev/null
    wait_for_server
    return
  fi

  mkdir -p "$DATA"

  if [ ! -f "$DATA/homeserver.yaml" ]; then
    log "Generating the homeserver configuration…"
    podman run --rm "${AS_US[@]}" \
      -v "$DATA:/data:Z" \
      -e SYNAPSE_SERVER_NAME=localhost \
      -e SYNAPSE_REPORT_STATS=no \
      "$IMAGE" generate >/dev/null

    # Open registration: this server is reachable from nowhere and is deleted
    # when the tests are over.
    cat >> "$DATA/homeserver.yaml" <<'YAML'

# Added by testing/local-homeserver.sh.
enable_registration: true
enable_registration_without_verification: true
rc_message:
  per_second: 1000
  burst_count: 1000
rc_registration:
  per_second: 1000
  burst_count: 1000
rc_login:
  address:
    per_second: 1000
    burst_count: 1000
  account:
    per_second: 1000
    burst_count: 1000
rc_reports:
  per_second: 1000
  burst_count: 1000
YAML
  fi

  log "Starting the homeserver on $HS…"
  podman run -d --name "$CONTAINER" "${AS_US[@]}" \
    -v "$DATA:/data:Z" \
    -p "$PORT:8008" \
    "$IMAGE" >/dev/null

  wait_for_server
}

wait_for_server() {
  log "Waiting for the homeserver to answer…"
  for _ in $(seq 1 60); do
    if curl -sf "$HS/_matrix/client/versions" >/dev/null 2>&1; then
      log "Homeserver is up."
      return
    fi
    sleep 1
  done
  podman logs --tail 40 "$CONTAINER" >&2 || true
  die "The homeserver did not come up in 60 seconds"
}

# ------------------------------------------------------------------ users ----

register() {
  local user=$1 pass=$2 admin=$3
  local admin_flag=--no-admin
  [ "$admin" = admin ] && admin_flag=--admin

  if podman exec "$CONTAINER" register_new_matrix_user \
      -c /data/homeserver.yaml -u "$user" -p "$pass" $admin_flag \
      http://localhost:8008 >/dev/null 2>&1; then
    log "Registered @$user:localhost"
  else
    log "@$user:localhost already exists."
  fi
}

login() {
  local user=$1 pass=$2
  curl -sf -X POST "$HS/_matrix/client/v3/login" \
    -H 'Content-Type: application/json' \
    -d "{\"type\":\"m.login.password\",\"identifier\":{\"type\":\"m.id.user\",\"user\":\"$user\"},\"password\":\"$pass\"}" \
    | jq -r .access_token
}

# ------------------------------------------------------------------ rooms ----

create_room() {
  local token=$1 body=$2
  curl -sf -X POST "$HS/_matrix/client/v3/createRoom" \
    -H "Authorization: Bearer $token" \
    -H 'Content-Type: application/json' \
    -d "$body" | jq -r .room_id
}

seed() {
  register admin admin-is-testing admin
  register alice "$ALICE_PASS" plain
  register bob "$BOB_PASS" plain

  local alice bob
  alice=$(login alice "$ALICE_PASS")
  bob=$(login bob "$BOB_PASS")
  [ -n "$alice" ] || die "Could not log in as alice"

  if [ -f "$STATE" ]; then
    log "Rooms were already seeded; reusing them."
    return
  fi

  log "Creating the space…"
  local space
  space=$(create_room "$alice" '{
    "name": "Test Space",
    "creation_content": {"type": "m.space"},
    "preset": "public_chat",
    "room_alias_name": "test-space"
  }')

  # The room the join rule subpage is really about: restricted to the space, so
  # the "Members of Test Space" row appears and the knock switch has something
  # to attach to.
  log "Creating the restricted room…"
  local restricted
  restricted=$(create_room "$alice" "$(jq -nc --arg space "$space" '{
    name: "Restricted Room",
    preset: "private_chat",
    initial_state: [{
      type: "m.room.join_rules",
      state_key: "",
      content: {
        join_rule: "restricted",
        allow: [{type: "m.room_membership", room_id: $space}]
      }
    }]
  }')")

  # The same room with knocking already on, so both sides of the switch can be
  # seen without changing anything first.
  log "Creating the knock restricted room…"
  local knock_restricted
  knock_restricted=$(create_room "$alice" "$(jq -nc --arg space "$space" '{
    name: "Knock Restricted Room",
    preset: "private_chat",
    room_version: "10",
    initial_state: [{
      type: "m.room.join_rules",
      state_key: "",
      content: {
        join_rule: "knock_restricted",
        allow: [{type: "m.room_membership", room_id: $space}]
      }
    }]
  }')")

  log "Creating the plain rooms…"
  local invite_room knock_room public_room
  invite_room=$(create_room "$alice" '{"name": "Invite Room", "preset": "private_chat"}')
  knock_room=$(create_room "$alice" '{
    "name": "Knock Room",
    "preset": "private_chat",
    "initial_state": [{
      "type": "m.room.join_rules",
      "state_key": "",
      "content": {"join_rule": "knock"}
    }]
  }')
  public_room=$(create_room "$alice" '{
    "name": "Public Room",
    "preset": "public_chat",
    "room_alias_name": "public-room"
  }')

  # Something to report, and someone to report it to.
  log "Inviting bob and adding him to the space…"
  for room in "$space" "$invite_room" "$restricted" "$knock_restricted"; do
    curl -sf -X POST "$HS/_matrix/client/v3/rooms/$room/invite" \
      -H "Authorization: Bearer $alice" -H 'Content-Type: application/json' \
      -d '{"user_id": "@bob:localhost"}' >/dev/null
    curl -sf -X POST "$HS/_matrix/client/v3/rooms/$room/join" \
      -H "Authorization: Bearer $bob" -H 'Content-Type: application/json' \
      -d '{}' >/dev/null
  done

  curl -sf -X PUT "$HS/_matrix/client/v3/rooms/$invite_room/send/m.room.message/seed1" \
    -H "Authorization: Bearer $bob" -H 'Content-Type: application/json' \
    -d '{"msgtype": "m.text", "body": "Something worth reporting."}' >/dev/null

  jq -nc \
    --arg space "$space" \
    --arg restricted "$restricted" \
    --arg knock_restricted "$knock_restricted" \
    --arg invite_room "$invite_room" \
    --arg knock_room "$knock_room" \
    --arg public_room "$public_room" \
    '$ARGS.named' > "$STATE"
}

# ------------------------------------------------------------------ check ----

# Confirm the server accepts what Commune sends, so that a failure in the app is
# known to be the app's and not the homeserver's.
check() {
  local alice bob room
  alice=$(login alice "$ALICE_PASS")
  bob=$(login bob "$BOB_PASS")
  room=$(jq -r .invite_room "$STATE")

  local failed=0

  log "Checking the room report endpoint…"
  if curl -sf -X POST "$HS/_matrix/client/v3/rooms/$room/report" \
      -H "Authorization: Bearer $bob" -H 'Content-Type: application/json' \
      -d '{"reason": "harness check: room"}' >/dev/null; then
    printf '    POST /_matrix/client/v3/rooms/{roomId}/report  OK\n'
  else
    warn "POST /_matrix/client/v3/rooms/{roomId}/report was rejected"
    failed=1
  fi

  log "Checking the user report endpoint…"
  if curl -sf -X POST "$HS/_matrix/client/v3/users/@alice:localhost/report" \
      -H "Authorization: Bearer $bob" -H 'Content-Type: application/json' \
      -d '{"reason": "harness check: user"}' >/dev/null; then
    printf '    POST /_matrix/client/v3/users/{userId}/report  OK\n'
  else
    warn "POST /_matrix/client/v3/users/{userId}/report was rejected — this homeserver is too old for MSC4260"
    failed=1
  fi

  log "Checking that the restricted rule round trips…"
  local restricted space rule
  restricted=$(jq -r .restricted "$STATE")
  space=$(jq -r .space "$STATE")
  rule=$(curl -sf "$HS/_matrix/client/v3/rooms/$restricted/state/m.room.join_rules/" \
    -H "Authorization: Bearer $alice")

  # This is the change the join rule subpage makes: turn knocking on while
  # keeping the allow list, then turn it back off.
  curl -sf -X PUT "$HS/_matrix/client/v3/rooms/$restricted/state/m.room.join_rules/" \
    -H "Authorization: Bearer $alice" -H 'Content-Type: application/json' \
    -d "$(jq -nc --arg space "$space" '{
      join_rule: "knock_restricted",
      allow: [{type: "m.room_membership", room_id: $space}]
    }')" >/dev/null || { warn "the server refused knock_restricted"; failed=1; }

  curl -sf -X PUT "$HS/_matrix/client/v3/rooms/$restricted/state/m.room.join_rules/" \
    -H "Authorization: Bearer $alice" -H 'Content-Type: application/json' \
    -d "$rule" >/dev/null

  local after
  after=$(curl -sf "$HS/_matrix/client/v3/rooms/$restricted/state/m.room.join_rules/" \
    -H "Authorization: Bearer $alice")
  if [ "$(jq -cS . <<<"$after")" = "$(jq -cS . <<<"$rule")" ]; then
    printf '    m.room.join_rules round trip                   OK\n'
  else
    warn "the allow list did not survive the round trip"
    failed=1
  fi

  [ "$failed" = 0 ] || die "Some checks failed; see above."
  log "All server-side checks passed."
}

# ---------------------------------------------------------------- reports ----

# Synapse exposes an admin API for event reports only, so read all three
# straight out of its database. It is SQLite here, and opened read-only, so
# there is no need to stop the server first.
reports() {
  [ -f "$DATA/homeserver.db" ] || die "No homeserver database; run './testing/local-homeserver.sh up' first"

  python3 - "$DATA/homeserver.db" <<'PY'
import sqlite3, sys
from datetime import datetime, timezone

db = sqlite3.connect(f"file:{sys.argv[1]}?mode=ro", uri=True)

def when(ms):
    return datetime.fromtimestamp(ms / 1000, timezone.utc).strftime("%H:%M:%S")

def show(title, query, line):
    print(f"\n  {title}")
    rows = list(db.execute(query))
    if not rows:
        print("    (none)")
        return
    for row in rows:
        print("    " + line(row))

show(
    "Room reports",
    "select received_ts, user_id, room_id, reason from room_reports order by id",
    lambda r: f"{when(r[0])}  {r[1]} reported {r[2]}: {r[3] or '(no reason)'}",
)
show(
    "User reports",
    "select received_ts, user_id, target_user_id, reason from user_reports order by id",
    lambda r: f"{when(r[0])}  {r[1]} reported {r[2]}: {r[3] or '(no reason)'}",
)
show(
    "Event reports",
    "select received_ts, user_id, room_id, event_id, reason from event_reports order by id",
    lambda r: f"{when(r[0])}  {r[1]} reported {r[3]} in {r[2]}: {r[4] or '(no reason)'}",
)
print()
PY
}

# ------------------------------------------------------------------ shell ----

summary() {
  cat <<EOF

  ────────────────────────────────────────────────────────────────────
  Homeserver   $HS
  Log in as    alice / $ALICE_PASS
               bob   / $BOB_PASS
               admin / admin-is-testing

  What to look at in Commune:

  Join rules — open Room Details ▸ Who Can Join on
    "Restricted Room"        the "Members of Test Space" row is shown and
                             selected, and "Allow Invite Requests" now saves
    "Knock Restricted Room"  the same, with the switch already on
    "Invite Room"            unchanged behaviour, for comparison

  Reporting — the report goes to the admin account above, not to a stranger
    Room menu ▸ Report Room…          on any room
    Sidebar right-click ▸ Report Room… including on an invite
    A user's profile ▸ Report…        on @alice or @bob
    A message's menu ▸ Report          unchanged, for comparison

  Then run  ./testing/local-homeserver.sh reports  to see what arrived.
  ────────────────────────────────────────────────────────────────────

EOF
}

case "${1:-up}" in
  up)
    need podman; need curl; need jq
    start_server
    seed
    summary
    ;;
  check)
    need podman; need curl; need jq
    [ -f "$STATE" ] || die "Nothing seeded yet; run './testing/local-homeserver.sh up' first"
    check
    ;;
  reports)
    need python3
    reports
    ;;
  down)
    podman stop "$CONTAINER" >/dev/null 2>&1 && log "Homeserver stopped." || log "Not running."
    ;;
  clean)
    podman rm -f "$CONTAINER" >/dev/null 2>&1 || true
    # The data is written by the container as root, so it may need help.
    rm -rf "$DATA" 2>/dev/null || podman unshare rm -rf "$DATA"
    log "Homeserver and its data are gone."
    ;;
  *)
    die "Unknown command '$1'. Try: up, check, reports, down, clean"
    ;;
esac
