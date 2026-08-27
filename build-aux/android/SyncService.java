/* Keep Commune running while it is not on screen.
 *
 * Android freezes an application's process the moment nothing of it is
 * visible. For Commune that does not mean messages arrive late, it means they
 * do not arrive at all: the sync loop is a tokio task in this process, and a
 * frozen process runs nothing. A foreground service is the platform's one
 * answer to that which needs no push gateway, no distributor application and
 * no co-operation from the homeserver.
 *
 * This class does nothing but exist. It holds no state and starts no work: the
 * sync loop is already running in the same process, and all a foreground
 * service has to do to keep it running is to be a foreground service. Every
 * decision about when it should exist is on the Rust side, in
 * `src/utils/android_sync_service.rs`.
 *
 * It lives in `org.gtk.android` because pixiewood symlinks exactly one
 * directory into the Gradle project — GTK's own glue, `java-sources` in
 * `pixiewood:715` — and gives an application no way to add a package of its
 * own. `build-aux/android/patch-gtk-service.sh` copies this file in beside the
 * glue after every `pixiewood generate`, the same way the other two patches
 * work. The package is not a claim that this is GTK's code.
 *
 * All the user-visible text arrives as extras rather than as Java resources,
 * because the translations live on the other side: `gettext` runs in Rust,
 * against Commune's own catalogues, and Java has no way to reach them.
 */

package org.gtk.android;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.os.IBinder;

public class SyncService extends Service {
	/* Extras, all filled in by the Rust side. */
	public static final String EXTRA_CHANNEL_NAME = "commune.channel_name";
	public static final String EXTRA_TITLE = "commune.title";
	public static final String EXTRA_TEXT = "commune.text";
	public static final String EXTRA_ICON = "commune.icon";

	/* A channel of its own, separate from the one messages are posted to.
	 * Sharing that one would mean this notification inherited its importance
	 * and its sound, and an ongoing notification that pings is a bug. It is
	 * also what lets someone silence the sync notification in system settings
	 * without silencing their messages. */
	private static final String CHANNEL_ID = "sync";

	private static final int NOTIFICATION_ID = 1;

	@Override
	public IBinder onBind(Intent intent) {
		/* Nothing binds to this; there is nothing to call. */
		return null;
	}

	@Override
	public int onStartCommand(Intent intent, int flags, int startId) {
		NotificationManager manager = getSystemService(NotificationManager.class);

		String channelName = intent != null ? intent.getStringExtra(EXTRA_CHANNEL_NAME) : null;
		if (channelName == null)
			channelName = "Sync";

		NotificationChannel channel = new NotificationChannel(
			CHANNEL_ID, channelName, NotificationManager.IMPORTANCE_LOW);
		manager.createNotificationChannel(channel);

		String title = intent != null ? intent.getStringExtra(EXTRA_TITLE) : null;
		String text = intent != null ? intent.getStringExtra(EXTRA_TEXT) : null;
		int icon = intent != null ? intent.getIntExtra(EXTRA_ICON, 0) : 0;
		if (icon == 0)
			icon = android.R.drawable.stat_notify_sync;

		Intent open = new Intent(this, ToplevelActivity.class);
		PendingIntent contentIntent = PendingIntent.getActivity(
			this, 0, open, PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);

		Notification notification = new Notification.Builder(this, CHANNEL_ID)
			.setContentTitle(title)
			.setContentText(text)
			.setSmallIcon(icon)
			.setContentIntent(contentIntent)
			.setOngoing(true)
			.build();

		startForeground(NOTIFICATION_ID, notification,
			ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC);

		/* Not START_STICKY. If the process is killed anyway, the system would
		 * restart this service with a null Intent, into a process with no GTK
		 * application in it and no Activity to show — a half-resurrected
		 * Commune that syncs nothing and cannot be opened. Staying dead until
		 * someone launches the app again is the honest behaviour. */
		return START_NOT_STICKY;
	}

	/* Android 15 caps a dataSync foreground service at six hours in any
	 * twenty-four, then calls this. Stopping is not optional: a service that
	 * is still in the foreground when the grace period ends is killed with an
	 * ANR. So it stops itself, and Commune goes back to receiving only while
	 * it is on screen until the app is next opened. That cap is the reason
	 * push remains the real answer rather than an optimisation. */
	@Override
	public void onTimeout(int startId, int fgsType) {
		stopSelf();
	}
}
