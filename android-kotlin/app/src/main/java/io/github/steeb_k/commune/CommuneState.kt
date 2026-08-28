// The bridge between the Rust core and Compose: the core's listeners and
// suspend calls on one side, snapshot state the UI recomposes from on the
// other.
package io.github.steeb_k.commune

import android.content.Context
import android.os.Handler
import android.os.Looper
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import io.github.steeb_k.commune.core.CoreApp
import io.github.steeb_k.commune.core.FfiCoreConfig
import io.github.steeb_k.commune.core.FfiRoom
import io.github.steeb_k.commune.core.FfiSessionSettings
import io.github.steeb_k.commune.core.FfiTimelineItem
import io.github.steeb_k.commune.core.Native
import io.github.steeb_k.commune.core.RoomListListener
import io.github.steeb_k.commune.core.TimelineListener
import io.github.steeb_k.commune.core.TypingListener
import io.github.steeb_k.commune.core.initCore
import kotlin.concurrent.thread
import kotlinx.coroutines.runBlocking

/// Where the app is, at the top level.
enum class Phase {
    /// Restoring the stored sessions.
    Loading,
    /// No session; the login flow is showing.
    Login,
    /// A session is up (or coming up) and the session UI is showing.
    Session,
}

class CommuneState(context: Context) {
    private val main = Handler(Looper.getMainLooper())

    val app: CoreApp

    var phase by mutableStateOf(Phase.Loading)
        private set
    var rooms by mutableStateOf<List<FfiRoom>>(emptyList())
        private set
    var timeline by mutableStateOf<List<FfiTimelineItem>>(emptyList())
        private set
    var openRoom by mutableStateOf<FfiRoom?>(null)
        private set
    var loginBusy by mutableStateOf(false)
        private set
    var loginError by mutableStateOf<String?>(null)
        private set
    var ownUserId by mutableStateOf<String?>(null)
        private set
    var typingUsers by mutableStateOf<List<String>>(emptyList())
        private set
    var settingsOpen by mutableStateOf(false)
        private set
    var settings by mutableStateOf<FfiSessionSettings?>(null)
        private set

    init {
        Native.seed(context.applicationContext)
        initCore(
            FfiCoreConfig(
                appId = "io.github.steeb_k.commune.skeleton",
                profile = "skeleton",
                dataDir = context.noBackupFilesDir.resolve("commune").absolutePath,
                cacheDir = context.cacheDir.resolve("commune").absolutePath,
            )
        )
        app = CoreApp()

        app.setRoomListListener(object : RoomListListener {
            override fun onUpdate(rooms: List<FfiRoom>) {
                main.post {
                    this@CommuneState.rooms = rooms
                    if (ownUserId == null) ownUserId = app.sessionUserId()
                    if (settings == null) settings = app.sessionSettings()
                }
            }
        })

        thread {
            runBlocking {
                app.restoreSessions()
                main.post {
                    phase = if (app.hasSessions()) Phase.Session else Phase.Login
                }
            }
        }
    }

    fun login(homeserver: String, username: String, password: String) {
        loginBusy = true
        loginError = null

        thread {
            runBlocking {
                try {
                    app.loginWithPassword(homeserver, username, password)
                    main.post {
                        loginBusy = false
                        phase = Phase.Session
                    }
                } catch (failure: Exception) {
                    main.post {
                        loginBusy = false
                        loginError = failure.message ?: "Could not log in"
                    }
                }
            }
        }
    }

    fun openRoom(room: FfiRoom) {
        openRoom = room
        timeline = emptyList()

        app.setTimelineListener(
            room.roomId,
            object : TimelineListener {
                override fun onUpdate(items: List<FfiTimelineItem>) {
                    main.post {
                        if (openRoom?.roomId == room.roomId) {
                            timeline = items
                            // The room is on screen at its newest message:
                            // reading it is what looking at it means.
                            markRead(room.roomId)
                        }
                    }
                }
            },
        )

        app.setTypingListener(
            room.roomId,
            object : TypingListener {
                override fun onUpdate(userIds: List<String>) {
                    main.post {
                        if (openRoom?.roomId == room.roomId) typingUsers = userIds
                    }
                }
            },
        )

        // Pull a first page of history in behind the cached events.
        thread { runBlocking { app.paginateBackwards(room.roomId) } }
    }

    fun closeRoom() {
        openRoom?.let { app.sendTyping(it.roomId, false) }
        openRoom = null
        timeline = emptyList()
        typingUsers = emptyList()
    }

    private var wasTyping = false

    /// Tell the room whether we are typing; only state changes go out.
    fun setTyping(typing: Boolean) {
        val room = openRoom ?: return
        if (typing == wasTyping) return
        wasTyping = typing
        app.sendTyping(room.roomId, typing)
    }

    fun openSettings() {
        settings = app.sessionSettings()
        settingsOpen = true
    }

    fun closeSettings() {
        settingsOpen = false
    }

    fun setNotificationsEnabled(enabled: Boolean) {
        app.setNotificationsEnabled(enabled)
        settings = app.sessionSettings()
    }

    fun setPublicReadReceiptsEnabled(enabled: Boolean) {
        app.setPublicReadReceiptsEnabled(enabled)
        settings = app.sessionSettings()
    }

    fun setTypingEnabled(enabled: Boolean) {
        app.setTypingEnabled(enabled)
        settings = app.sessionSettings()
    }

    private var markingRead = false

    /// Send a read receipt for the given room, coalescing bursts.
    private fun markRead(roomId: String) {
        if (markingRead) return
        markingRead = true
        thread {
            runBlocking {
                try {
                    app.markRoomRead(roomId)
                } catch (_: Exception) {
                    // Nothing to do; the next update tries again.
                }
            }
            main.post { markingRead = false }
        }
    }

    fun send(body: String) {
        val room = openRoom ?: return
        setTyping(false)
        thread {
            runBlocking {
                try {
                    app.sendMessage(room.roomId, body)
                } catch (_: Exception) {
                    // The send queue retries; a failed-send surface comes
                    // with its own chunk.
                }
            }
        }
    }
}
