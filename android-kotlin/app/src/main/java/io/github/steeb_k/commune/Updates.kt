// Keeping this installation current.
//
// The core does the same work here as on the desktop — read the signed
// release manifest, decide whether what it offers is newer, fetch the
// artifact and check its digest — and this is the part Android owns: asking
// at a sensible moment, and handing the APK to the system package installer.
//
// Installing is deliberately not something this code does. Android will only
// replace an installed application with one signed by the same key, and it is
// the system that enforces that, not us. The updater's whole security model
// rests on it, which is why the release keystore in
// ../../../../../../keystore.properties is the one irreplaceable file in the
// project: an APK signed by anything else cannot upgrade what is installed.
package io.github.steeb_k.commune

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.Settings
import android.util.Log
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.core.content.FileProvider
import io.github.steeb_k.commune.core.CoreException
import io.github.steeb_k.commune.core.FfiRelease
import io.github.steeb_k.commune.core.FfiUpdateSettings
import io.github.steeb_k.commune.core.UpdateProgressListener
import io.github.steeb_k.commune.core.appBuildNumber
import io.github.steeb_k.commune.core.appVersion
import io.github.steeb_k.commune.core.checkForUpdate
import io.github.steeb_k.commune.core.downloadUpdate
import io.github.steeb_k.commune.core.setUpdateChannel
import io.github.steeb_k.commune.core.setUpdateCheckAutomatically
import io.github.steeb_k.commune.core.setUpdateSkippedVersion
import io.github.steeb_k.commune.core.updateSettings
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private const val TAG = "CommuneUpdates"

/// What the updater is doing, and what it last found.
enum class UpdateState {
    /// Nothing has been asked yet this run.
    Idle,

    /// A check is in flight.
    Checking,

    /// The feed was read and this build is current.
    UpToDate,

    /// There is a newer release to install.
    Available,

    /// The APK is being fetched.
    Downloading,

    /// The package installer has been handed the APK.
    Installing,

    /// The last attempt did not finish.
    Failed,
}

/// The state of this installation with respect to the release feed.
///
/// One of these, like the desktop's `Updates` object, because there is one
/// installation: the settings rows observe it rather than each doing their
/// own check.
object Updates {
    /// What the updater is doing.
    var state by mutableStateOf(UpdateState.Idle)
        private set

    /// A sentence describing [state], for a row's subtitle.
    var status by mutableStateOf("")
        private set

    /// How much of the download has arrived, from 0 to 1.
    var progress by mutableStateOf(0f)
        private set

    /// The version of the release that is available, when one is.
    var availableVersion by mutableStateOf<String?>(null)
        private set

    /// Where a person can read what the available release changed.
    var releaseNotesUrl by mutableStateOf<String?>(null)
        private set

    /// The settings as they were last read.
    var settings by mutableStateOf<FfiUpdateSettings?>(null)
        private set

    /// The version of a release an automatic check found and has not yet
    /// been shown for outside Settings — a Snackbar's cue, cleared by
    /// [dismissAnnouncement] once it has been shown. `null` most of the
    /// time: only a check nobody asked for, that found a release nobody
    /// skipped, sets this.
    var announcement by mutableStateOf<String?>(null)
        private set

    /// The version this build reports as its own.
    val version: String by lazy { appVersion() }

    /// The build number this build reports as its own — the number the
    /// updater orders two builds of one version by.
    val buildNumber: ULong by lazy { appBuildNumber() }

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private var release: FfiRelease? = null

    /// Whether [start] has already run once.
    ///
    /// `start()` is called from the main activity as soon as the core is up
    /// so that an installation nobody opens Settings on still checks, and
    /// `UpdateRows()` also calls it (so Settings works even if that call
    /// somehow never ran) — this makes the second call a no-op instead of a
    /// second check racing the first.
    private var started = false

    /// Read the settings, and check if one is due.
    ///
    /// Called once the core is up. An automatic check that cannot reach the
    /// feed says nothing: a phone with no signal is not a problem the user
    /// needs to be told about. Safe to call more than once; only the first
    /// call does anything.
    fun start() {
        if (started) return
        started = true

        scope.launch {
            val current = withContext(Dispatchers.IO) { updateSettings() }
            settings = current

            if (current.checkAutomatically && current.checkDue) {
                check(userInitiated = false)
            }
        }
    }

    /// Ask the feed what the current release is.
    fun check(userInitiated: Boolean) {
        if (state == UpdateState.Checking ||
            state == UpdateState.Downloading ||
            state == UpdateState.Installing
        ) {
            return
        }

        state = UpdateState.Checking
        status = "Checking…"

        scope.launch {
            try {
                val result = checkForUpdate()
                val current = withContext(Dispatchers.IO) { updateSettings() }
                settings = current

                val available = result.available
                // A release the user asked not to hear about again is not
                // announced by a check nobody asked for — but a press of
                // "Check Now" always gets an answer, skipped or not, which
                // is what makes the skip reversible rather than permanent.
                val isSkipped = !userInitiated &&
                    available != null &&
                    current.skippedVersion == available.version

                if (available?.asset == null || isSkipped) {
                    release = null
                    availableVersion = null
                    releaseNotesUrl = null
                    state = UpdateState.UpToDate
                    status = "Commune is up to date."
                } else {
                    release = available
                    availableVersion = available.version
                    releaseNotesUrl = available.notesUrl
                    state = UpdateState.Available
                    status = "Commune ${available.version} is available."

                    if (!userInitiated) {
                        announcement = available.version
                    }
                }
            } catch (error: Exception) {
                Log.w(TAG, "Could not check for updates", error)

                if (userInitiated) {
                    state = UpdateState.Failed
                    // `error.message` is not this: a `CoreException` other
                    // than `Failed` carries whatever `.message` a bindings
                    // internal-error class defaults to, and `Failed.message`
                    // itself is a debug rendering ("msg=...") rather than the
                    // core's user-facing sentence, which lives in `.msg`.
                    status = (error as? CoreException.Failed)?.msg
                        ?: "Could not check for updates."
                } else {
                    state = UpdateState.Idle
                    status = ""
                }
            }
        }
    }

    /// Stop offering the release the last check found, and put the row back
    /// to up to date — the same thing a check that found this version
    /// skipped would show.
    fun skip() {
        val version = release?.version ?: return

        scope.launch {
            withContext(Dispatchers.IO) { setUpdateSkippedVersion(version) }
            settings = withContext(Dispatchers.IO) { updateSettings() }
            release = null
            availableVersion = null
            releaseNotesUrl = null
            state = UpdateState.UpToDate
            status = "Commune is up to date."
        }
    }

    /// The last automatic-check announcement has been shown; do not show it
    /// again.
    fun dismissAnnouncement() {
        announcement = null
    }

    /// Turn automatic checks on or off.
    fun setCheckAutomatically(enabled: Boolean) {
        scope.launch {
            withContext(Dispatchers.IO) { setUpdateCheckAutomatically(enabled) }
            settings = withContext(Dispatchers.IO) { updateSettings() }
        }
    }

    /// Follow the named channel, and look at it straight away.
    fun setChannel(channel: String) {
        scope.launch {
            withContext(Dispatchers.IO) { setUpdateChannel(channel) }
            settings = withContext(Dispatchers.IO) { updateSettings() }
            check(userInitiated = true)
        }
    }

    /// Download the available release and offer it to the package installer.
    ///
    /// The user has to have allowed this application to install packages.
    /// There is no way to grant that from here, so the first time through
    /// this opens the settings page that can and stops; the next press gets
    /// as far as the installer.
    fun install(activity: Activity) {
        val asset = release?.asset ?: return

        if (state == UpdateState.Downloading || state == UpdateState.Installing) {
            return
        }

        if (!activity.packageManager.canRequestPackageInstalls()) {
            status = "Allow Commune to install apps, then press Update again."
            activity.startActivity(
                Intent(
                    Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                    Uri.parse("package:${activity.packageName}"),
                ),
            )
            return
        }

        progress = 0f
        state = UpdateState.Downloading
        status = "Downloading…"

        scope.launch {
            try {
                val directory = File(activity.cacheDir, "updates")
                val listener = object : UpdateProgressListener {
                    override fun progress(downloaded: ULong, total: ULong) {
                        val fraction =
                            if (total == 0UL) 0f else downloaded.toFloat() / total.toFloat()

                        // Called from the download's own thread, often; the
                        // state it writes belongs to the main one.
                        scope.launch {
                            progress = fraction
                            status = "Downloading… ${(fraction * 100).toInt()}%"
                        }
                    }
                }

                val path = downloadUpdate(asset, directory.absolutePath, listener)

                state = UpdateState.Installing
                status = "Opening the installer…"
                offerToInstaller(activity, File(path))
            } catch (error: Exception) {
                Log.w(TAG, "Could not download the update", error)
                state = UpdateState.Failed
                status = (error as? CoreException.Failed)?.msg
                    ?: "Could not download the update."
            }
        }
    }

    /// The activity that handed an APK to the package installer has been
    /// resumed.
    ///
    /// `install()` starts the installer with `startActivity` and never
    /// learns what happened to it: the sheet can be cancelled, or opened and
    /// closed without a dialog, and neither tells this object anything.
    /// Before this existed, the row was then stuck on "Opening the
    /// installer…" with both buttons dead (`check()` returns early while
    /// `state == Installing`) until the process was killed and restarted. An
    /// app that is resumed while it still thinks it is being replaced was not
    /// replaced, so the release stays available — it is still held in
    /// [release] — and the row goes back to announcing it rather than
    /// staying stuck.
    fun installerReturned() {
        if (state != UpdateState.Installing) return

        val version = release?.version ?: run {
            state = UpdateState.Idle
            status = ""
            return
        }

        state = UpdateState.Available
        status = "Commune $version is available."
    }

    /// Hand the downloaded APK to the system package installer.
    private fun offerToInstaller(context: Context, apk: File) {
        val uri: Uri =
            FileProvider.getUriForFile(context, "${context.packageName}.updates", apk)

        context.startActivity(
            Intent(Intent.ACTION_VIEW).apply {
                setDataAndType(uri, "application/vnd.android.package-archive")
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            },
        )
    }
}
