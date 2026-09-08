// One notification per room in the shade, in the platform's messaging
// shape: the messages that arrived since it was last read, each with its
// sender, the way the AOSP Messages app lays a conversation out when it
// is expanded. Both sources of message notifications — the sidebar's
// counts and the push service — post through here, so a room's messages
// accumulate whichever brought them.
//
// The GTK application sends one plain notification per message with no
// buttons; the conversation shape and the Mark as Read button are this
// platform's own, asked for by the user on 4 September 2026.
package io.github.steeb_k.commune

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Person
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import kotlin.concurrent.thread
import kotlinx.coroutines.runBlocking

internal const val MESSAGES_CHANNEL_ID = "messages"

/// The most messages one room's notification keeps; older ones fall off.
private const val MAX_MESSAGES = 25

/// Posting reads what the shade shows and writes it back with one more
/// message; two sources on their own threads take turns, or one's message
/// would be lost.
private val postLock = Any()

/// Post the room's notification with one more message in it.
///
/// `sender` is who the message is from, as [senderPerson] builds them;
/// null means the room itself speaks — a count with no readable body, an
/// emote that already names its sender. `count` is the server's unread
/// count when known, shown as the badge and in the subtext.
internal fun postRoomMessage(
    context: Context,
    roomId: String,
    roomName: String,
    isDirect: Boolean,
    sender: Person?,
    text: String,
    count: Int? = null,
) = synchronized(postLock) {
    val manager = context.getSystemService(NotificationManager::class.java)
    manager.createNotificationChannel(
        NotificationChannel(
            MESSAGES_CHANNEL_ID,
            "Messages",
            NotificationManager.IMPORTANCE_HIGH,
        )
    )
    val id = roomId.hashCode()

    // What the shade already shows for this room, so the new message
    // joins the earlier ones instead of replacing them. The sidebar's
    // count and the push for the same event both post it: the same words
    // from the same sender as the newest entry are not added twice. The
    // sender is told by key — their Matrix ID — not by name, since the
    // two sources can spell the name differently.
    val previous = manager.activeNotifications
        .firstOrNull { it.id == id }
        ?.notification
        ?.extras
        ?.getParcelableArray(Notification.EXTRA_MESSAGES)
        ?.let { Notification.MessagingStyle.Message.getMessagesFromBundleArray(it) }
        .orEmpty()
    val person = sender ?: Person.Builder().setName(roomName).setKey(roomId).build()
    val newest = previous.lastOrNull()
    val duplicate = newest != null &&
        newest.text?.toString() == text &&
        newest.senderPerson?.identity() == person.identity()
    val messages = if (duplicate) {
        previous
    } else {
        (previous + Notification.MessagingStyle.Message(text, System.currentTimeMillis(), person))
            .takeLast(MAX_MESSAGES)
    }

    val style = Notification.MessagingStyle(Person.Builder().setName("You").build())
        .setGroupConversation(!isDirect)
    if (!isDirect) style.setConversationTitle(roomName)
    for (message in messages) style.addMessage(message)

    val openApp = PendingIntent.getActivity(
        context,
        id,
        Intent(context, MainActivity::class.java)
            .putExtra("room_id", roomId)
            .setAction("io.github.steeb_k.commune.OPEN_ROOM"),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )
    val markRead = PendingIntent.getBroadcast(
        context,
        id,
        Intent(context, MarkReadReceiver::class.java)
            .putExtra("room_id", roomId)
            .setAction(MarkReadReceiver.ACTION),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )
    val countText = when {
        count == null -> null
        count == 1 -> "1 new message"
        else -> "$count new messages"
    }

    val builder = Notification.Builder(context, MESSAGES_CHANNEL_ID)
        .setSmallIcon(R.drawable.ic_notify_symbolic)
        .setContentTitle(roomName)
        .setContentText(if (sender != null && !isDirect) "${sender.name}: $text" else text)
        .setStyle(style)
        .setSubText(countText)
        .setContentIntent(openApp)
        .setAutoCancel(true)
        .setCategory(Notification.CATEGORY_MESSAGE)
        .addAction(
            Notification.Action.Builder(
                android.graphics.drawable.Icon.createWithResource(context, R.drawable.ic_notify_symbolic),
                "Mark as read",
                markRead,
            ).build()
        )
        // Message content stays off the lock screen when the user hides
        // sensitive notifications; the OS shows this instead.
        .setVisibility(Notification.VISIBILITY_PRIVATE)
        .setPublicVersion(redactedNotification(context, MESSAGES_CHANNEL_ID))
    if (count != null) builder.setNumber(count)
    manager.notify(id, builder.build())
}

/// What tells one Person from another: the key when there is one, the
/// name otherwise — the system's own rule for filing messages under a
/// person.
private fun Person.identity(): String = key ?: name?.toString().orEmpty()

/// The Mark as Read button: the room is marked read on the server and its
/// notification taken down, without the app coming to the front.
class MarkReadReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val roomId = intent.getStringExtra("room_id") ?: return
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.cancel(roomId.hashCode())
        val app = (context.applicationContext as CommuneApplication).state.app
        // The request outlives this callback, which the system allows for
        // a short while once told.
        val result = goAsync()
        thread {
            try {
                runBlocking { app.markRoomRead(roomId) }
            } catch (_: Exception) {
                // The next sync will still show the room unread, which is
                // the truth.
            } finally {
                result.finish()
            }
        }
    }

    companion object {
        const val ACTION = "io.github.steeb_k.commune.MARK_READ"
    }
}
