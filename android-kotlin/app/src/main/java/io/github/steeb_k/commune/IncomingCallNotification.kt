// A call arriving while the app is not on screen: Android's own
// incoming-call treatment — a full-screen intent that becomes a
// heads-up banner when the phone is in use, with answer and decline on
// it. Without this, a backgrounded app rings into the void.
package io.github.steeb_k.commune

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent

object IncomingCallNotification {
    private const val CHANNEL_ID = "incoming-calls"
    private const val NOTIFICATION_ID = 43

    const val ACTION_ANSWER = "io.github.steeb_k.commune.ANSWER_CALL"
    const val ACTION_DECLINE = "io.github.steeb_k.commune.DECLINE_CALL"

    fun show(context: Context, caller: String) {
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(
                CHANNEL_ID,
                "Incoming calls",
                NotificationManager.IMPORTANCE_HIGH,
            ).apply {
                description = "Calls ringing now"
                setBypassDnd(true)
            }
        )

        val name = caller.removePrefix("@").substringBefore(':')
        val open = PendingIntent.getActivity(
            context,
            1,
            Intent(context, MainActivity::class.java)
                .setAction(Intent.ACTION_MAIN)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val answer = PendingIntent.getActivity(
            context,
            2,
            Intent(context, MainActivity::class.java)
                .setAction(ACTION_ANSWER)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val decline = PendingIntent.getActivity(
            context,
            3,
            Intent(context, MainActivity::class.java)
                .setAction(ACTION_DECLINE)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )

        val builder = Notification.Builder(context, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_notify_symbolic)
            .setContentTitle("Incoming call")
            .setContentText(name)
            .setCategory(Notification.CATEGORY_CALL)
            .setOngoing(true)
            .setContentIntent(open)
            // The whole screen when the phone is idle, a banner when it
            // is not — which is what a ringing phone does.
            .setFullScreenIntent(open, true)

        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.S) {
            // A plain high-importance notification gets swept into
            // Android's automatic group once the app has a few of them,
            // and a grouped notification does not pop up — which is how a
            // ringing phone came to show a silent line in the shade
            // instead. CallStyle is ranked as a call: never auto-grouped,
            // always at the top, and drawn with the answer and decline
            // buttons the system uses for every other call.
            val caller = android.app.Person.Builder()
                .setName(name)
                .setImportant(true)
                .build()
            builder.setStyle(
                Notification.CallStyle.forIncomingCall(caller, decline, answer)
            )
        } else {
            builder
                .addAction(
                    Notification.Action.Builder(null, "Decline", decline).build()
                )
                .addAction(
                    Notification.Action.Builder(null, "Answer", answer).build()
                )
        }

        manager.notify(NOTIFICATION_ID, builder.build())
    }

    fun dismiss(context: Context) {
        context.getSystemService(NotificationManager::class.java).cancel(NOTIFICATION_ID)
    }
}
