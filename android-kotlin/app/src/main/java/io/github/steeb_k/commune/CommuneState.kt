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
import io.github.steeb_k.commune.core.FfiMember
import io.github.steeb_k.commune.core.FfiRoom
import io.github.steeb_k.commune.core.FfiRecoveryState
import io.github.steeb_k.commune.core.FfiSessionSettings
import io.github.steeb_k.commune.core.FfiTargetRoomCategory
import io.github.steeb_k.commune.core.FfiTimelineItem
import io.github.steeb_k.commune.core.Native
import io.github.steeb_k.commune.core.MemberListListener
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
    private val appContext = context.applicationContext
    private val notifier = Notifier(appContext)

    /// Whether the activity is in the foreground; backgrounded, the open
    /// room notifies like any other.
    var uiVisible: Boolean = true
        set(value) {
            field = value
            notifier.visibleRoomId = if (value) openRoom?.roomId else null
        }

    /// Set by the activity: opens the system file picker for an attachment.
    var pickAttachment: (() -> Unit)? = null

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
    var openThreadRoot by mutableStateOf<String?>(null)
        private set
    var threadItems by mutableStateOf<List<FfiTimelineItem>>(emptyList())
        private set
    var pinnedOpen by mutableStateOf(false)
        private set
    var pinnedItems by mutableStateOf<List<FfiTimelineItem>>(emptyList())
        private set
    var settings by mutableStateOf<FfiSessionSettings?>(null)
        private set
    var viewerImagePath by mutableStateOf<String?>(null)
        private set
    var viewerIsVideo by mutableStateOf(false)
        private set
    var roomDetailsOpen by mutableStateOf(false)
        private set
    var membersOpen by mutableStateOf(false)
        private set
    var members by mutableStateOf<List<FfiMember>>(emptyList())
        private set

    /// The event whose long-press action sheet is showing, if any.
    var actionSheetEvent by mutableStateOf<FfiTimelineItem.Event?>(null)
        private set

    /// The event a reply is being composed to, if any.
    var replyingTo by mutableStateOf<FfiTimelineItem.Event?>(null)
        private set

    /// The event whose text is being edited, if any.
    var editing by mutableStateOf<FfiTimelineItem.Event?>(null)
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
                    // The open room rides along with its list entry, so a
                    // category flip (accepted invite) reaches the screen.
                    openRoom?.let { current ->
                        rooms.find { it.roomId == current.roomId }?.let { openRoom = it }
                    }
                    if (ownUserId == null) ownUserId = app.sessionUserId()
                    if (settings == null) settings = app.sessionSettings()
                    notifier.enabled = settings?.notificationsEnabled != false
                    notifier.update(rooms)
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
        if (uiVisible) notifier.visibleRoomId = room.roomId

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
        notifier.visibleRoomId = null
        timeline = emptyList()
        typingUsers = emptyList()
        closeThread()
        closeMembers()
        closePinned()
        roomDetailsOpen = false
    }

    fun openRoomDetails() {
        roomDetailsOpen = true
    }

    fun closeRoomDetails() {
        roomDetailsOpen = false
    }

    fun openMembers() {
        val room = openRoom ?: return
        membersOpen = true
        members = emptyList()

        app.setMemberListListener(
            room.roomId,
            object : MemberListListener {
                override fun onUpdate(members: List<FfiMember>) {
                    main.post {
                        if (membersOpen) this@CommuneState.members = members
                    }
                }
            },
        )
    }

    fun closeMembers() {
        membersOpen = false
        members = emptyList()
        app.clearMemberListListener()
    }

    fun openPinned() {
        val room = openRoom ?: return
        pinnedOpen = true
        pinnedItems = emptyList()

        app.setPinnedListener(
            room.roomId,
            object : TimelineListener {
                override fun onUpdate(items: List<FfiTimelineItem>) {
                    main.post {
                        if (pinnedOpen) pinnedItems = items
                    }
                }
            },
        )
    }

    fun closePinned() {
        pinnedOpen = false
        pinnedItems = emptyList()
        app.clearPinnedListener()
    }

    fun openThread(rootEventId: String) {
        val room = openRoom ?: return
        openThreadRoot = rootEventId
        threadItems = emptyList()

        app.setThreadListener(
            room.roomId,
            rootEventId,
            object : TimelineListener {
                override fun onUpdate(items: List<FfiTimelineItem>) {
                    main.post {
                        if (openThreadRoot == rootEventId) threadItems = items
                    }
                }
            },
        )
    }

    fun closeThread() {
        openThreadRoot = null
        threadItems = emptyList()
        app.clearThreadListener()
    }

    fun showActionSheet(event: FfiTimelineItem.Event) {
        actionSheetEvent = event
    }

    fun dismissActionSheet() {
        actionSheetEvent = null
    }

    fun startReply(event: FfiTimelineItem.Event) {
        editing = null
        replyingTo = event
    }

    fun startEdit(event: FfiTimelineItem.Event) {
        replyingTo = null
        editing = event
    }

    fun cancelComposerAction() {
        replyingTo = null
        editing = null
    }

    /// Send the composer's text: as a reply or edit when one is armed,
    /// as a plain message otherwise.
    fun sendFromComposer(body: String) {
        val reply = replyingTo
        val edit = editing
        cancelComposerAction()
        when {
            reply?.eventId != null -> sendReply(reply.eventId!!, body)
            edit?.eventId != null -> sendEdit(edit.eventId!!, body)
            else -> send(body)
        }
    }

    private fun sendReply(inReplyTo: String, body: String) {
        val room = openRoom ?: return
        thread {
            runBlocking {
                try {
                    app.sendReply(room.roomId, inReplyTo, body)
                } catch (_: Exception) {
                    // The next update reflects reality either way.
                }
            }
        }
    }

    private fun sendEdit(eventId: String, body: String) {
        val room = openRoom ?: return
        thread {
            runBlocking {
                try {
                    app.editMessage(room.roomId, eventId, body)
                } catch (_: Exception) {
                    // The next update reflects reality either way.
                }
            }
        }
    }

    fun redact(eventId: String) {
        val room = openRoom ?: return
        thread {
            runBlocking {
                try {
                    app.redactEvent(room.roomId, eventId)
                } catch (_: Exception) {
                    // The next update reflects reality either way.
                }
            }
        }
    }

    /// Busy/error state of the create-or-join dialogs.
    var conversationBusy by mutableStateOf(false)
        private set
    var conversationError by mutableStateOf<String?>(null)
        private set

    fun clearConversationError() {
        conversationError = null
    }

    /// Open (or create) the direct chat with the given user, then show it
    /// once it reaches the room list.
    fun startDirectChat(userId: String, onDone: () -> Unit) {
        conversationBusy = true
        conversationError = null
        thread {
            runBlocking {
                try {
                    val roomId = app.createDirectChat(userId.trim())
                    main.post {
                        conversationBusy = false
                        onDone()
                        openRoomWhenListed(roomId)
                    }
                } catch (failure: Exception) {
                    main.post {
                        conversationBusy = false
                        conversationError = failure.message?.removePrefix("msg=") ?: "Could not open the chat"
                    }
                }
            }
        }
    }

    /// Join the room with the given ID or alias, then show it once it
    /// reaches the room list.
    fun joinRoom(idOrAlias: String, onDone: () -> Unit) {
        conversationBusy = true
        conversationError = null
        thread {
            runBlocking {
                try {
                    val roomId = app.joinRoom(idOrAlias.trim())
                    main.post {
                        conversationBusy = false
                        onDone()
                        openRoomWhenListed(roomId)
                    }
                } catch (failure: Exception) {
                    main.post {
                        conversationBusy = false
                        conversationError = failure.message?.removePrefix("msg=") ?: "Could not join the room"
                    }
                }
            }
        }
    }

    /// Open the given room as soon as the room list carries it — freshly
    /// created rooms arrive with the next sync.
    private fun openRoomWhenListed(roomId: String, attempt: Int = 0) {
        val room = rooms.find { it.roomId == roomId }
        when {
            room != null -> openRoom(room)
            attempt < 20 -> main.postDelayed({ openRoomWhenListed(roomId, attempt + 1) }, 500)
        }
    }

    /// Move any room to a category from the sidebar's long-press menu.
    fun changeRoomCategory(roomId: String, category: FfiTargetRoomCategory) {
        thread {
            runBlocking {
                try {
                    app.changeRoomCategory(roomId, category)
                } catch (_: Exception) {
                    // The sidebar reflects what actually happened.
                }
            }
        }
    }

    fun acceptInvite() {
        val room = openRoom ?: return
        thread {
            runBlocking {
                try {
                    app.changeRoomCategory(room.roomId, FfiTargetRoomCategory.NORMAL)
                } catch (_: Exception) {
                    // The sidebar reflects what actually happened.
                }
            }
        }
    }

    fun declineInvite() {
        val room = openRoom ?: return
        main.post { closeRoom() }
        thread {
            runBlocking {
                try {
                    app.changeRoomCategory(room.roomId, FfiTargetRoomCategory.LEFT)
                } catch (_: Exception) {
                    // The sidebar reflects what actually happened.
                }
            }
        }
    }

    fun toggleReaction(eventId: String, key: String) {
        val room = openRoom ?: return
        thread {
            runBlocking {
                try {
                    app.toggleReaction(room.roomId, eventId, key)
                } catch (_: Exception) {
                    // The next update reflects reality either way.
                }
            }
        }
    }

    fun sendAttachmentFromUri(uri: android.net.Uri) {
        val room = openRoom ?: return
        val resolver = appContext.contentResolver
        val mime = resolver.getType(uri) ?: "application/octet-stream"

        thread {
            try {
                var name = "attachment"
                resolver.query(uri, null, null, null, null)?.use { cursor ->
                    val index =
                        cursor.getColumnIndex(android.provider.OpenableColumns.DISPLAY_NAME)
                    if (index >= 0 && cursor.moveToFirst()) {
                        name = cursor.getString(index) ?: name
                    }
                }

                val dir = java.io.File(appContext.cacheDir, "outgoing")
                dir.mkdirs()
                val file = java.io.File(dir, name)
                resolver.openInputStream(uri)?.use { input ->
                    file.outputStream().use { output -> input.copyTo(output) }
                } ?: return@thread

                runBlocking { app.sendAttachment(room.roomId, file.absolutePath, mime) }
            } catch (_: Exception) {
                // The timeline reflects what actually sent.
            }
        }
    }

    fun sendInThread(body: String) {
        val room = openRoom ?: return
        val root = openThreadRoot ?: return
        thread {
            runBlocking {
                try {
                    app.sendThreadMessage(room.roomId, root, body)
                } catch (_: Exception) {
                    // The next update reflects reality either way.
                }
            }
        }
    }

    private var wasTyping = false

    /// Tell the room whether we are typing; only state changes go out.
    fun setTyping(typing: Boolean) {
        val room = openRoom ?: return
        if (typing == wasTyping) return
        wasTyping = typing
        app.sendTyping(room.roomId, typing)
    }

    var recoveryState by mutableStateOf(FfiRecoveryState.UNKNOWN)
        private set
    var recoveryKey by mutableStateOf<String?>(null)
        private set
    var recoveryBusy by mutableStateOf(false)
        private set
    var recoveryError by mutableStateOf<String?>(null)
        private set

    fun refreshRecoveryState() {
        thread {
            runBlocking {
                val state = app.recoveryState()
                main.post { recoveryState = state }
            }
        }
    }

    fun enableRecovery() {
        recoveryBusy = true
        recoveryError = null
        thread {
            runBlocking {
                try {
                    val key = app.enableRecovery()
                    main.post {
                        recoveryBusy = false
                        recoveryKey = key
                    }
                } catch (failure: Exception) {
                    main.post {
                        recoveryBusy = false
                        recoveryError = failure.message?.removePrefix("msg=")
                            ?: "Could not set up recovery"
                    }
                }
                main.post { refreshRecoveryState() }
            }
        }
    }

    fun recover(key: String) {
        recoveryBusy = true
        recoveryError = null
        thread {
            runBlocking {
                try {
                    app.recover(key)
                    main.post { recoveryBusy = false }
                } catch (failure: Exception) {
                    main.post {
                        recoveryBusy = false
                        recoveryError = failure.message?.removePrefix("msg=")
                            ?: "Could not recover"
                    }
                }
                main.post { refreshRecoveryState() }
            }
        }
    }

    fun dismissRecoveryKey() {
        recoveryKey = null
    }

    fun openSettings() {
        settings = app.sessionSettings()
        settingsOpen = true
        refreshRecoveryState()
    }

    fun closeSettings() {
        settingsOpen = false
    }

    fun openViewer(path: String) {
        viewerIsVideo = false
        viewerImagePath = path
    }

    /// Fetch the media behind the given timeline item, then open it as
    /// video or audio playback.
    fun openMediaPlayer(uniqueId: String) {
        val room = openRoom ?: return
        thread {
            runBlocking {
                try {
                    val path = app.getTimelineMedia(room.roomId, uniqueId)
                    if (path != null) {
                        main.post {
                            viewerIsVideo = true
                            viewerImagePath = path
                        }
                    }
                } catch (_: Exception) {
                    // Nothing to show.
                }
            }
        }
    }

    fun closeViewer() {
        viewerImagePath = null
        viewerIsVideo = false
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
    fun markRead(roomId: String) {
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
