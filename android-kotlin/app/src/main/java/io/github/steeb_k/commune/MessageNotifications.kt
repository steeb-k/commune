// One notification per room in the shade, in the platform's messaging
// shape — avatar, sender, group title, the lock-screen stand-in — carrying
// exactly one message: the newest. The rest of what is unread is a count
// in the title above it, "3 more", the way every other messaging
// application keeps the latest words in focus. Both sources of message notifications,
// the sidebar's counts and the push service, post through here.
//
// It used to accumulate: each post read the shade's messages back and added
// one, up to twenty-five, oldest first. Swiping that away without reading
// it left the sidebar's bookkeeping knowing a count but not which messages
// the push service had already shown, so the next sync re-announced the
// swiped message and the next real one landed underneath it. The user asked
// on 15 October 2026 for the newest message alone with the rest counted,
// and for a swiped message never to come back. [AnnouncedMessages] is the
// one record both sources consult, keyed by event, which is what makes the
// second half true.
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
import android.os.Bundle
import kotlin.concurrent.thread
import kotlinx.coroutines.runBlocking

internal const val MESSAGES_CHANNEL_ID = "messages"

/// Where a room's notification keeps how many unread messages it says
/// there are besides the one it shows. A push does not know the room's
/// count, so it reads this back and adds one; the next sync replaces it
/// with the server's figure.
private const val EXTRA_MORE = "io.github.steeb_k.commune.MORE"

/// Set on a notification whose line is the count itself ("3 new
/// messages"): saying "2 more messages" above that would count the same
/// messages twice.
private const val EXTRA_SUMMARY = "io.github.steeb_k.commune.SUMMARY"

/// Posting reads what the shade shows — the room's "more" figure and its
/// badge — and writes it back changed; two sources on their own threads
/// take turns, or one would overwrite the other's figure.
private val postLock = Any()

/// Which messages have been announced, per account and room, shared by the
/// sidebar's counts and the push service so that neither raises what the
/// other already has.
///
/// A message is announced when its event ID has not been seen and it is not
/// well older than the newest message announced for the room. The second
/// half drops what arrives stale — a room list restored from the store
/// still carrying a latest message from before the shade moved on — since
/// the badge already counts it and the newest message is what should be in
/// focus. "Well older" is [SKEW_MS]: the timestamp is the origin server's,
/// and in a federated room two servers stamp within a few seconds of each
/// other, so a strict order would count a message from the slower server in
/// the badge and never show it. Within the allowance the event ID alone
/// decides, which is how Element X and Signal decide at all times.
///
/// Everything posted was recorded before it was posted, so a swipe needs no
/// bookkeeping of its own: what the user dismissed is already in here and
/// cannot be raised again. The record is dropped when the room's count
/// reaches zero, which is the server saying it has been read.
///
/// The account is the session's user ID, asked for by the caller at the
/// moment of use rather than set on a field: the field it replaces was only
/// set when the account switcher opened, so the record lived under two keys
/// and every switch re-announced every unread room.
internal object AnnouncedMessages {
    /// How many event IDs a room keeps. Only the newest few can still come
    /// round again; anything older is caught by the timestamp.
    private const val MAX_IDS = 50

    /// How far behind the newest announced message a message may be stamped
    /// and still count as news: the clock skew allowed between homeservers,
    /// and between the phone and the server when the push fallback stamps
    /// with the phone's clock. A stale preview is minutes or hours behind,
    /// never this close.
    private const val SKEW_MS = 60_000L

    /// Deciding and recording are one step; the two sources are on
    /// different threads.
    private val lock = Any()

    /// The file the per-count bookkeeping before this used. Its keys have
    /// no field suffix, so they cannot collide with these and are left to
    /// lie there harmlessly.
    private fun prefs(context: Context) =
        context.getSharedPreferences("notified", Context.MODE_PRIVATE)

    private fun key(account: String, roomId: String, field: String) = "$account|$roomId|$field"

    /// Whether the message is news, recording it as announced when it is.
    /// The caller posts it if and only if this says yes.
    fun claim(
        context: Context,
        account: String,
        roomId: String,
        eventId: String,
        timestamp: Long,
    ): Boolean = synchronized(lock) {
        val prefs = prefs(context)
        val newest = prefs.getLong(key(account, roomId, "ts"), 0L)
        val ids = prefs.getString(key(account, roomId, "ids"), null)
            ?.split('\n')
            ?.filter { it.isNotEmpty() }
            .orEmpty()
        if (eventId in ids || timestamp < newest - SKEW_MS) return false
        prefs.edit()
            .putLong(key(account, roomId, "ts"), maxOf(newest, timestamp))
            .putString(key(account, roomId, "ids"), (ids + eventId).takeLast(MAX_IDS).joinToString("\n"))
            .apply()
        true
    }

    /// The server count last announced for the room. Only the generic line
    /// — a count with no readable message to show — goes by this.
    fun known(context: Context, account: String, roomId: String): Long =
        prefs(context).getLong(key(account, roomId, "known"), 0L)

    fun setKnown(context: Context, account: String, roomId: String, count: Long) {
        prefs(context).edit().putLong(key(account, roomId, "known"), count).apply()
    }

    /// Drop everything recorded for the room, saying whether there was
    /// anything.
    fun forget(context: Context, account: String, roomId: String): Boolean = synchronized(lock) {
        val prefs = prefs(context)
        val fields = listOf("ts", "ids", "known").map { key(account, roomId, it) }
        if (fields.none { prefs.contains(it) }) return false
        val editor = prefs.edit()
        for (field in fields) editor.remove(field)
        editor.apply()
        true
    }
}

/// Post the room's notification showing this one message.
///
/// Whether the message is news is [AnnouncedMessages]' to say, and the
/// caller asks before it gets here. `sender` is who the message is from, as
/// [senderPerson] builds them; null means the room itself speaks — a count
/// with no readable body, an emote that already names its sender.
///
/// `count` is the server's unread count when known: the badge, and all but
/// one of it is the "more" above the message. The push service does not
/// know it, so with null the figure is the one already in the shade plus
/// this message, or nothing when the shade holds nothing for the room.
/// `summary` says the text is itself the count, which then has no "more".
internal fun postRoomMessage(
    context: Context,
    roomId: String,
    roomName: String,
    isDirect: Boolean,
    sender: Person?,
    text: String,
    count: Int?,
    timestamp: Long = System.currentTimeMillis(),
    summary: Boolean = false,
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

    val active = manager.activeNotifications.firstOrNull { it.id == id }?.notification
    val more = when {
        summary -> 0
        count != null -> (count - 1).coerceAtLeast(0)
        active != null -> active.extras.getInt(EXTRA_MORE) + 1
        else -> 0
    }
    // Without the server's count the badge stays as the shade had it.
    val badge = count ?: active?.number?.takeIf { it > 0 }

    // The conversation shape, unless the line is the count itself: a count
    // "from" the room read as "Invite Room: Invite Room: 5 new messages" in
    // that shape, and a plain title and line say it once.
    val style = if (summary) {
        null
    } else {
        val person = sender ?: Person.Builder().setName(roomName).setKey(roomId).build()
        Notification.MessagingStyle(Person.Builder().setName("You").build())
            .setGroupConversation(!isDirect)
            .addMessage(Notification.MessagingStyle.Message(text, timestamp, person))
            .setConversationTitle(conversationTitle(roomName, isDirect, more))
    }

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

    val builder = Notification.Builder(context, MESSAGES_CHANNEL_ID)
        .setSmallIcon(R.drawable.ic_notify_symbolic)
        .setContentTitle(roomName)
        .setContentText(if (sender != null && !isDirect) "${sender.name}: $text" else text)
        .setStyle(style)
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
        .addExtras(
            Bundle().apply {
                putInt(EXTRA_MORE, more)
                putBoolean(EXTRA_SUMMARY, summary)
            }
        )
        // Message content stays off the lock screen when the user hides
        // sensitive notifications; the OS shows this instead.
        .setVisibility(Notification.VISIBILITY_PRIVATE)
        .setPublicVersion(redactedNotification(context, MESSAGES_CHANNEL_ID))
    if (badge != null) builder.setNumber(badge)
    manager.notify(id, builder.build())
}

/// Bring the room's notification, if the shade still holds it, in line with
/// the server's count without raising anything: the same message, a new
/// "more" and badge, and no sound. A notification the user swiped away
/// stays gone — that is the whole of the complaint this answers.
internal fun refreshRoomCount(context: Context, roomId: String, count: Int) = synchronized(postLock) {
    val manager = context.getSystemService(NotificationManager::class.java)
    val id = roomId.hashCode()
    val active = manager.activeNotifications.firstOrNull { it.id == id }?.notification
        ?: return@synchronized
    val more = if (active.extras.getBoolean(EXTRA_SUMMARY)) 0 else (count - 1).coerceAtLeast(0)
    if (active.number == count && active.extras.getInt(EXTRA_MORE) == more) return@synchronized
    val builder = Notification.Builder.recoverBuilder(context, active)
        .setNumber(count)
        .setOnlyAlertOnce(true)
        .addExtras(Bundle().apply { putInt(EXTRA_MORE, more) })
    // The recovered builder carries the style it was built with; the title
    // is the only part of it the count changes.
    (builder.style as? Notification.MessagingStyle)?.let { style ->
        val roomName = active.extras.getCharSequence(Notification.EXTRA_TITLE)?.toString().orEmpty()
        style.setConversationTitle(conversationTitle(roomName, !style.isGroupConversation, more))
    }
    manager.notify(id, builder.build())
}

/// What the shade draws above the message: the room's name in a group
/// room, "weekend plans · 3 more" when more is unread than the message
/// shown; in a direct chat nothing — the platform names the sender on its
/// own — or just "3 more". The collapsed line of a bundled notification is
/// the platform's "title: sender text", and with the sender's name in the
/// title as well it read "bob · 2 more: bob r2 m3" on the emulator.
///
/// The subtext would be the natural place for the count, and it was the
/// first place tried; but once Android bundles two of this application's
/// notifications into a group, which two rooms notifying is enough for, a
/// child's header shows only its time and the subtext is gone. The
/// conversation title is drawn in every state the shade has.
private fun conversationTitle(roomName: String, isDirect: Boolean, more: Int): String? = when {
    more > 0 && isDirect -> "$more more"
    more > 0 -> "$roomName · $more more"
    isDirect -> null
    else -> roomName
}

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
