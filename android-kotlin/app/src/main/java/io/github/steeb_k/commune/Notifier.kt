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
    private val posted = mutableMapOf<String, ULong>()

    /// Whether notifications are wanted at all: the account setting, and
    /// the room being looked at right now never notifies.
    var enabled: Boolean = true
    var visibleRoomId: String? = null

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
            val count = room.notificationCount
            when {
                count == 0uL || room.roomId == visibleRoomId || !enabled -> {
                    if (posted.remove(room.roomId) != null) {
                        manager.cancel(room.roomId.hashCode())
                    }
                }
                posted[room.roomId] != count -> {
                    posted[room.roomId] = count
                    manager.notify(room.roomId.hashCode(), build(room, count))
                }
            }
        }
    }

    private fun build(room: FfiRoom, count: ULong): Notification {
        val openApp = PendingIntent.getActivity(
            context,
            room.roomId.hashCode(),
            Intent(context, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE,
        )
        val name = io.github.steeb_k.commune.ui.roomName(room)
        val text = if (count == 1uL) "1 new message" else "$count new messages"

        return Notification.Builder(context, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_notify_symbolic)
            .setContentTitle(name)
            .setContentText(text)
            .setContentIntent(openApp)
            .setAutoCancel(true)
            .setNumber(count.toInt())
            .build()
    }

    companion object {
        private const val CHANNEL_ID = "messages"
    }
}
