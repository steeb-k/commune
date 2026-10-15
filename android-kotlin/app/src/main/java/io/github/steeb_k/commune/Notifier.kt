// Message notifications from the sidebar's own numbers: the server's
// per-room notification counts arrive with every room-list update, and
// the shade mirrors them — one notification per room, gone when the
// count clears. The line it shows is the room's latest message when that
// is readable, announced by event through the record the push service
// shares (MessageNotifications.kt); the counts are what the server
// already vouches for.
package io.github.steeb_k.commune

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import io.github.steeb_k.commune.core.CoreApp
import io.github.steeb_k.commune.core.FfiRoom
import kotlin.concurrent.thread

/// `app` is the core, asked for when a message is posted: the notifier
/// is built before the core is.
class Notifier(private val context: Context, private val app: () -> CoreApp) {
    private val manager = context.getSystemService(NotificationManager::class.java)

    /// Whether notifications are wanted at all: the account setting, and
    /// the room being looked at right now never notifies.
    var enabled: Boolean = true
    var visibleRoomId: String? = null

    /// The room a call is happening in, which announces itself: an
    /// m.call.invite notifies by push rule like any other event, so a
    /// ringing phone was also posting "1 new message" for the same call.
    /// The call handler owns that announcement; this stays quiet.
    var callRoomId: String? = null

    init {
        manager.createNotificationChannel(
            NotificationChannel(
                CHANNEL_ID,
                "Messages",
                NotificationManager.IMPORTANCE_HIGH,
            )
        )
    }

    /// Mirror the given rooms' notification counts into the shade.
    ///
    /// What counts as news is the room's latest message, by event, through
    /// the same [AnnouncedMessages] the push service uses: a count that
    /// rose is not news by itself when the message behind it was already
    /// shown by a push, or shown here and swiped away. Only a room with no
    /// readable latest message falls back on the count, since there is
    /// nothing else to tell one announcement from the next.
    fun update(rooms: List<FfiRoom>) {
        // Asked for now rather than remembered: the bookkeeping must be the
        // same account's the push service records under.
        val account = app().sessionUserId().orEmpty()
        // What the shade holds right now. The push service posts rooms
        // this bookkeeping never saw, and a room read on another device
        // comes down whichever source put it up: the count at zero is the
        // server saying every device is done with it.
        val shown = manager.activeNotifications.mapTo(HashSet()) { it.id }
        for (room in rooms) {
            val id = room.roomId.hashCode()
            val count = room.notificationCount.toLong()
            if (count == 0L || room.roomId == visibleRoomId ||
                room.roomId == callRoomId || !enabled
            ) {
                val had = AnnouncedMessages.forget(context, account, room.roomId)
                if (had || id in shown) manager.cancel(id)
                continue
            }

            // The latest message, when it is readable, with its sender. Our
            // own message is never shown: the count that raised this
            // notification is always for someone else, but the room list can
            // carry a reply we just sent as the latest event before that
            // message settles, which is how our own words came to appear on
            // an incoming notification.
            val body = room.latestEventBody?.takeIf { !room.latestEventIsOwn }
            val eventId = room.latestEventId
            when {
                body != null && eventId != null -> {
                    val timestamp = room.latestEventTimestamp.toLong()
                    if (AnnouncedMessages.claim(context, account, room.roomId, eventId, timestamp)) {
                        AnnouncedMessages.setKnown(context, account, room.roomId, count)
                        post(room, body, timestamp, count.toInt())
                    } else if (id in shown) {
                        // Already announced, and still in the shade: only
                        // the count moves. Swiped away, it stays away.
                        AnnouncedMessages.setKnown(context, account, room.roomId, count)
                        refreshRoomCount(context, room.roomId, count.toInt())
                    }
                }
                // Nothing readable to show: the count itself is the line,
                // and only more unread than last announced is news — the
                // same count after a restart is not.
                count > AnnouncedMessages.known(context, account, room.roomId) -> {
                    AnnouncedMessages.setKnown(context, account, room.roomId, count)
                    val countText = if (count == 1L) "1 new message" else "$count new messages"
                    postRoomMessage(
                        context,
                        room.roomId,
                        io.github.steeb_k.commune.ui.roomName(room),
                        room.isDirect,
                        null,
                        countText,
                        count.toInt(),
                        summary = true,
                    )
                }
            }
        }
    }

    private fun post(room: FfiRoom, body: String, timestamp: Long, count: Int) {
        val name = io.github.steeb_k.commune.ui.roomName(room)
        val senderId = room.latestEventSender
        // Naming the sender reads the store and may fetch their picture;
        // the room-list update this rides on is on the main thread.
        thread {
            val sender = senderId?.let { senderPerson(app(), room.roomId, it) }
            postRoomMessage(context, room.roomId, name, room.isDirect, sender, body, count, timestamp)
        }
    }

    companion object {
        private const val CHANNEL_ID = MESSAGES_CHANNEL_ID
    }
}

/// The lock-screen stand-in: app name and "New message", nothing else.
internal fun redactedNotification(context: Context, channelId: String): Notification =
    Notification.Builder(context, channelId)
        .setSmallIcon(R.drawable.ic_notify_symbolic)
        .setContentTitle("Commune")
        .setContentText("New message")
        .build()
