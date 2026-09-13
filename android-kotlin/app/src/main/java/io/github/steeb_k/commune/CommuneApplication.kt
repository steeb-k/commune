// The session belongs to the process, not to a window.
//
// It used to be built in MainActivity.onCreate, which made the whole
// session an accessory of whatever was on screen. Two things followed from
// that. A rotation destroyed the activity, built a fresh empty state, and
// dropped a call in progress — the call itself carried on inside
// CallService with nothing left able to reach it. And an incoming call
// could only ever ring while an activity happened to exist, because the
// listener that shows the incoming-call notification is installed by
// CommuneState; a push that woke the process found nobody listening for
// m.call.invite and posted a plain message notification instead.
//
// Building it here means the core starts restoring, and the call handlers
// install, as soon as the process exists — however it came to exist.
package io.github.steeb_k.commune

import android.app.Application
import android.util.Log
import io.github.steeb_k.commune.core.setUpdateFeed
import java.io.File

private const val TAG = "CommuneApplication"

class CommuneApplication : Application() {
    lateinit var state: CommuneState
        private set

    override fun onCreate() {
        super.onCreate()
        applyUpdateFeedOverride()
        // Cheap: CommuneState's constructor seeds the core and hands the
        // session restore to a background thread of its own.
        state = CommuneState(this)
    }

    /// A release build has no way to receive an env var, so testing the
    /// updater against a local feed (see doc/updates.md) instead drops a
    /// file: one line naming the feed base URL, at
    /// `getExternalFilesDir(null)/update-feed`, which `adb push` can reach on
    /// an emulator or a device. Must run before anything touches the core.
    /// Nobody has this file in production.
    ///
    /// Goes through `setUpdateFeed()`, an in-process override the core
    /// consults before the environment, rather than `Os.setenv`: an
    /// `arm64`-only release APK on an x86_64 emulator runs under ARM
    /// translation, and that writes the *guest* libc's environment, which
    /// `reqwest` on the host side of the translation never sees.
    private fun applyUpdateFeedOverride() {
        val dir = getExternalFilesDir(null) ?: return
        val file = File(dir, "update-feed")
        if (!file.exists()) return

        val feed = file.readLines().firstOrNull()?.trim()
        if (feed.isNullOrEmpty()) return

        setUpdateFeed(feed)
        Log.i(TAG, "Update feed overridden by $file: $feed")
    }
}
