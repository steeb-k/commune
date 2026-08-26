/* Receive UnifiedPush broadcasts from the distributor.
 *
 * UnifiedPush (spec AND_3.1.0) is a set of broadcasts: Commune asks a
 * distributor application to hold the one push connection, and the distributor
 * answers here — an endpoint on registration, and later the raw bytes of every
 * push. This class forwards all of it to the Rust side and decides nothing
 * itself: which broadcasts are trusted, what a message means and what to do
 * about an endpoint are all questions for `src/utils/android_push.rs`.
 *
 * The forwarding is a plain native method rather than `RegisterNatives`,
 * which needs the class found through the application class loader — the
 * dance `android_sync_service.rs` documents avoiding. GTK's glue loaded the
 * application library with `g_module_open`, which the JVM's native-method
 * lookup does not see, so `ensureLoaded` loads the same library once more by
 * name; dlopen reference-counts, so the second load is bookkeeping, not a
 * second copy. The symbol itself is kept alive by `build-aux/android/stub.c`,
 * because Meson links the Rust staticlib as a plain archive and an object
 * nothing references would be dropped.
 *
 * By the time any broadcast arrives the Rust side is up:
 * `RuntimeApplication.onCreate` runs `main` on the GTK thread and blocks until
 * the GLib event loop is live, on every process start — a broadcast-only one
 * included. See `doc/android-push-plan.md`.
 *
 * The receiver is exported, because the distributor is another application.
 * Anything on the device can therefore feed it lies; the token is what makes
 * that harmless, and checking it happens on the Rust side.
 *
 * It lives in `org.gtk.android` for the same reason `SyncService` does:
 * pixiewood symlinks exactly one Java directory into the Gradle project, and
 * `build-aux/android/patch-gtk-receiver.sh` copies this file in beside the
 * glue. The package is not a claim that this is GTK's code.
 */

package org.gtk.android;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.pm.ApplicationInfo;
import android.content.pm.PackageManager;

public class PushReceiver extends BroadcastReceiver {
	private static boolean loaded = false;

	/* Make the application library visible to the JVM's native-method
	 * lookup. The name comes from the same manifest metadata
	 * `RuntimeApplication` starts the runtime from. */
	private static synchronized void ensureLoaded(Context context) {
		if (loaded)
			return;

		try {
			ApplicationInfo info = context.getPackageManager().getApplicationInfo(
				context.getPackageName(),
				PackageManager.GET_META_DATA
			);
			System.loadLibrary(info.metaData.getString(RuntimeApplication.LIBNAME_KEY));
			loaded = true;
		} catch (Exception err) {
			throw new RuntimeException("Unable to load the application library", err);
		}
	}

	@Override
	public void onReceive(Context context, Intent intent) {
		String action = intent.getAction();
		if (action == null)
			return;

		ensureLoaded(context);

		/* Every extra the AND_3 connector actions carry, each null when
		 * absent. Which of them mean anything depends on the action, and
		 * sorting that out is the Rust side's job. */
		nativeReceive(
			context,
			action,
			intent.getStringExtra("token"),
			intent.getStringExtra("endpoint"),
			intent.getStringExtra("reason"),
			intent.getStringExtra("useDistributor"),
			intent.getStringExtra("id"),
			intent.getByteArrayExtra("bytesMessage"));
	}

	private static native void nativeReceive(
		Context context,
		String action,
		String token,
		String endpoint,
		String reason,
		String useDistributor,
		String id,
		byte[] message);
}
