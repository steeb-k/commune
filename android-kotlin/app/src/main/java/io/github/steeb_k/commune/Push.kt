// Instant notifications over UnifiedPush — ntfy is the distributor we
// steer people toward — with the foreground SyncService as the fallback
// when no distributor is installed or wanted. A Play Store build slots
// an FCM distributor in here later; nothing else has to change.
//
// The homeserver pushes through the distributor's Matrix gateway (ntfy
// serves one at /_matrix/push/v1/notify), and the payload it delivers is
// the gateway notification itself — enough to post from directly, no
// sync required while the app sleeps.
package io.github.steeb_k.commune

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import org.json.JSONObject
import org.unifiedpush.android.connector.FailedReason
import org.unifiedpush.android.connector.PushService
import org.unifiedpush.android.connector.UnifiedPush
import org.unifiedpush.android.connector.data.PushEndpoint
import org.unifiedpush.android.connector.data.PushMessage

/// How notifications reach the device, and the glue to change it.
object PushManager {
    const val MODE_UNSET = "unset"
    const val MODE_UNIFIEDPUSH = "unifiedpush"
    const val MODE_SYNC = "sync"

    /// Set while the UI is up, so a fresh endpoint reaches the session.
    var onEndpoint: ((String) -> Unit)? = null
    var onFailed: ((String) -> Unit)? = null

    private fun prefs(context: Context) =
        context.getSharedPreferences("push", Context.MODE_PRIVATE)

    fun mode(context: Context): String =
        prefs(context).getString("mode", MODE_UNSET) ?: MODE_UNSET

    fun setMode(context: Context, mode: String) {
        prefs(context).edit().putString("mode", mode).apply()
    }

    fun endpoint(context: Context): String? = prefs(context).getString("endpoint", null)

    fun saveEndpoint(context: Context, endpoint: String?) {
        prefs(context).edit().putString("endpoint", endpoint).apply()
    }

    fun distributors(context: Context): List<String> = UnifiedPush.getDistributors(context)

    /// Register with the first available distributor; the endpoint lands
    /// in [Push.onNewEndpoint].
    fun connect(context: Context) {
        val distributor = distributors(context).firstOrNull() ?: return
        UnifiedPush.saveDistributor(context, distributor)
        UnifiedPush.register(context)
    }

    fun disconnect(context: Context) {
        UnifiedPush.unregister(context)
        saveEndpoint(context, null)
    }
}

/// The UnifiedPush service the distributor delivers to.
class Push : PushService() {
    override fun onNewEndpoint(endpoint: PushEndpoint, instance: String) {
        PushManager.saveEndpoint(this, endpoint.url)
        PushManager.onEndpoint?.invoke(endpoint.url)
    }

    override fun onMessage(message: PushMessage, instance: String) {
        postFromPayload(this, message.content.toString(Charsets.UTF_8))
    }

    override fun onRegistrationFailed(reason: FailedReason, instance: String) {
        PushManager.onFailed?.invoke(reason.name)
    }

    override fun onUnregistered(instance: String) {
        // The distributor is gone; fall back to the foreground sync so
        // messages keep arriving.
        PushManager.saveEndpoint(this, null)
        PushManager.setMode(this, PushManager.MODE_SYNC)
        SyncService.start(this)
    }
}

/// Post a notification straight from the push gateway's payload.
private fun postFromPayload(context: Context, payload: String) {
    val notification = try {
        JSONObject(payload).optJSONObject("notification")
    } catch (_: Exception) {
        null
    }

    val unread = notification?.optJSONObject("counts")?.optInt("unread", -1) ?: -1
    val roomId = notification?.optString("room_id").orEmpty()
    val eventId = notification?.optString("event_id").orEmpty()

    // Unread going to zero clears; everything was read elsewhere.
    if (notification != null && unread == 0) {
        val manager = context.getSystemService(NotificationManager::class.java)
        if (roomId.isNotEmpty()) manager.cancel(roomId.hashCode()) else manager.cancelAll()
        return
    }
    // A push without an event is badge synchronization, not a message —
    // posting it would be the generic notification next to the real one.
    if (roomId.isEmpty() || eventId.isEmpty()) {
        return
    }

    // A call is not a message. The push for an m.call.invite used to post
    // a plain message notification here, seconds before the sync carried
    // the invite itself to the call handler and the phone actually rang:
    // two notifications for one call, the wrong one first. Ringing belongs
    // to the handler, which is the only thing holding the offer needed to
    // answer; this keeps out of its way.
    val eventType = notification?.optString("type").orEmpty()
    if (eventType.startsWith("m.call")) {
        return
    }

    val sender = notification?.optString("sender_display_name")
        ?.takeIf { it.isNotBlank() }
        ?: notification?.optString("sender")?.takeIf { it.isNotBlank() }
    val roomName = notification?.optString("room_name")?.takeIf { it.isNotBlank() }
    val body = notification?.optJSONObject("content")?.optString("body")
        ?.takeIf { it.isNotBlank() }

    val title = roomName ?: sender ?: "Commune"
    val text = when {
        body != null && sender != null && roomName != null -> "$sender: $body"
        body != null -> body
        else -> "New message"
    }

    val manager = context.getSystemService(NotificationManager::class.java)
    manager.createNotificationChannel(
        NotificationChannel(
            "messages",
            "Messages",
            NotificationManager.IMPORTANCE_HIGH,
        )
    )
    val openApp = PendingIntent.getActivity(
        context,
        roomId.hashCode(),
        Intent(context, MainActivity::class.java)
            .putExtra("room_id", roomId)
            .setAction("io.github.steeb_k.commune.OPEN_ROOM"),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )
    manager.notify(
        roomId.hashCode(),
        Notification.Builder(context, "messages")
            .setSmallIcon(R.drawable.ic_notify_symbolic)
            .setContentTitle(title)
            .setContentText(text)
            .setContentIntent(openApp)
            .setAutoCancel(true)
            .setVisibility(Notification.VISIBILITY_PRIVATE)
            .setPublicVersion(redactedNotification(context, "messages"))
            .build(),
    )
}
