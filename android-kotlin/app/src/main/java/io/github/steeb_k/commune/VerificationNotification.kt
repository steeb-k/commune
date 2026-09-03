// A verification request arriving while the app is not on screen. A
// verification request is a to-device event, so it is not delivered by
// push and only reaches the phone while the app is syncing in the
// background; when it does, and the app is not foreground, this is what
// tells the person, the way the desktop posts one. Tapping it opens the
// app, where the request is already waiting as a sheet.
package io.github.steeb_k.commune

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent

object VerificationNotification {
    private const val CHANNEL_ID = "verification"
    private const val NOTIFICATION_ID = 44

    fun show(context: Context, userId: String?) {
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(
                CHANNEL_ID,
                "Verification requests",
                NotificationManager.IMPORTANCE_HIGH,
            ).apply {
                description = "Requests to verify a session or a contact"
            }
        )

        // A request from ourselves (another of our sessions) has no user;
        // one from someone else names them.
        val title = "Verification Request"
        val text = if (userId != null) {
            val name = userId.removePrefix("@").substringBefore(':')
            "$name wants to verify with you"
        } else {
            "Verify your new session"
        }

        val open = PendingIntent.getActivity(
            context,
            4,
            Intent(context, MainActivity::class.java)
                .setAction(Intent.ACTION_MAIN)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )

        val notification = Notification.Builder(context, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_notify_symbolic)
            .setContentTitle(title)
            .setContentText(text)
            .setContentIntent(open)
            .setAutoCancel(true)
            .build()

        manager.notify(NOTIFICATION_ID, notification)
    }

    fun dismiss(context: Context) {
        context.getSystemService(NotificationManager::class.java)
            .cancel(NOTIFICATION_ID)
    }
}
