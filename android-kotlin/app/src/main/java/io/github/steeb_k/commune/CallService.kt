// A call outlives the screen it started on: this keeps the process — and
// with it the peer connection and the core's sync — alive for as long as
// somebody is talking, and puts the call in the shade where Android
// expects an ongoing call to be.
package io.github.steeb_k.commune

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.IBinder

class CallService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val who = intent?.getStringExtra(EXTRA_PEER) ?: "Call"
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(
                CHANNEL_ID,
                "Calls",
                NotificationManager.IMPORTANCE_LOW,
            )
        )

        val open = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val notification = Notification.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_notify_symbolic)
            .setContentTitle("Ongoing call")
            .setContentText(who)
            .setContentIntent(open)
            .setOngoing(true)
            .build()

        startForeground(
            NOTIFICATION_ID,
            notification,
            ServiceInfo.FOREGROUND_SERVICE_TYPE_PHONE_CALL,
        )
        return START_STICKY
    }

    companion object {
        private const val CHANNEL_ID = "calls"
        private const val NOTIFICATION_ID = 42

        fun start(context: Context, peer: String) {
            val intent = Intent(context, CallService::class.java).putExtra(EXTRA_PEER, peer)
            context.startForegroundService(intent)
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, CallService::class.java))
        }

        private const val EXTRA_PEER = "peer"
    }
}
