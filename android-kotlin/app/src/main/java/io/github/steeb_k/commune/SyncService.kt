// The foreground service that keeps the process — and the Rust core's
// sync loop living inside it — alive while the UI is away. The service
// itself does nothing; being started is its whole job.
package io.github.steeb_k.commune

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.os.IBinder

class SyncService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(
                CHANNEL_ID,
                "Background sync",
                NotificationManager.IMPORTANCE_MIN,
            )
        )

        val openApp = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE,
        )
        val notification = Notification.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_notify_symbolic)
            .setContentTitle("Commune is syncing")
            .setContentIntent(openApp)
            .setOngoing(true)
            .build()

        startForeground(ONGOING_ID, notification)
        return START_STICKY
    }

    companion object {
        private const val CHANNEL_ID = "sync"
        private const val ONGOING_ID = 1

        fun start(context: Context) {
            context.startForegroundService(Intent(context, SyncService::class.java))
        }
    }
}
