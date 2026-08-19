#!/usr/bin/env python3
"""Create an image pack on a homeserver, for testing.

Commune cannot create packs yet, and there is no registry of packs to pull
one from, so this builds one and uploads it.

By default the images are emoji rendered from the system emoji font, which
needs nothing from the network. Pass --images to use a directory of files
instead; their names become the shortcodes.

Examples:

    # A pack in the state of a room.
    ./image-pack-tool.py --homeserver https://matrix.example.org \\
        --user alice --password hunter2 --room '!abc:example.org'

    # A personal pack, in the account data.
    ./image-pack-tool.py --homeserver https://matrix.example.org \\
        --token syt_… --personal

    # A pack under its stable name, and enabled in every room.
    ./image-pack-tool.py --homeserver https://matrix.example.org \\
        --token syt_… --room '!abc:example.org' --stable --enable-globally
"""

import argparse
import io
import json
import mimetypes
import pathlib
import sys
import urllib.error
import urllib.parse
import urllib.request

# The emoji of the generated pack, and the shortcode of each.
DEMO_EMOJI = [
    ("cat", "\U0001F431"),
    ("thumbs_up", "\U0001F44D"),
    ("party", "\U0001F389"),
    ("fire", "\U0001F525"),
    ("rocket", "\U0001F680"),
    ("hundred", "\U0001F4AF"),
    ("crab", "\U0001F980"),
    ("penguin", "\U0001F427"),
]

# The specification asks for stickers of at least 512 pixels.
STICKER_SIZE = 512

# The event types we can write. The unstable ones are what clients use.
ROOM_PACK_UNSTABLE = "im.ponies.room_emotes"
ROOM_PACK_STABLE = "m.room.image_pack"
USER_PACK = "im.ponies.user_emotes"
ENABLED_PACKS_UNSTABLE = "im.ponies.emote_rooms"
ENABLED_PACKS_STABLE = "m.image_pack.rooms"


def die(message):
    print(f"error: {message}", file=sys.stderr)
    sys.exit(1)


class Client:
    """The bit of the client-server API that we need."""

    def __init__(self, homeserver, token=None):
        self.homeserver = homeserver.rstrip("/")
        self.token = token

    def _request(self, method, path, body=None, content_type=None, query=None):
        url = f"{self.homeserver}{path}"
        if query:
            url += "?" + urllib.parse.urlencode(query)

        headers = {}
        if self.token:
            headers["Authorization"] = f"Bearer {self.token}"
        if content_type:
            headers["Content-Type"] = content_type

        request = urllib.request.Request(url, data=body, headers=headers, method=method)
        try:
            with urllib.request.urlopen(request) as response:
                return json.load(response)
        except urllib.error.HTTPError as error:
            detail = error.read().decode("utf-8", "replace")
            die(f"{method} {path} failed with {error.code}: {detail}")
        except urllib.error.URLError as error:
            die(f"could not reach {self.homeserver}: {error.reason}")

    def _json_request(self, method, path, content):
        return self._request(
            method,
            path,
            body=json.dumps(content).encode("utf-8"),
            content_type="application/json",
        )

    def login(self, user, password):
        content = {
            "type": "m.login.password",
            "identifier": {"type": "m.id.user", "user": user},
            "password": password,
            "initial_device_display_name": "image-pack-tool",
        }
        response = self._json_request("POST", "/_matrix/client/v3/login", content)
        self.token = response["access_token"]
        return response["user_id"]

    def whoami(self):
        return self._request("GET", "/_matrix/client/v3/account/whoami")["user_id"]

    def upload(self, data, filename, mimetype):
        response = self._request(
            "POST",
            "/_matrix/media/v3/upload",
            body=data,
            content_type=mimetype,
            query={"filename": filename},
        )
        return response["content_uri"]

    def send_message(self, room_id, transaction_id, content):
        room = urllib.parse.quote(room_id, safe="")
        path = f"/_matrix/client/v3/rooms/{room}/send/m.room.message/{transaction_id}"
        return self._json_request("PUT", path, content)

    def set_room_state(self, room_id, event_type, state_key, content):
        room = urllib.parse.quote(room_id, safe="")
        state_key = urllib.parse.quote(state_key, safe="")
        path = f"/_matrix/client/v3/rooms/{room}/state/{event_type}/{state_key}"
        return self._json_request("PUT", path, content)

    def account_data(self, user_id, event_type):
        user = urllib.parse.quote(user_id, safe="")
        path = f"/_matrix/client/v3/user/{user}/account_data/{event_type}"
        try:
            return self._request("GET", path)
        except SystemExit:
            # There is no such event yet, which is not an error here.
            return {}

    def set_account_data(self, user_id, event_type, content):
        user = urllib.parse.quote(user_id, safe="")
        path = f"/_matrix/client/v3/user/{user}/account_data/{event_type}"
        return self._json_request("PUT", path, content)


def render_emoji(emoji, size):
    """Render the given emoji to the bytes of a square PNG."""
    try:
        from PIL import Image, ImageDraw, ImageFont
    except ImportError:
        die("rendering emoji needs Pillow; install it, or pass --images")

    candidates = sorted(pathlib.Path("/usr/share/fonts").rglob("*moji*.tt[fc]"))
    candidates += sorted(pathlib.Path("/usr/share/fonts").rglob("*moji*.otf"))
    if not candidates:
        die("no emoji font found under /usr/share/fonts; pass --images instead")

    # A colour emoji font usually only has one bitmap size, and refuses the
    # others, so try each until one is accepted.
    font = None
    for path in candidates:
        for font_size in (109, 128, 96, 64, 137):
            try:
                font = ImageFont.truetype(str(path), font_size)
                break
            except OSError:
                continue
        if font is not None:
            break
    if font is None:
        die("could not load an emoji font at any size; pass --images instead")

    canvas = Image.new("RGBA", (512, 512), (0, 0, 0, 0))
    draw = ImageDraw.Draw(canvas)
    draw.text((256, 256), emoji, font=font, anchor="mm", embedded_color=True)

    box = canvas.getbbox()
    if box is None:
        die(f"the emoji font rendered nothing for {emoji!r}")
    glyph = canvas.crop(box)

    # Pad to a square so that the aspect ratio is kept when resizing.
    side = max(glyph.size)
    square = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    square.paste(glyph, ((side - glyph.width) // 2, (side - glyph.height) // 2))
    square = square.resize((size, size), Image.LANCZOS)

    buffer = io.BytesIO()
    square.save(buffer, format="PNG")
    return buffer.getvalue()


def demo_images(size):
    """The images of the generated pack, as (shortcode, bytes, name, type)."""
    for shortcode, emoji in DEMO_EMOJI:
        print(f"  rendering :{shortcode}: {emoji}")
        yield shortcode, render_emoji(emoji, size), f"{shortcode}.png", "image/png"


def directory_images(directory):
    """The images of a directory, as (shortcode, bytes, name, type)."""
    paths = sorted(p for p in pathlib.Path(directory).iterdir() if p.is_file())
    if not paths:
        die(f"no files in {directory}")

    for path in paths:
        mimetype, _ = mimetypes.guess_type(path.name)
        if mimetype is None or not mimetype.startswith("image/"):
            print(f"  skipping {path.name}, not an image")
            continue

        # The grammar of a shortcode is [A-Za-z0-9_-], up to 100 bytes.
        shortcode = "".join(
            c if (c.isascii() and (c.isalnum() or c in "-_")) else "_" for c in path.stem
        )[:100]
        if not shortcode:
            print(f"  skipping {path.name}, its name has no usable characters")
            continue

        print(f"  reading :{shortcode}: from {path.name}")
        yield shortcode, path.read_bytes(), path.name, mimetype


def image_info(data, mimetype):
    """The info block of an image, which m.sticker events reuse."""
    info = {"mimetype": mimetype, "size": len(data)}
    try:
        from PIL import Image

        with Image.open(io.BytesIO(data)) as image:
            info["w"], info["h"] = image.size
    except Exception:
        # The dimensions are optional.
        pass
    return info


def escape(text):
    """Escape the text for inclusion in HTML."""
    return (
        text.replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
        .replace('"', "&quot;")
    )


def send_emoticons(client, room_id, images):
    """Send messages using the images inline, to test their rendering.

    Commune cannot send custom emoticons yet, so this stands in for a client
    that can. The height attribute is the one the specification requires, for
    the clients that do not support image packs.
    """
    shortcodes = list(images)[:3]

    def img(shortcode):
        image = images[shortcode]
        body = image.get("body") or shortcode
        return (
            f'<img data-mx-emoticon src="{escape(image["url"])}" '
            f'alt="{escape(body)}" title="{escape(shortcode)}" height="32">'
        )

    messages = [
        (
            "a custom emoticon in a sentence",
            "Look at this " + " ".join(f":{s}:" for s in shortcodes[:1]) + " one",
            "Look at this " + "".join(img(s) for s in shortcodes[:1]) + " one",
        ),
        (
            "only custom emoticons",
            " ".join(f":{s}:" for s in shortcodes),
            "".join(img(s) for s in shortcodes),
        ),
        (
            "a custom emoticon next to markup",
            "bold and " + f":{shortcodes[0]}:",
            "<b>bold</b> and " + img(shortcodes[0]),
        ),
        (
            "an image that is not an emoticon, which must stay text",
            "not an emoticon",
            f'<img src="{escape(images[shortcodes[0]]["url"])}" alt="not an emoticon">',
        ),
    ]

    for index, (description, plain, formatted) in enumerate(messages):
        client.send_message(
            room_id,
            f"image-pack-tool-{index}",
            {
                "msgtype": "m.text",
                "body": plain,
                "format": "org.matrix.custom.html",
                "formatted_body": formatted,
            },
        )
        print(f"  sent {description}")


def main():
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("--homeserver", required=True, help="e.g. https://matrix.example.org")
    parser.add_argument("--token", help="an access token; otherwise log in")
    parser.add_argument("--user", help="the localpart to log in as")
    parser.add_argument("--password", help="the password to log in with")
    parser.add_argument("--room", help="the room to put the pack in, e.g. '!abc:example.org'")
    parser.add_argument(
        "--personal", action="store_true", help="put the pack in the account data instead"
    )
    parser.add_argument("--state-key", default="testpack", help="the identifier of the pack")
    parser.add_argument("--name", default="Test Pack", help="the display name of the pack")
    parser.add_argument(
        "--usage",
        choices=["sticker", "emoticon", "both"],
        default="both",
        help="what the pack is for; 'both' leaves it unset, which means everything",
    )
    parser.add_argument(
        "--stable", action="store_true", help="write the stable event names instead"
    )
    parser.add_argument(
        "--enable-globally",
        action="store_true",
        help="also enable the pack of the room in every room",
    )
    parser.add_argument(
        "--send-emoticons",
        action="store_true",
        help="also send a message using the images inline, to test their rendering",
    )
    parser.add_argument("--images", help="a directory of images to use instead of emoji")
    parser.add_argument(
        "--size", type=int, default=STICKER_SIZE, help="the size of the rendered emoji"
    )
    args = parser.parse_args()

    if not args.room and not args.personal:
        die("pass --room, --personal, or both")
    if not args.token and not (args.user and args.password):
        die("pass --token, or --user and --password")

    client = Client(args.homeserver, args.token)
    if args.token:
        user_id = client.whoami()
        print(f"using the token of {user_id}")
    else:
        user_id = client.login(args.user, args.password)
        print(f"logged in as {user_id}")

    print("preparing the images:")
    source = directory_images(args.images) if args.images else demo_images(args.size)

    images = {}
    for shortcode, data, filename, mimetype in source:
        uri = client.upload(data, filename, mimetype)
        images[shortcode] = {
            "url": uri,
            "body": shortcode.replace("_", " "),
            "info": image_info(data, mimetype),
        }
        print(f"    uploaded to {uri}")

    if not images:
        die("no image to put in the pack")

    pack = {"display_name": args.name}
    if args.usage != "both":
        pack["usage"] = [args.usage]
    content = {"images": images, "pack": pack}

    if args.room:
        event_type = ROOM_PACK_STABLE if args.stable else ROOM_PACK_UNSTABLE
        client.set_room_state(args.room, event_type, args.state_key, content)
        print(f"put {len(images)} images in {args.room} as {event_type}/{args.state_key}")

        if args.enable_globally:
            event_type = ENABLED_PACKS_STABLE if args.stable else ENABLED_PACKS_UNSTABLE
            enabled = client.account_data(user_id, event_type)
            rooms = enabled.get("rooms") or {}
            # Keep the properties we do not know about, as the specification asks.
            rooms.setdefault(args.room, {}).setdefault(args.state_key, {})
            enabled["rooms"] = rooms
            client.set_account_data(user_id, event_type, enabled)
            print(f"enabled it in every room, through {event_type}")

    if args.personal:
        client.set_account_data(user_id, USER_PACK, content)
        print(f"put {len(images)} images in the personal pack, as {USER_PACK}")

    if args.send_emoticons:
        if not args.room:
            die("--send-emoticons needs --room")
        send_emoticons(client, args.room, images)

    print("\nOpen the room in Commune and click the sticker button in the composer.")
    print("If a pack does not appear, leave the room and come back: the picker")
    print("only loads the packs once per room.")


if __name__ == "__main__":
    main()
