# The Android room screen's touch handling — downstream implementation notes

This file is the ledger for four things the Kotlin application's room screen
(`android-kotlin/app/src/main/java/io/github/steeb_k/commune/ui/RoomScreen.kt`)
does with a finger that it did not do before 15 October 2026: a long press
anywhere on a message opens its action sheet, a swipe towards the end edge
arms a reply, the attach menu opens without closing the keyboard, and the
attach menu can take a photo with the system camera. `doc/chat-bubbles.md` is
the GTK bubbled layout and says nothing about Android; the Kotlin bubble's
shape is the comment on `MessageBubble`, which points there for the
measurements. See `fork.md` for why none of this goes upstream.

**Status: built on 15 October 2026 and never seen on a screen**, emulator or
phone. The checks are in `doc/eyeball-android.md` under the section dated 15
October 2026.

## Long press anywhere on a message

The bubble's `combinedClickable` always opened the action sheet on a long
press, but the picture, the video poster, the "play video" row, the map, the
"open in maps" row and the link-preview card each had a plain `clickable` of
their own, and a clickable child takes the whole gesture, long press
included. The bubble never heard it, so the only way to reach "Save to
Downloads" for a picture was to find the sliver of bubble padding around it.

`MessageBubble` now builds one `onBubbleLongPress` — toggle selection in
select mode, otherwise `state.showActionSheet(event)` — and every one of those
children is a `combinedClickable` that hands its long press to it.
`UrlPreviewCard` takes it as a new `onLongClick` parameter, null by default so
its other callers are unchanged. The children's taps go through `childTap`,
which in select mode selects the message instead of opening the picture, the
player or the map, as a tap on the bubble's padding already did; the link card
does the same when it has been given `onLongClick`.

## Swipe to reply

`Modifier.swipeToReply(key, onReply)` in `RoomScreen.kt`, applied to the
message's outer row. The row follows the finger one for one up to 64dp, then
at a third of the pace up to a stop at 96dp, so the point of no return is felt
as the row getting heavier. A reply arrow (`Icons.AutoMirrored.Filled.Reply`,
in the primary colour) is drawn behind the row at the start edge and fades in
with the distance; the phone ticks once, a `LongPress` haptic, when the
threshold is crossed. Letting go past it calls `state.startReply(event)` and
the row springs back; letting go short of it only springs back. Only
horizontal drags are claimed, so the timeline still scrolls, and a drag that
begins on a code block's own sideways scroll moves the code. Distances run
from the start edge, so in a right-to-left layout the swipe and the arrow
mirror; the lambda form of `offset` is not mirrored by Compose, which is why
the modifier names the edge itself.

With `onReply` null the modifier returns `this` — no drag handler, no drawing.
Only `RoomScreen`'s own `Timeline` passes `onSwipeReply`, because the reply it
arms is the one the room's composer sends; the thread view and the pinned
list get null and their messages do not move. Within the room it is also null
for a message with no event ID yet (a local echo), for the "muted" kinds that
are not text, media or a location, and while messages are being selected.

## The attach menu keeps the keyboard

The composer's attach menu is a `DropdownMenu`, which is a popup window, and a
focusable popup takes window focus from the activity when it opens: the text
field lost focus, the keyboard closed, the room column's `imePadding()`
collapsed and the composer dropped to the bottom of the screen, taking the
menu that hangs off the attach button with it. The menu is now opened with
`PopupProperties(focusable = false, dismissOnClickOutside = true)`, so the
keyboard stays up and the menu appears above the composer where the button
is.

Two consequences, both deliberate. A non-focusable popup does not swallow the
outside tap that dismisses it, so a tap on the attach button itself would
close the menu on the press and open it again on the release; the button
therefore ignores a press within 300ms of an outside dismissal
(`attachMenuDismissedAt`, against `SystemClock.uptimeMillis()`). And a
non-focusable window never receives the back key, so back does not close the
menu: with the keyboard up the first back closes the keyboard anyway, and with
it down back leaves the room, which takes the menu with it.

## Take Photo

A "Take Photo" item in the attach menu, between "Attach File" and "Share
Location", present when `CommuneState.takePhoto` is set — which
`MainActivity.onCreate` does, and clears with the other activity callbacks.
`MainActivity.takePhoto()` asks for the `CAMERA` runtime permission through
the existing `state.ensureCameraPermission`. That is needed although the app
never opens the camera itself: `CAMERA` is declared in the manifest for calls
and the QR scanner, and an app that declares it and has not been granted it
is refused `IMAGE_CAPTURE` with a `SecurityException`.

It then clears `cacheDir/outgoing/camera/`, names a file
`IMG_yyyyMMdd_HHmmss.jpg` there, and hands the system camera app a
`FileProvider` URI for it through `ActivityResultContracts.TakePicture`. The
previous shot can be deleted at that point because `queueAttachmentsFromUris`
has long since copied it into `outgoing/`. A shot that comes back goes through
`queueAttachmentsFromUris` exactly as a picked file does, so it lands in the
same "Send File?" preview and nothing is sent until that is confirmed; a
cancelled shot's empty file is deleted. With no camera app, the file is
deleted and a toast says so.

`TakePicture` reports only whether a picture was taken, not where, so the
activity remembers `cameraUri`, and keeps it across a recreation in
`onSaveInstanceState` under `"camera_uri"`: rotating the phone inside the
camera app recreates this activity behind it.

The URI is served by the existing provider with the authority
`${applicationId}.updates`, which until now exposed only `cache/updates/` for
handing a downloaded APK to the installer. `res/xml/updates_paths.xml` gains a
second `cache-path`, `outgoing/camera/`. It is the same provider rather than a
second one because two `FileProvider`s in one manifest each need their own
subclass, and the authority keeps the name "updates" because `Updates.kt` and
`doc/updates.md` both name it. From API 30 an app sees only the packages it
declares an interest in, so the manifest gains a `<queries>` entry for
`android.media.action.IMAGE_CAPTURE`; without it the intent resolves to
nothing on a phone that has a camera.

## Rebase guide

Nothing here is in a file Fractal has, so an upstream release cannot conflict
with it. Within the fork: anything new drawn inside `MessageBubble` that
answers a tap must also take `onBubbleLongPress`, or long press stops working
on it again; a new list that shows messages passes `onSwipeReply` only if its
composer is the room's. Anyone renaming the `updates` provider authority
changes `MainActivity.takePhoto` with it, and anyone narrowing
`updates_paths.xml` keeps `outgoing/camera/` or Take Photo stops resolving.
