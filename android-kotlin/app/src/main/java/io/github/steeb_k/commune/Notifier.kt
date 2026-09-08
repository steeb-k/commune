// Message notifications from the sidebar's own numbers: the server's
// per-room notification counts arrive with every room-list update, and
// the shade mirrors them — one notification per room, gone when the
// count clears. Rich per-message bodies arrive with the push-rules
// chunk; the counts are what the server already vouches for.
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

    /// The per-room counts already notified, persisted so an app restart
    /// does not re-announce the same unread messages.
    private val posted = context.getSharedPreferences("notified", Context.MODE_PRIVATE)

    /// Whether notifications are wanted at all: the account setting, and
    /// the room being looked at right now never notifies.
    var enabled: Boolean = true
    var visibleRoomId: String? = null

    /// The room a call is happening in, which announces itself: an
    /// m.call.invite notifies by push rule like any other event, so a
    /// ringing phone was also posting "1 new message" for the same call.
    /// The call handler owns that announcement; this stays quiet.
    var callRoomId: String? = null

    /// The active account, keying the bookkeeping: what was announced
    /// for one account must not silence another's.
    var accountKey: String = ""

    private fun postedKey(roomId: String) =
        if (accountKey.isEmpty()) roomId else "$accountKey|$roomId"

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
    fun update(rooms: List<FfiRoom>) {
        for (room in rooms) {
            val count = room.notificationCount.toLong()
            val known = posted.getLong(postedKey(room.roomId), 0L)
            when {
                count == 0L || room.roomId == visibleRoomId ||
                    room.roomId == callRoomId || !enabled -> {
                    if (known != 0L) {
                        posted.edit().remove(postedKey(room.roomId)).apply()
                        manager.cancel(room.roomId.hashCode())
                    }
                }
                // Only more unread than last announced is news; the same
                // count after a restart is not.
                count > known -> {
                    posted.edit().putLong(postedKey(room.roomId), count).apply()
                    post(room, room.notificationCount)
                }
            }
        }
    }

    private fun post(room: FfiRoom, count: ULong) {
        val name = io.github.steeb_k.commune.ui.roomName(room)
        val countText = if (count == 1uL) "1 new message" else "$count new messages"
        // The latest message, when it is readable, with its sender. Our
        // own message is never shown: the count that raised this
        // notification is always for someone else, but the room list can
        // carry a reply we just sent as the latest event before that
        // message settles, which is how our own words came to appear on
        // an incoming notification. Without a readable message the count
        // itself is the line.
        val body = room.latestEventBody?.takeIf { !room.latestEventIsOwn }
        val senderId = if (body != null) room.latestEventSender else null
        // Naming the sender reads the store and may fetch their picture;
        // the room-list update this rides on is on the main thread.
        thread {
            val sender = senderId?.let { senderPerson(app(), room.roomId, it) }
            postRoomMessage(
                context,
                room.roomId,
                name,
                room.isDirect,
                sender,
                body ?: countText,
                count.toInt(),
            )
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
