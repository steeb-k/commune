// Message notifications from the sidebar's own numbers: the server's
// per-room notification counts arrive with every room-list update, and
// the shade mirrors them — one notification per room, gone when the
// count clears. Rich per-message bodies arrive with the push-rules
// chunk; the counts are what the server already vouches for.
package io.github.steeb_k.commune

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import io.github.steeb_k.commune.core.FfiRoom

class Notifier(private val context: Context) {
    private val manager = context.getSystemService(NotificationManager::class.java)

    /// The per-room counts already notified, persisted so an app restart
    /// does not re-announce the same unread messages.
    private val posted = context.getSharedPreferences("notified", Context.MODE_PRIVATE)

    /// Whether notifications are wanted at all: the account setting, and
    /// the room being looked at right now never notifies.
    var enabled: Boolean = true
    var visibleRoomId: String? = null

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
                count == 0L || room.roomId == visibleRoomId || !enabled -> {
                    if (known != 0L) {
                        posted.edit().remove(postedKey(room.roomId)).apply()
                        manager.cancel(room.roomId.hashCode())
                    }
                }
                // Only more unread than last announced is news; the same
                // count after a restart is not.
                count > known -> {
                    posted.edit().putLong(postedKey(room.roomId), count).apply()
                    manager.notify(
                        room.roomId.hashCode(),
                        build(room, room.notificationCount),
                    )
                }
            }
        }
    }

    private fun build(room: FfiRoom, count: ULong): Notification {
        val openApp = PendingIntent.getActivity(
            context,
            room.roomId.hashCode(),
            Intent(context, MainActivity::class.java)
                .putExtra("room_id", room.roomId)
                .setAction("io.github.steeb_k.commune.OPEN_ROOM"),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val name = io.github.steeb_k.commune.ui.roomName(room)
        val countText = if (count == 1uL) "1 new message" else "$count new messages"
        // The latest message, when it is readable: "sender: body". A
        // direct chat's name already names the sender.
        val preview = room.latestEventBody?.let { body ->
            val sender = room.latestEventSender
                ?.substringAfter("@")?.substringBefore(":")
            if (sender != null && !room.isDirect) "$sender: $body" else body
        }

        return Notification.Builder(context, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_notify_symbolic)
            .setContentTitle(name)
            .setContentText(preview ?: countText)
            .setStyle(Notification.BigTextStyle().bigText(preview ?: countText))
            .setSubText(if (preview != null) countText else null)
            .setContentIntent(openApp)
            .setAutoCancel(true)
            .setNumber(count.toInt())
            // Message content stays off the lock screen when the user
            // hides sensitive notifications; the OS shows this instead.
            .setVisibility(Notification.VISIBILITY_PRIVATE)
            .setPublicVersion(redactedNotification(context, CHANNEL_ID))
            .build()
    }

    companion object {
        private const val CHANNEL_ID = "messages"
    }
}

/// The lock-screen stand-in: app name and "New message", nothing else.
internal fun redactedNotification(context: Context, channelId: String): Notification =
    Notification.Builder(context, channelId)
        .setSmallIcon(R.drawable.ic_notify_symbolic)
        .setContentTitle("Commune")
        .setContentText("New message")
        .build()
