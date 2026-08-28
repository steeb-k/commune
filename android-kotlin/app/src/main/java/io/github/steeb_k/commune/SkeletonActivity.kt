// The walking skeleton, one step further: the Rust core logs into a
// homeserver (the local test harness by default), syncs, and this activity
// renders the core's room list live — the same rooms, categories and unread
// state the GTK sidebar shows. Still throwaway: the real UI is Jetpack
// Compose (chunk 8 of doc/kotlin-plan.md).
package io.github.steeb_k.commune

import android.app.Activity
import android.os.Bundle
import android.view.Gravity
import android.widget.ArrayAdapter
import android.widget.LinearLayout
import android.widget.ListView
import android.widget.TextView
import io.github.steeb_k.commune.core.CoreApp
import io.github.steeb_k.commune.core.FfiCoreConfig
import io.github.steeb_k.commune.core.FfiRoom
import io.github.steeb_k.commune.core.FfiRoomCategory
import io.github.steeb_k.commune.core.FfiRoomDisplayName
import io.github.steeb_k.commune.core.RoomListListener
import io.github.steeb_k.commune.core.Native
import io.github.steeb_k.commune.core.coreVersion
import io.github.steeb_k.commune.core.initCore
import kotlin.concurrent.thread
import kotlinx.coroutines.runBlocking

/// The homeserver of `testing/local-homeserver.sh`, reached through
/// `adb reverse tcp:8008 tcp:8008`.
private const val TEST_HOMESERVER = "http://localhost:8008"
private const val TEST_USER = "alice"
private const val TEST_PASSWORD = "alice-is-testing"

/// The sidebar's section order, as the GTK app shows it.
private val CATEGORY_ORDER = listOf(
    FfiRoomCategory.KNOCKED to "Invite Requests",
    FfiRoomCategory.INVITED to "Invited",
    FfiRoomCategory.SERVER_NOTICE to "Server Notices",
    FfiRoomCategory.SPACE to "Spaces",
    FfiRoomCategory.FAVORITE to "Favorites",
    FfiRoomCategory.NORMAL to "Rooms",
    FfiRoomCategory.LOW_PRIORITY to "Low Priority",
    FfiRoomCategory.LEFT to "Historical",
)

class SkeletonActivity : Activity() {
    private lateinit var status: TextView
    private lateinit var list: ListView
    private lateinit var adapter: ArrayAdapter<String>
    private lateinit var app: CoreApp

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        status = TextView(this)
        status.textSize = 16f
        status.gravity = Gravity.CENTER
        status.setPadding(24, 24, 24, 24)
        status.text = "commune-core ${coreVersion()} — starting…"

        adapter = ArrayAdapter(this, android.R.layout.simple_list_item_1, mutableListOf())
        list = ListView(this)
        list.adapter = adapter

        val column = LinearLayout(this)
        column.orientation = LinearLayout.VERTICAL
        // API 35 draws edge-to-edge by default; keep the skeleton's content
        // out from under the bars.
        column.fitsSystemWindows = true
        column.addView(
            status,
            LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                LinearLayout.LayoutParams.WRAP_CONTENT,
            ),
        )
        column.addView(
            list,
            LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f),
        )
        setContentView(column)

        Native.seed(applicationContext)
        initCore(
            FfiCoreConfig(
                appId = "io.github.steeb_k.commune.skeleton",
                profile = "skeleton",
                dataDir = noBackupFilesDir.resolve("commune").absolutePath,
                cacheDir = cacheDir.resolve("commune").absolutePath,
            )
        )
        app = CoreApp()

        app.setRoomListListener(object : RoomListListener {
            override fun onUpdate(rooms: List<FfiRoom>) {
                runOnUiThread { render(rooms) }
            }
        })

        thread {
            runBlocking {
                setStatus("Restoring sessions…")
                app.restoreSessions()

                if (!app.hasSessions()) {
                    setStatus("Logging in as $TEST_USER…")
                    try {
                        app.loginWithPassword(TEST_HOMESERVER, TEST_USER, TEST_PASSWORD)
                    } catch (failure: Exception) {
                        setStatus("Login failed: ${failure.message}")
                        return@runBlocking
                    }
                }

                setStatus("Synchronizing…")
            }
        }
    }

    private fun setStatus(text: String) {
        runOnUiThread { status.text = text }
    }

    private fun render(rooms: List<FfiRoom>) {
        status.text = "commune-core ${coreVersion()} — ${rooms.size} rooms"

        val lines = mutableListOf<String>()
        for ((category, title) in CATEGORY_ORDER) {
            val section = rooms
                .filter { it.category == category }
                .sortedByDescending { it.latestActivity }
            if (section.isEmpty()) continue

            lines.add("— $title —")
            for (room in section) {
                val name = when (val displayName = room.displayName) {
                    is FfiRoomDisplayName.Named -> displayName.name
                    is FfiRoomDisplayName.EmptyWas -> "Empty Room (was ${displayName.user})"
                    is FfiRoomDisplayName.Empty -> "Empty Room"
                    is FfiRoomDisplayName.Unknown -> "Unknown"
                }
                val badge = if (room.notificationCount > 0uL) {
                    "  (${room.notificationCount})"
                } else if (!room.isRead) {
                    "  •"
                } else {
                    ""
                }
                val direct = if (room.isDirect) "@ " else ""
                lines.add("    $direct$name$badge")
            }
        }

        adapter.clear()
        adapter.addAll(lines)
        adapter.notifyDataSetChanged()
    }
}
