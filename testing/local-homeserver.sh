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
#   * A server ACL decides which homeservers may take part in a room, and
#     getting one wrong shuts people out of a room for good. It is not a thing
#     to try out on a room anybody is using.
#
#   * Server notices come from the homeserver itself. Getting one on a real
#     account means waiting for that server to have something to say, and
#     Synapse ships them switched off. Here they are turned on and one is sent
#     and pinned on demand.
#
#   * Calls need a TURN server, and a homeserver configured to hand out
#     credentials for it. `up` runs a coturn beside the Synapse and points the
#     two at each other, so `GET /_matrix/client/v3/voip/turnServer` answers
#     with something a client can actually use.
#
# Everything lives under testing/.homeserver, which is git-ignored. Run
# `./testing/local-homeserver.sh down` to stop the server, or `clean` to also
# delete its data.
#
# Usage:
#   ./testing/local-homeserver.sh up      # start and seed (default)
#   ./testing/local-homeserver.sh turn    # show the TURN credentials a client gets
#   ./testing/local-homeserver.sh notice  # send another server notice to alice
#   ./testing/local-homeserver.sh limit on|off  # cross the MAU limit, so Synapse
#                                         # sends and pins a notice of its own
#   ./testing/local-homeserver.sh check   # verify the endpoints server-side
#   ./testing/local-homeserver.sh reports # show what the admin has received
#   ./testing/local-homeserver.sh down    # stop the server, keep the data
#   ./testing/local-homeserver.sh clean   # stop and delete everything

set -euo pipefail

CONTAINER=commune-test-homeserver
IMAGE=${COMMUNE_TEST_SYNAPSE_IMAGE:-ghcr.io/element-hq/synapse:latest}
PORT=${COMMUNE_TEST_PORT:-8008}
HS=http://localhost:$PORT

TURN_CONTAINER=commune-test-turn
TURN_IMAGE=${COMMUNE_TEST_COTURN_IMAGE:-docker.io/coturn/coturn:latest}
TURN_PORT=${COMMUNE_TEST_TURN_PORT:-3478}
# coturn's default relay range is the whole ephemeral range, and publishing
# sixteen thousand UDP ports is not a thing to do to a machine. Two calls need
# four of these; forty is room to spare.
TURN_RELAY_MIN=${COMMUNE_TEST_TURN_RELAY_MIN:-49160}
TURN_RELAY_MAX=${COMMUNE_TEST_TURN_RELAY_MAX:-49200}
# Shared with Synapse, which derives a per-user credential from it. Fixed rather
# than generated so that restarting one container does not invalidate the other.
TURN_SECRET=commune-test-turn-secret

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

# Synapse ships with URL previews off, and most servers leave them that way, so
# a real account is no use for testing them. The blacklist is not optional:
# Synapse refuses to start with previews on and no range excluded, because
# without it the endpoint will happily fetch anything on the host's network.
# Where the homeserver sends clients for TURN. Synapse does not run one; it
# hands out a credential derived from the shared secret, which the coturn beside
# it verifies without ever being told about the user.
turn_uris:
  - "turn:127.0.0.1:TURN_PORT_PLACEHOLDER?transport=udp"
  - "turn:127.0.0.1:TURN_PORT_PLACEHOLDER?transport=tcp"
turn_shared_secret: "TURN_SECRET_PLACEHOLDER"
turn_user_lifetime: 86400000
turn_allow_guests: false

# Server notices are off by default, and there is no way for a client to turn
# them on. The localpart below becomes @notices:localhost, which is the user the
# homeserver speaks as.
server_notices:
  system_mxid_localpart: notices
  system_mxid_display_name: "Server Notices"
  room_name: "Server Notices"
  room_topic: "Messages from the administrator of this homeserver"
  auto_join: false

# Without this, the notice Synapse writes itself carries `admin_contact: null`
# and the banner has nothing to offer a button for.
admin_contact: 'mailto:admin@localhost'

url_preview_enabled: true
url_preview_ip_range_blacklist:
  - '127.0.0.0/8'
  - '10.0.0.0/8'
  - '172.16.0.0/12'
  - '192.168.0.0/16'
  - '100.64.0.0/10'
  - '192.0.0.0/24'
  - '169.254.0.0/16'
  - '192.88.99.0/24'
  - '198.18.0.0/15'
  - '192.0.2.0/24'
  - '198.51.100.0/24'
  - '203.0.113.0/24'
  - '224.0.0.0/4'
  - '::1/128'
  - 'fe80::/10'
  - 'fc00::/7'
  - '2001:db8::/32'
  - 'ff00::/8'
  - 'fec0::/10'
YAML
    sed -i \
      -e "s/TURN_PORT_PLACEHOLDER/$TURN_PORT/g" \
      -e "s/TURN_SECRET_PLACEHOLDER/$TURN_SECRET/g" \
      "$DATA/homeserver.yaml"
  fi

  log "Starting the homeserver on $HS…"
  podman run -d --name "$CONTAINER" "${AS_US[@]}" \
    -v "$DATA:/data:Z" \
    -p "$PORT:8008" \
    "$IMAGE" >/dev/null

  wait_for_server
}

start_turn_server() {
  if podman container exists "$TURN_CONTAINER" 2>/dev/null; then
    if [ "$(podman inspect -f '{{.State.Running}}' "$TURN_CONTAINER")" = true ]; then
      log "TURN server is already running on port $TURN_PORT."
      return
    fi
    log "Starting the existing TURN container…"
    podman start "$TURN_CONTAINER" >/dev/null
    return
  fi

  log "Starting coturn on port $TURN_PORT…"
  # Host networking, not published ports. A TURN server allocates a relay socket
  # per call and tells the client where to find it; behind a port mapping it
  # binds one address and advertises another, and the relay is then unreachable
  # while every call still connects over host candidates, so nothing ever says
  # the relay was never usable. On the host's own stack the address it discovers
  # is the address it advertises.
  podman run -d --name "$TURN_CONTAINER" --network=host \
    "$TURN_IMAGE" \
    -n \
    --listening-port="$TURN_PORT" \
    --realm=localhost \
    --use-auth-secret \
    --static-auth-secret="$TURN_SECRET" \
    --min-port="$TURN_RELAY_MIN" \
    --max-port="$TURN_RELAY_MAX" \
    --no-tls \
    --no-dtls \
    --no-multicast-peers \
    --allow-loopback-peers \
    --log-file=stdout >/dev/null

  # coturn exits immediately on a bad option rather than complaining, so make
  # sure it is still there before saying it started.
  sleep 1
  if [ "$(podman inspect -f '{{.State.Running}}' "$TURN_CONTAINER")" != true ]; then
    podman logs --tail 20 "$TURN_CONTAINER" >&2 || true
    die "coturn did not stay up"
  fi
  log "TURN server is up."
}

# A homeserver.yaml generated before TURN was part of this script has no
# `turn_uris`, and the endpoint then answers with an empty object that a client
# cannot tell from "this server has no TURN". Add it in place.
ensure_turn_config() {
  local config=$DATA/homeserver.yaml
  [ -f "$config" ] || return 0
  grep -q '^turn_shared_secret:' "$config" && return 0

  log "Pointing the existing configuration at the TURN server…"
  cat >> "$config" <<YAML

# Added by testing/local-homeserver.sh.
turn_uris:
  - "turn:127.0.0.1:$TURN_PORT?transport=udp"
  - "turn:127.0.0.1:$TURN_PORT?transport=tcp"
turn_shared_secret: "$TURN_SECRET"
turn_user_lifetime: 86400000
turn_allow_guests: false
YAML

  if podman container exists "$CONTAINER" 2>/dev/null \
    && [ "$(podman inspect -f '{{.State.Running}}' "$CONTAINER")" = true ]; then
    log "Restarting the homeserver so it reads the change…"
    podman restart "$CONTAINER" >/dev/null
    wait_for_server
  fi
}

# What a client is actually handed when it asks the homeserver where to find a
# TURN server. The username is a timestamp and the password an HMAC over it, so
# both change every time this is called.
turn_info() {
  local alice
  alice=$(login alice "$ALICE_PASS")
  [ -n "$alice" ] || die "Could not log in as alice"

  curl -sf "$HS/_matrix/client/v3/voip/turnServer" -H "Authorization: Bearer $alice"
}

# A homeserver.yaml generated before server notices were part of this script has
# no `server_notices` block, and the admin endpoint answers 400 without one. Add
# it in place rather than making the user throw the server away.
ensure_server_notices_config() {
  local config=$DATA/homeserver.yaml
  [ -f "$config" ] || return 0
  grep -q '^server_notices:' "$config" && return 0

  log "Turning server notices on in the existing configuration…"
  cat >> "$config" <<'YAML'

# Added by testing/local-homeserver.sh.
server_notices:
  system_mxid_localpart: notices
  system_mxid_display_name: "Server Notices"
  room_name: "Server Notices"
  room_topic: "Messages from the administrator of this homeserver"
  auto_join: false

# Without this, the notice Synapse writes itself carries `admin_contact: null`
# and the banner has nothing to offer a button for.
admin_contact: 'mailto:admin@localhost'
YAML

  if podman container exists "$CONTAINER" 2>/dev/null \
    && [ "$(podman inspect -f '{{.State.Running}}' "$CONTAINER")" = true ]; then
    log "Restarting the homeserver so it reads the change…"
    podman restart "$CONTAINER" >/dev/null
    wait_for_server
  fi
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

send_text() {
  local token=$1 room=$2 txn=$3 body=$4
  curl -sf -X PUT "$HS/_matrix/client/v3/rooms/$room/send/m.room.message/$txn" \
    -H "Authorization: Bearer $token" -H 'Content-Type: application/json' \
    -d "$body" >/dev/null
}

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

  # The server ACL subpage has nothing to show until a room has an ACL. This one
  # gets two, so the timeline holds both the first restriction and a change to
  # it — the two lines `server_acl_message()` tells apart.
  log "Creating the server ACL room…"
  local acl_room
  acl_room=$(create_room "$alice" '{"name": "ACL Room", "preset": "private_chat"}')

  curl -sf -X PUT "$HS/_matrix/client/v3/rooms/$acl_room/state/m.room.server_acl/" \
    -H "Authorization: Bearer $alice" -H 'Content-Type: application/json' \
    -d '{"allow": ["*"], "deny": [], "allow_ip_literals": false}' >/dev/null \
    || warn "the server refused the first ACL; the ACL Room will be empty"

  curl -sf -X PUT "$HS/_matrix/client/v3/rooms/$acl_room/state/m.room.server_acl/" \
    -H "Authorization: Bearer $alice" -H 'Content-Type: application/json' \
    -d '{"allow": ["*"], "deny": ["evil.example"], "allow_ip_literals": false}' >/dev/null \
    || warn "the server refused the second ACL"

  # Link previews need a room full of the cases the extraction rules are about,
  # because most of those rules are about what must *not* get a card.
  log "Creating the link room…"
  local link_room
  link_room=$(create_room "$alice" '{"name": "Link Room", "preset": "private_chat"}')

  send_text "$alice" "$link_room" link1 '{"msgtype": "m.text", "body": "A plain link: https://matrix.org"}'
  send_text "$alice" "$link_room" link2 '{"msgtype": "m.text", "body": "No scheme, still a link: spec.matrix.org"}'
  send_text "$alice" "$link_room" link3 "$(jq -nc '{
    msgtype: "m.text",
    body: "An anchor whose text is not the URL: the spec",
    format: "org.matrix.custom.html",
    formatted_body: "An anchor whose text is not the URL: <a href=\"https://spec.matrix.org/v1.19/\">the spec</a>"
  }')"
  send_text "$alice" "$link_room" link4 "$(jq -nc '{
    msgtype: "m.text",
    body: "In code, so no card: `https://matrix.org`",
    format: "org.matrix.custom.html",
    formatted_body: "In code, so no card: <code>https://matrix.org</code>"
  }')"
  send_text "$alice" "$link_room" link5 '{"msgtype": "m.text", "body": "A mention is not a link: @alice:localhost"}'
  send_text "$alice" "$link_room" link6 '{"msgtype": "m.text", "body": "Not a link at all: version 1.5, e.g. this one"}'
  send_text "$alice" "$link_room" link7 '{"msgtype": "m.text", "body": "First one only: https://matrix.org and https://spec.matrix.org"}'

  # The card must never appear here, whatever the setting says.
  log "Creating the encrypted room…"
  local encrypted_room
  encrypted_room=$(create_room "$alice" '{
    "name": "Encrypted Room",
    "preset": "private_chat",
    "initial_state": [{
      "type": "m.room.encryption",
      "state_key": "",
      "content": {"algorithm": "m.megolm.v1.aes-sha2"}
    }]
  }')

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
    --arg acl_room "$acl_room" \
    --arg link_room "$link_room" \
    --arg encrypted_room "$encrypted_room" \
    '$ARGS.named' > "$STATE"
}

# ------------------------------------------------------------ direct chat ----

# Make sure alice and bob have a direct chat.
#
# Nothing else in the seed is one: the rooms bob is in are ordinary rooms that
# happen to have two people in them. A DM is what the sidebar shows as a person
# rather than a room, and it is where anybody testing calls will look first.
#
# Idempotent, and outside the `seeded.json` gate, so a homeserver seeded before
# this existed gets one on the next `up`.
seed_direct_chat() {
  local alice bob existing dm
  alice=$(login alice "$ALICE_PASS")
  bob=$(login bob "$BOB_PASS")
  [ -n "$alice" ] || return 0

  # A user who has never had a direct chat has no `m.direct` at all, and the
  # homeserver answers 404. Under `set -e` with `pipefail` that ends the script,
  # so the failure is swallowed here rather than treated as one.
  existing=$(curl -sf "$HS/_matrix/client/v3/user/@alice:localhost/account_data/m.direct" \
    -H "Authorization: Bearer $alice" 2>/dev/null || true)
  existing=$(printf '%s' "$existing" | jq -r '.["@bob:localhost"][0] // empty' 2>/dev/null || true)

  if [ -n "$existing" ]; then
    log "alice and bob already have a direct chat."
    return 0
  fi

  log "Creating the direct chat between alice and bob…"
  dm=$(create_room "$alice" '{
    "preset": "trusted_private_chat",
    "is_direct": true,
    "invite": ["@bob:localhost"]
  }')
  [ -n "$dm" ] && [ "$dm" != null ] || { warn "could not create the direct chat"; return 0; }

  curl -sf -X POST "$HS/_matrix/client/v3/rooms/$dm/join" \
    -H "Authorization: Bearer $bob" -H 'Content-Type: application/json' \
    -d '{}' >/dev/null || true

  # `is_direct` on createRoom only marks the invite; the account data is what
  # actually makes it a DM for the person who created it, and Synapse does not
  # write it for them.
  curl -sf -X PUT "$HS/_matrix/client/v3/user/@alice:localhost/account_data/m.direct" \
    -H "Authorization: Bearer $alice" -H 'Content-Type: application/json' \
    -d "$(jq -nc --arg dm "$dm" '{"@bob:localhost": [$dm]}')" >/dev/null || true
  curl -sf -X PUT "$HS/_matrix/client/v3/user/@bob:localhost/account_data/m.direct" \
    -H "Authorization: Bearer $bob" -H 'Content-Type: application/json' \
    -d "$(jq -nc --arg dm "$dm" '{"@alice:localhost": [$dm]}')" >/dev/null || true

  log "Direct chat is $dm"
}

# --------------------------------------------------------------- notices ----

# Send a server notice to alice and put her in the room.
#
# The room is created by @notices:localhost, which is not a registered user —
# Synapse speaks as it without ever giving it an account — so the admin API
# cannot log in as it and nothing outside the server can act on its behalf. That
# is also why the notice cannot be pinned from here; see `limit` below for the
# path that makes Synapse pin one itself.
#
# The `m.server_notice` tag is room account data, and a client is sent none of
# that for a room it has only been invited to. So the tag, and with it the whole
# of the client behaviour this module describes, only appears once the invite is
# accepted. This joins alice for that reason.
send_notice() {
  local admin room
  admin=$(login admin admin-is-testing)
  [ -n "$admin" ] || die "Could not log in as admin"

  log "Sending a server notice to alice…"
  if ! curl -sf -X POST "$HS/_synapse/admin/v1/send_server_notice" \
      -H "Authorization: Bearer $admin" -H 'Content-Type: application/json' \
      -d "$(jq -nc '{
        user_id: "@alice:localhost",
        content: {
          msgtype: "m.server_notice",
          body: "This homeserver has exceeded its monthly active user limit. Please contact your administrator.",
          server_notice_type: "m.server_notice.usage_limit_reached",
          admin_contact: "mailto:admin@localhost",
          limit_type: "monthly_active_user"
        }
      }')" >/dev/null; then
    warn "the homeserver refused to send a server notice; is the server_notices block in homeserver.yaml?"
    return 1
  fi

  room=$(notice_room_for_alice)
  if [ -z "$room" ]; then
    warn "the notice was sent but alice is in no server notices room"
    return 0
  fi

  local alice
  alice=$(login alice "$ALICE_PASS")
  curl -sf -X POST "$HS/_matrix/client/v3/rooms/$room/join" \
    -H "Authorization: Bearer $alice" -H 'Content-Type: application/json' \
    -d '{}' >/dev/null || true

  log "Server notices room is $room"
}

# The room ID of alice's server notices room, joined or invited, or nothing.
notice_room_for_alice() {
  local alice
  alice=$(login alice "$ALICE_PASS")
  curl -sf "$HS/_matrix/client/v3/sync?timeout=0" -H "Authorization: Bearer $alice" \
    | jq -r '
        [ (.rooms.join // {} | to_entries[]
           | select(any(.value.account_data.events[]?;
                        .type == "m.tag" and (.content.tags | has("m.server_notice"))))),
          (.rooms.invite // {} | to_entries[]) ]
        | .[0].key // empty'
}

# Turn Synapse's monthly active user limit on or off.
#
# This is the only way to see an *active* notice without a registered account
# for the notices user: over the limit, Synapse sends a
# `m.server_notice.usage_limit_reached` notice itself and pins it, which is what
# makes it active, and unpins it when the limit is lifted. The cost is real —
# while it is on, nobody on this server can send a message or register — so it
# is its own command and not part of `up`.
limit() {
  local mode=${1:-on}
  local config=$DATA/homeserver.yaml
  [ -f "$config" ] || die "No homeserver configuration; run './testing/local-homeserver.sh up' first"

  # Drop any block this command added before, so the two directions are the
  # same operation with a different value.
  python3 - "$config" <<'PY'
import re, sys
path = sys.argv[1]
text = open(path).read()
text = re.sub(r"\n# Added by testing/local-homeserver\.sh \(limit\)\.\n(?:.*\n)*?# End limit\.\n", "\n", text)
open(path, "w").write(text)
PY

  if [ "$mode" = on ]; then
    log "Putting the homeserver over its monthly active user limit…"
    cat >> "$config" <<'YAML'

# Added by testing/local-homeserver.sh (limit).
limit_usage_by_mau: true
max_mau_value: 1
mau_trial_days: 0
# End limit.
YAML
  elif [ "$mode" = off ]; then
    log "Lifting the monthly active user limit…"
  else
    die "Unknown limit mode '$mode'. Try: on, off"
  fi

  podman restart "$CONTAINER" >/dev/null
  wait_for_server

  # Synapse acts on the limit when a user next does something, not on startup,
  # so give it something to act on in both directions.
  local alice room
  alice=$(login alice "$ALICE_PASS")
  curl -s -X PUT "$HS/_matrix/client/v3/rooms/$(jq -r .invite_room "$STATE")/send/m.room.message/limit-$RANDOM" \
    -H "Authorization: Bearer $alice" -H 'Content-Type: application/json' \
    -d '{"msgtype": "m.text", "body": "poke"}' >/dev/null || true
  curl -sf "$HS/_matrix/client/v3/sync?timeout=0" -H "Authorization: Bearer $alice" >/dev/null || true

  room=$(notice_room_for_alice)
  [ -n "$room" ] || return 0

  local pinned
  pinned=$(curl -sf "$HS/_matrix/client/v3/rooms/$room/state/m.room.pinned_events/" \
    -H "Authorization: Bearer $alice" 2>/dev/null | jq -r '.pinned | length // 0')

  log "Server notices room $room now has ${pinned:-0} pinned event(s)."
  if [ "$mode" = off ] && [ "${pinned:-0}" != 0 ]; then
    warn "Synapse has not unpinned it yet; it does that the next time it looks, so open the app and wait"
  fi
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

  log "Checking that the server ACL round trips…"
  local acl_room acl_before acl_after
  acl_room=$(jq -r .acl_room "$STATE")
  acl_before=$(curl -sf "$HS/_matrix/client/v3/rooms/$acl_room/state/m.room.server_acl/" \
    -H "Authorization: Bearer $alice" || echo '{}')

  # This is the change the subpage makes: add a blocked server, leave the allow
  # list and the IP literal switch alone.
  curl -sf -X PUT "$HS/_matrix/client/v3/rooms/$acl_room/state/m.room.server_acl/" \
    -H "Authorization: Bearer $alice" -H 'Content-Type: application/json' \
    -d '{"allow": ["*"], "deny": ["evil.example", "worse.example"], "allow_ip_literals": false}' \
    >/dev/null || { warn "the server refused the new ACL"; failed=1; }

  acl_after=$(curl -sf "$HS/_matrix/client/v3/rooms/$acl_room/state/m.room.server_acl/" \
    -H "Authorization: Bearer $alice")
  if [ "$(jq -c '.deny' <<<"$acl_after")" = '["evil.example","worse.example"]' ] \
     && [ "$(jq -c '.allow' <<<"$acl_after")" = '["*"]' ]; then
    printf '    m.room.server_acl round trip                   OK\n'
  else
    warn "the ACL did not survive the round trip"
    failed=1
  fi

  curl -sf -X PUT "$HS/_matrix/client/v3/rooms/$acl_room/state/m.room.server_acl/" \
    -H "Authorization: Bearer $alice" -H 'Content-Type: application/json' \
    -d "$acl_before" >/dev/null || true

  # Not a pass or a fail: it records whether the client-side warning is the only
  # thing standing between the user and a room they have shut themselves out of.
  log "Checking whether the server guards against shutting itself out…"
  if curl -sf -X PUT "$HS/_matrix/client/v3/rooms/$acl_room/state/m.room.server_acl/" \
      -H "Authorization: Bearer $alice" -H 'Content-Type: application/json' \
      -d '{"allow": ["*"], "deny": ["localhost"], "allow_ip_literals": false}' >/dev/null 2>&1; then
    printf '    the server ACCEPTED an ACL denying its own name —\n'
    printf '    the confirmation dialog in Commune is the only guard\n'
    curl -sf -X PUT "$HS/_matrix/client/v3/rooms/$acl_room/state/m.room.server_acl/" \
      -H "Authorization: Bearer $alice" -H 'Content-Type: application/json' \
      -d "$acl_before" >/dev/null || true
  else
    printf '    the server refused it as well\n'
  fi

  log "Checking the URL preview endpoint…"
  local preview
  preview=$(curl -sf -G "$HS/_matrix/client/v1/media/preview_url" \
    -H "Authorization: Bearer $alice" \
    --data-urlencode 'url=https://matrix.org' || true)

  if [ -z "$preview" ]; then
    warn "GET /_matrix/client/v1/media/preview_url was rejected — previews are off,"
    warn "or the container cannot reach the internet. Run 'clean' then 'up' if this"
    warn "homeserver was created before previews were added to the config."
    failed=1
  elif [ "$(jq -r 'has("og:title")' <<<"$preview")" = true ]; then
    printf '    GET /_matrix/client/v1/media/preview_url       OK\n'
    printf '    og:title  %s\n' "$(jq -r '."og:title"' <<<"$preview")"
    printf '    og:image  %s\n' "$(jq -r '."og:image" // "(none)"' <<<"$preview")"
  else
    warn "the endpoint answered but sent no og:title; the card will stay hidden"
    failed=1
  fi

  # Not a pass or a fail: it records whether the image really arrives as an
  # `mxc:` URI, which is the one thing the spec says is different from
  # OpenGraph, and the one thing the card refuses to render without.
  if [ -n "$preview" ]; then
    local image
    image=$(jq -r '."og:image" // ""' <<<"$preview")
    case "$image" in
      "") printf '    this page has no preview image\n' ;;
      mxc://*) printf '    the preview image is an mxc: URI, as specified\n' ;;
      *) printf '    the preview image is NOT an mxc: URI — Commune ignores it\n' ;;
    esac
  fi

  log "Checking the TURN credentials the homeserver hands out…"
  local turn turn_user turn_pass turn_uri relayed
  turn=$(turn_info || true)
  turn_user=$(printf '%s' "$turn" | jq -r '.username // empty')
  turn_pass=$(printf '%s' "$turn" | jq -r '.password // empty')
  turn_uri=$(printf '%s' "$turn" | jq -r '.uris[0] // empty')

  if [ -z "$turn_user" ] || [ -z "$turn_pass" ] || [ -z "$turn_uri" ]; then
    warn "GET /_matrix/client/v3/voip/turnServer returned nothing usable; is turn_uris set?"
    failed=1
  else
    printf '    GET /_matrix/client/v3/voip/turnServer         OK\n'
    printf '    %s as %s\n' "$turn_uri" "$turn_user"

    # Credentials that parse are not credentials that work. Allocate a relay
    # and push packets through it, which is the only thing that proves the
    # shared secret the homeserver signs with is the one coturn verifies.
    if podman container exists "$TURN_CONTAINER" 2>/dev/null; then
      relayed=$(podman exec "$TURN_CONTAINER" turnutils_uclient \
        -T -u "$turn_user" -w "$turn_pass" -p "$TURN_PORT" -n 4 -m 1 127.0.0.1 2>&1 \
        | sed -n 's/.*start_mclient: tot_send_msgs=[0-9]*, tot_recv_msgs=\([0-9]*\).*/\1/p' \
        | tail -1)

      if [ "${relayed:-0}" -gt 0 ]; then
        printf '    a relay allocation carried %s packets       OK\n' "$relayed"
      else
        warn "the TURN server took the credentials but relayed nothing"
        failed=1
      fi
    else
      printf '    no TURN container to allocate against; skipped\n'
    fi
  fi

  log "Checking the server notices room…"
  local notice_room pinned
  notice_room=$(notice_room_for_alice)

  if [ -z "$notice_room" ]; then
    warn "alice has no server notices room; run './testing/local-homeserver.sh notice' first"
    failed=1
  elif curl -sf "$HS/_matrix/client/v3/user/@alice:localhost/rooms/$notice_room/tags" \
      -H "Authorization: Bearer $alice" \
      | jq -e '.tags | has("m.server_notice")' >/dev/null; then
    printf '    m.server_notice tag on %s  OK\n' "$notice_room"

    # The banner is driven by the pinned events, and only Synapse itself pins a
    # notice, when it is over a limit. Absence is not a failure here.
    pinned=$(curl -sf "$HS/_matrix/client/v3/rooms/$notice_room/state/m.room.pinned_events/" \
      -H "Authorization: Bearer $alice" | jq -r '.pinned[0] // empty')

    if [ -n "$pinned" ]; then
      printf '    m.room.pinned_events holds %s  OK\n' "$pinned"
    else
      printf '    nothing pinned, so no banner — run "limit on" to make Synapse pin one\n'
    fi
  else
    warn "the server notices room carries no m.server_notice tag; the client has nothing to recognise"
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

  Server ACLs — open Room Details ▸ Server Access on
    "ACL Room"               "*" is allowed and "evil.example" is blocked;
                             the timeline shows the first ACL and the change
    Remove "*" and save      the page refuses it: nobody could take part
    Block "localhost"        it asks first, because that is your own server

  Link previews — open "Link Room"; a card should appear under
    "A plain link"           matrix.org, with title, description and image
    "No scheme"              the same, the linkifier's rule is matched
    "An anchor"              the card is for the href, not the link text
    "In code"                NO card: it was written to be read
    "A mention"              NO card: @alice:localhost is not a page
    "Not a link at all"      NO card: 1.5 and e.g. are not domains
    "First one only"         exactly one card, for the first of the two
    "Encrypted Room"         post a link yourself: NO card, ever, and the
                             switch below cannot turn one on
    Settings ▸ General ▸ Messages ▸ Link Previews turns the rest off

  Server notices — log in as alice; the homeserver has sent her one
    Sidebar                  a "Server Notices" section sits above Favorites,
                             with a warning icon on the room
    Open the room            the notice reads as a warning in the timeline
    Any other room           post a message with msgtype m.server_notice from
                             another client: it must not appear at all
    Try to leave it          Synapse allows it; a server that refuses gets a
                             message saying so rather than "Could not leave"

    ./testing/local-homeserver.sh notice      sends another one
    ./testing/local-homeserver.sh limit on    Synapse sends and PINS one of its
                             own, which is what raises the banner over the
                             timeline with its "Contact Administrator" button.
                             Nobody can send a message while this is on.
    ./testing/local-homeserver.sh limit off   lifts it. Synapse unpins its own
                             notice the next time it looks at the account, so
                             the banner goes away a beat later, not at once.

  Calls — alice and bob have a direct chat; the call buttons are in its header
    Any two-person room            also gets them, DM or not: the spec's rule is
                                   about the member count, not about m.direct
    ./testing/local-homeserver.sh turn        shows what a client is given
    ./testing/local-homeserver.sh check       allocates a relay with them

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
    start_turn_server
    ensure_server_notices_config
    ensure_turn_config
    seed
    seed_direct_chat
    send_notice || true
    summary
    ;;
  turn)
    need podman; need curl; need jq
    start_turn_server
    ensure_turn_config
    turn_info | jq .
    ;;
  notice)
    need podman; need curl; need jq
    ensure_server_notices_config
    send_notice
    ;;
  limit)
    need podman; need curl; need jq; need python3
    limit "${2:-on}"
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
    podman stop "$TURN_CONTAINER" >/dev/null 2>&1 && log "TURN server stopped." || true
    podman stop "$CONTAINER" >/dev/null 2>&1 && log "Homeserver stopped." || log "Not running."
    ;;
  clean)
    podman rm -f "$TURN_CONTAINER" >/dev/null 2>&1 || true
    podman rm -f "$CONTAINER" >/dev/null 2>&1 || true
    # The data is written by the container as root, so it may need help.
    rm -rf "$DATA" 2>/dev/null || podman unshare rm -rf "$DATA"
    log "Homeserver and its data are gone."
    ;;
  *)
    die "Unknown command '$1'. Try: up, notice, limit, turn, check, reports, down, clean"
    ;;
esac
