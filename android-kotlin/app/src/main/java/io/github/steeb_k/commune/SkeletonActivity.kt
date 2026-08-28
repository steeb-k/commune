// The walking skeleton, now a chat: the Rust core logs into a homeserver
// (the local test harness by default), syncs, and this activity renders the
// core's room list and — one tap deeper — a room's live timeline with a
// working composer. Still throwaway: the real UI is Jetpack Compose
// (chunk 8 of doc/kotlin-plan.md).
package io.github.steeb_k.commune

import android.app.Activity
import android.os.Bundle
import android.view.Gravity
import android.widget.ArrayAdapter
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ListView
import android.widget.TextView
import io.github.steeb_k.commune.core.CoreApp
import io.github.steeb_k.commune.core.FfiCoreConfig
import io.github.steeb_k.commune.core.FfiEventKind
import io.github.steeb_k.commune.core.FfiRoom
import io.github.steeb_k.commune.core.FfiRoomCategory
import io.github.steeb_k.commune.core.FfiRoomDisplayName
import io.github.steeb_k.commune.core.FfiTimelineItem
import io.github.steeb_k.commune.core.RoomListListener
import io.github.steeb_k.commune.core.TimelineListener
import io.github.steeb_k.commune.core.Native
import io.github.steeb_k.commune.core.coreVersion
import io.github.steeb_k.commune.core.initCore
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
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

private fun roomName(room: FfiRoom): String = when (val displayName = room.displayName) {
    is FfiRoomDisplayName.Named -> displayName.name
    is FfiRoomDisplayName.EmptyWas -> "Empty Room (was ${displayName.user})"
    is FfiRoomDisplayName.Empty -> "Empty Room"
    is FfiRoomDisplayName.Unknown -> "Unknown"
}

class SkeletonActivity : Activity() {
    private lateinit var status: TextView
    private lateinit var list: ListView
    private lateinit var adapter: ArrayAdapter<String>
    private lateinit var app: CoreApp

    /// The room id behind each sidebar row; null for section headers.
    private var sidebarRowRooms: List<FfiRoom?> = emptyList()
    private var lastRooms: List<FfiRoom> = emptyList()

    /// The room being shown, if the room view is open.
    private var openRoomId: String? = null

    private val timeFormat = SimpleDateFormat("HH:mm", Locale.getDefault())
    private val dateFormat = SimpleDateFormat("EEEE, MMMM d", Locale.getDefault())

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
        list.setOnItemClickListener { _, _, position, _ ->
            if (openRoomId == null) {
                sidebarRowRooms.getOrNull(position)?.let { showRoom(it) }
            }
        }

        showSidebarView()

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
                runOnUiThread {
                    lastRooms = rooms
                    if (openRoomId == null) renderSidebar(rooms)
                }
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

    @Deprecated("Deprecated in Java")
    override fun onBackPressed() {
        if (openRoomId != null) {
            openRoomId = null
            showSidebarView()
            renderSidebar(lastRooms)
        } else {
            @Suppress("DEPRECATION")
            super.onBackPressed()
        }
    }

    private fun setStatus(text: String) {
        runOnUiThread { status.text = text }
    }

    // ---- Sidebar ----

    private fun showSidebarView() {
        val column = LinearLayout(this)
        column.orientation = LinearLayout.VERTICAL
        // API 35 draws edge-to-edge by default; keep the skeleton's content
        // out from under the bars.
        column.fitsSystemWindows = true
        column.addView(
            status.also { (it.parent as? LinearLayout)?.removeView(it) },
            LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                LinearLayout.LayoutParams.WRAP_CONTENT,
            ),
        )
        column.addView(
            list.also { (it.parent as? LinearLayout)?.removeView(it) },
            LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f),
        )
        setContentView(column)
    }

    private fun renderSidebar(rooms: List<FfiRoom>) {
        status.text = "commune-core ${coreVersion()} — ${rooms.size} rooms"

        val lines = mutableListOf<String>()
        val rowRooms = mutableListOf<FfiRoom?>()
        for ((category, title) in CATEGORY_ORDER) {
            val section = rooms
                .filter { it.category == category }
                .sortedByDescending { it.latestActivity }
            if (section.isEmpty()) continue

            lines.add("— $title —")
            rowRooms.add(null)
            for (room in section) {
                val badge = if (room.notificationCount > 0uL) {
                    "  (${room.notificationCount})"
                } else if (!room.isRead) {
                    "  •"
                } else {
                    ""
                }
                val direct = if (room.isDirect) "@ " else ""
                lines.add("    $direct${roomName(room)}$badge")
                rowRooms.add(room)
            }
        }

        sidebarRowRooms = rowRooms
        adapter.clear()
        adapter.addAll(lines)
        adapter.notifyDataSetChanged()
    }

    // ---- Room view ----

    private fun showRoom(room: FfiRoom) {
        openRoomId = room.roomId

        status.text = roomName(room)

        val input = EditText(this)
        input.hint = "Message"

        val send = Button(this)
        send.text = "Send"
        send.setOnClickListener {
            val body = input.text.toString().trim()
            if (body.isEmpty()) return@setOnClickListener
            input.setText("")
            thread {
                runBlocking {
                    try {
                        app.sendMessage(room.roomId, body)
                    } catch (failure: Exception) {
                        setStatus("Send failed: ${failure.message}")
                    }
                }
            }
        }

        val inputRow = LinearLayout(this)
        inputRow.orientation = LinearLayout.HORIZONTAL
        inputRow.addView(input, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
        inputRow.addView(send)

        val column = LinearLayout(this)
        column.orientation = LinearLayout.VERTICAL
        column.fitsSystemWindows = true
        column.addView(
            status.also { (it.parent as? LinearLayout)?.removeView(it) },
            LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                LinearLayout.LayoutParams.WRAP_CONTENT,
            ),
        )
        column.addView(
            list.also { (it.parent as? LinearLayout)?.removeView(it) },
            LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f),
        )
        column.addView(inputRow)
        setContentView(column)

        adapter.clear()
        adapter.add("Loading…")

        app.setTimelineListener(
            room.roomId,
            object : TimelineListener {
                override fun onUpdate(items: List<FfiTimelineItem>) {
                    runOnUiThread {
                        if (openRoomId == room.roomId) renderTimeline(items)
                    }
                }
            },
        )

        // Pull a first page of history in behind the cached events.
        thread { runBlocking { app.paginateBackwards(room.roomId) } }
    }

    private fun renderTimeline(items: List<FfiTimelineItem>) {
        val lines = mutableListOf<String>()

        for (item in items) {
            when (item) {
                is FfiTimelineItem.Event -> {
                    val time = timeFormat.format(Date(item.timestamp.toLong()))
                    val who = item.senderDisplayName ?: item.sender
                    val marker = if (item.isOwn) "»" else " "
                    val text = when (item.kind) {
                        FfiEventKind.TEXT -> item.body
                        FfiEventKind.MEDIA -> "[media] ${item.body}"
                        FfiEventKind.STICKER -> "[sticker]"
                        FfiEventKind.UNABLE_TO_DECRYPT -> "[could not decrypt]"
                        FfiEventKind.REDACTED -> "[message removed]"
                        FfiEventKind.MEMBERSHIP -> "· membership change ·"
                        FfiEventKind.OTHER_STATE -> "· state change ·"
                        FfiEventKind.UNSUPPORTED -> "[unsupported]"
                    }
                    lines.add("$marker $time  $who\n    $text")
                }
                is FfiTimelineItem.DateDivider ->
                    lines.add("——  ${dateFormat.format(Date(item.timestamp.toLong()))}  ——")
                is FfiTimelineItem.ReadMarker -> {}
                is FfiTimelineItem.TimelineStart ->
                    lines.add("——  The conversation starts here  ——")
            }
        }

        adapter.clear()
        adapter.addAll(lines)
        adapter.notifyDataSetChanged()
        // Open at the newest message, like the room-open behavior upstream.
        list.setSelection(adapter.count - 1)
    }
}
