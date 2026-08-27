/* Let the distributor hold Commune out of the freezer while a push lands.
 *
 * Android freezes a cached process about ten seconds after a broadcast starts
 * it — measured, see `doc/android.md` S5b — which is not always enough to
 * fetch the pushed event, and a request in flight when the freezer strikes
 * dies with its socket. The UnifiedPush spec's answer is this service: a
 * distributor may bind it around a message delivery, and a process another
 * application's foreground service is bound to is important enough that the
 * freezer leaves it alone. ntfy already looks for it on every delivery and
 * logged `targetHasService=false` until this existed.
 *
 * There is nothing to bind *to*: the elevation is the point, so the binder is
 * as empty as an object can be. It is exported because the distributor is
 * another application; anything else on the device binding it gains nothing
 * and costs a few seconds of not being frozen.
 *
 * It lives in `org.gtk.android` for the same reason `SyncService` and
 * `PushReceiver` do: pixiewood symlinks exactly one Java directory into the
 * Gradle project, and `build-aux/android/patch-gtk-receiver.sh` copies this
 * file in beside the glue.
 */

package org.gtk.android;

import android.app.Service;
import android.content.Intent;
import android.os.Binder;
import android.os.IBinder;

public class PushRaiseService extends Service {
	@Override
	public IBinder onBind(Intent intent) {
		return new Binder();
	}
}
