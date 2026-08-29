// The bridge between the Rust core and Compose: the core's listeners and
// suspend calls on one side, snapshot state the UI recomposes from on the
// other.
package io.github.steeb_k.commune

import android.content.Context
import android.os.Handler
import android.os.Looper
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import io.github.steeb_k.commune.core.CoreApp
import io.github.steeb_k.commune.core.FfiCoreConfig
import io.github.steeb_k.commune.core.FfiDevice
import io.github.steeb_k.commune.core.FfiRoomNotificationMode
import io.github.steeb_k.commune.core.FfiGif
import io.github.steeb_k.commune.core.FfiHistoryEvent
import io.github.steeb_k.commune.core.FfiHistoryKind
import io.github.steeb_k.commune.core.FfiMember
import io.github.steeb_k.commune.core.FfiRoom
import io.github.steeb_k.commune.core.FfiRecoveryState
import io.github.steeb_k.commune.core.FfiSasEmoji
import io.github.steeb_k.commune.core.FfiPublicRoom
import io.github.steeb_k.commune.core.FfiRoomCategory
import io.github.steeb_k.commune.core.FfiSearchResult
import io.github.steeb_k.commune.core.FfiSessionSettings
import io.github.steeb_k.commune.core.FfiSticker
import io.github.steeb_k.commune.core.FfiStickerPack
import io.github.steeb_k.commune.core.FfiSpaceChild
import io.github.steeb_k.commune.core.FfiTargetRoomCategory
import io.github.steeb_k.commune.core.FfiTimelineItem
import io.github.steeb_k.commune.core.Native
import io.github.steeb_k.commune.core.MemberListListener
import io.github.steeb_k.commune.core.RoomListListener
import io.github.steeb_k.commune.core.TimelineListener
import io.github.steeb_k.commune.core.TypingListener
import io.github.steeb_k.commune.core.VerificationListener
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

    /// Set by the activity: opens the QR scanner for verification.
    var scanQrCode: (() -> Unit)? = null

    val app: CoreApp

    var phase by mutableStateOf(Phase.Loading)
        private set
    var rooms by mutableStateOf<List<FfiRoom>>(emptyList())
        private set
    var timeline by mutableStateOf<List<FfiTimelineItem>>(emptyList())
        private set
    var timelineLoading by mutableStateOf(false)
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

    /// The room-list bridge. The core binds a listener to one session, so
    /// it must be re-armed whenever a new session replaces an old one —
    /// after a fresh login, not just at startup.
    private val roomListListener = object : RoomListListener {
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
                // The first delivery proves the session is ready; the
                // profile fetch at startup can have been too early.
                if (profileName == null) loadProfile()
                notifier.enabled = settings?.notificationsEnabled != false
                notifier.update(rooms)
            }
        }
    }

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

        app.setRoomListListener(roomListListener)

        thread {
            runBlocking {
                app.restoreSessions()
                main.post {
                    phase = if (app.hasSessions()) Phase.Session else Phase.Login
                    if (phase == Phase.Session) {
                        watchVerifications()
                        loadProfile()
                    }
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
                        // The old listener task died with the old session.
                        app.setRoomListListener(roomListListener)
                        watchVerifications()
                        loadProfile()
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

    var openSpace by mutableStateOf<FfiRoom?>(null)
        private set
    var spaceChildren by mutableStateOf<List<FfiSpaceChild>>(emptyList())
        private set
    var spaceLoading by mutableStateOf(false)
        private set

    fun openSpace(space: FfiRoom) {
        openSpace = space
        spaceChildren = emptyList()
        spaceLoading = true
        thread {
            runBlocking {
                val children = try {
                    app.spaceChildren(space.roomId)
                } catch (_: Exception) {
                    emptyList()
                }
                main.post {
                    if (openSpace?.roomId == space.roomId) {
                        spaceChildren = children
                        spaceLoading = false
                    }
                }
            }
        }
    }

    fun closeSpace() {
        openSpace = null
        spaceChildren = emptyList()
    }

    /// The open room's members, for the composer's mention completion.
    var composerMembers by mutableStateOf<List<FfiMember>>(emptyList())
        private set

    /// Where each room's timeline was left, when it was left away from the
    /// bottom: room id → (anchor event id, pixel offset). A room left at
    /// the bottom has no entry and opens at the newest message.
    val savedScroll = mutableMapOf<String, Pair<String, Int>>()

    /// Set while a notification tap wants the room opened at the oldest
    /// unread message; cleared once the timeline has made the jump.
    var jumpToUnread by mutableStateOf(false)
        private set

    /// Set for the whole of a notification-opened visit: marking read
    /// would remove the read-marker line (and the timeline anchor with
    /// it) while the user is still reading up. The room is marked read
    /// when it is left instead.
    private var suppressMarkRead = false

    /// The unread jump landed (or gave up).
    fun completeUnreadJump() {
        jumpToUnread = false
    }

    private var paginatingOlder = false

    /// Pull one more page of history into the open room's timeline.
    fun paginateOlder() {
        val room = openRoom ?: return
        if (paginatingOlder) return
        paginatingOlder = true
        thread {
            runBlocking { app.paginateBackwards(room.roomId) }
            paginatingOlder = false
        }
    }

    fun openRoom(room: FfiRoom, toUnread: Boolean = false) {
        if (room.category == FfiRoomCategory.SPACE) {
            openSpace(room)
            return
        }
        // A room opened from inside a space must replace the space view,
        // which outranks the room in the routing chain.
        closeSpace()
        openRoom = room
        jumpToUnread = toUnread
        suppressMarkRead = toUnread
        roomNotifMode = FfiRoomNotificationMode.DEFAULT
        loadRoomNotificationMode()
        timeline = emptyList()
        timelineLoading = true
        composerMembers = emptyList()
        thread {
            runBlocking {
                val members = app.roomMembers(room.roomId)
                main.post {
                    if (openRoom?.roomId == room.roomId) composerMembers = members
                }
            }
        }
        if (uiVisible) notifier.visibleRoomId = room.roomId

        app.setTimelineListener(
            room.roomId,
            object : TimelineListener {
                override fun onUpdate(items: List<FfiTimelineItem>) {
                    main.post {
                        if (openRoom?.roomId == room.roomId) {
                            timeline = items
                            timelineLoading = false
                            // The room is on screen: reading it is what
                            // looking at it means — except during a
                            // notification-opened visit, which keeps
                            // the read marker where it was until the
                            // room is left.
                            if (!suppressMarkRead) markRead(room.roomId)
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
        paginateOlder()
    }

    fun closeRoom() {
        openRoom?.let { room ->
            app.sendTyping(room.roomId, false)
            // A notification-opened visit marks read on the way out.
            if (suppressMarkRead) markRead(room.roomId)
        }
        suppressMarkRead = false
        openRoom = null
        jumpToUnread = false
        notifier.visibleRoomId = null
        timeline = emptyList()
        typingUsers = emptyList()
        closeThread()
        closeMembers()
        closePinned()
        closeRoomSearch()
        closeHistory()
        roomDetailsOpen = false
    }

    var detailsError by mutableStateOf<String?>(null)
        private set

    fun setRoomDetails(name: String, topic: String, onDone: () -> Unit) {
        val room = openRoom ?: return
        detailsError = null
        thread {
            runBlocking {
                try {
                    app.setRoomDetails(room.roomId, name, topic)
                    main.post { onDone() }
                } catch (failure: Exception) {
                    main.post {
                        detailsError = failure.message?.removePrefix("msg=")
                            ?: "Could not save"
                    }
                }
            }
        }
    }

    var roomSearchOpen by mutableStateOf(false)
        private set
    var roomSearchResults by mutableStateOf<List<FfiSearchResult>>(emptyList())
        private set
    var roomSearchBusy by mutableStateOf(false)
        private set

    fun openRoomSearch() {
        roomSearchOpen = true
        roomSearchResults = emptyList()
    }

    fun closeRoomSearch() {
        roomSearchOpen = false
        roomSearchResults = emptyList()
    }

    fun searchRoom(query: String) {
        val room = openRoom ?: return
        roomSearchBusy = true
        thread {
            runBlocking {
                val results = try {
                    app.searchRoom(room.roomId, query)
                } catch (_: Exception) {
                    emptyList()
                }
                main.post {
                    if (roomSearchOpen) {
                        roomSearchResults = results
                        roomSearchBusy = false
                    }
                }
            }
        }
    }

    // The GIF picker — searches KLIPY through the core, previews cached
    // per-URL so scrolling back does not re-download.
    var gifPickerOpen by mutableStateOf(false)
        private set
    var gifResults by mutableStateOf<List<FfiGif>>(emptyList())
        private set
    var gifBusy by mutableStateOf(false)
        private set
    var gifHasNext by mutableStateOf(false)
        private set
    private var gifQuery = ""
    private var gifPage = 0
    val gifPreviews = mutableStateMapOf<String, ByteArray>()

    fun openGifPicker() {
        gifPickerOpen = true
    }

    fun closeGifPicker() {
        gifPickerOpen = false
        gifResults = emptyList()
        gifHasNext = false
        gifQuery = ""
        gifPage = 0
        gifPreviews.clear()
    }

    fun searchGifs(query: String) {
        gifQuery = query
        gifPage = 1
        gifResults = emptyList()
        gifPreviews.clear()
        fetchGifPage(replace = true)
    }

    fun loadMoreGifs() {
        if (gifBusy || !gifHasNext) return
        gifPage += 1
        fetchGifPage(replace = false)
    }

    private fun fetchGifPage(replace: Boolean) {
        val query = gifQuery
        val page = gifPage
        gifBusy = true
        thread {
            runBlocking {
                val result = try {
                    app.searchGifs(query, page.toUInt())
                } catch (_: Exception) {
                    null
                }
                main.post {
                    if (gifPickerOpen && query == gifQuery) {
                        if (result != null) {
                            gifResults = if (replace) result.gifs else gifResults + result.gifs
                            gifHasNext = result.hasNext
                        }
                        gifBusy = false
                    }
                }
                // Previews arrive one by one, each posted as it lands.
                result?.gifs?.forEach { gif ->
                    if (gifPreviews.containsKey(gif.previewUrl)) return@forEach
                    val bytes = try {
                        app.fetchGifPreview(gif.previewUrl)
                    } catch (_: Exception) {
                        return@forEach
                    }
                    main.post {
                        if (gifPickerOpen) gifPreviews[gif.previewUrl] = bytes
                    }
                }
            }
        }
    }

    fun sendGif(gif: FfiGif) {
        val room = openRoom ?: return
        closeGifPicker()
        thread {
            runBlocking {
                try {
                    app.sendGif(room.roomId, gif)
                } catch (_: Exception) {
                }
            }
        }
    }

    // The media history pages under room details: one paginated list per
    // kind, walked backward through /messages as the user scrolls.
    var historyKind by mutableStateOf<FfiHistoryKind?>(null)
        private set
    var historyEvents by mutableStateOf<List<FfiHistoryEvent>>(emptyList())
        private set
    var historyBusy by mutableStateOf(false)
        private set
    private var historyNextToken: String? = null
    private var historyDone = false

    fun openHistory(kind: FfiHistoryKind) {
        historyKind = kind
        historyEvents = emptyList()
        historyNextToken = null
        historyDone = false
        loadMoreHistory()
    }

    fun closeHistory() {
        historyKind = null
        historyEvents = emptyList()
    }

    fun loadMoreHistory() {
        val room = openRoom ?: return
        val kind = historyKind ?: return
        if (historyBusy || historyDone) return
        historyBusy = true
        thread {
            runBlocking {
                val page = try {
                    app.roomMediaHistory(room.roomId, historyNextToken)
                } catch (_: Exception) {
                    null
                }
                main.post {
                    if (historyKind == kind) {
                        if (page != null) {
                            historyEvents = historyEvents + page.events.filter { it.kind == kind }
                            historyNextToken = page.nextToken
                            historyDone = page.nextToken == null
                        }
                        historyBusy = false
                    }
                }
            }
        }
    }

    /// Media paths for history events, filled as thumbnails download.
    val historyMedia = mutableStateMapOf<String, String>()

    fun fetchHistoryMedia(eventId: String, onDone: ((String?) -> Unit)? = null) {
        val room = openRoom ?: return
        if (historyMedia.containsKey(eventId)) {
            onDone?.invoke(historyMedia[eventId])
            return
        }
        thread {
            runBlocking {
                val path = try {
                    app.getHistoryMedia(room.roomId, eventId)
                } catch (_: Exception) {
                    null
                }
                main.post {
                    if (path != null) historyMedia[eventId] = path
                    onDone?.invoke(path)
                }
            }
        }
    }

    // The public room directory — the GTK Explore page.
    var exploreOpen by mutableStateOf(false)
        private set
    var exploreRooms by mutableStateOf<List<FfiPublicRoom>>(emptyList())
        private set
    var exploreBusy by mutableStateOf(false)
        private set
    private var exploreQuery: String? = null
    private var exploreNextBatch: String? = null
    private var exploreDone = false

    fun openExplore() {
        exploreOpen = true
        searchExplore(null)
    }

    fun closeExplore() {
        exploreOpen = false
        exploreRooms = emptyList()
    }

    fun searchExplore(query: String?) {
        exploreQuery = query?.takeIf { it.isNotBlank() }
        exploreRooms = emptyList()
        exploreNextBatch = null
        exploreDone = false
        loadMoreExplore()
    }

    fun loadMoreExplore() {
        if (exploreBusy || exploreDone) return
        val query = exploreQuery
        exploreBusy = true
        thread {
            runBlocking {
                val page = try {
                    app.exploreRooms(query, exploreNextBatch)
                } catch (_: Exception) {
                    null
                }
                main.post {
                    if (exploreOpen && query == exploreQuery) {
                        if (page != null) {
                            exploreRooms = exploreRooms + page.rooms
                            exploreNextBatch = page.nextBatch
                            exploreDone = page.nextBatch == null
                        }
                        exploreBusy = false
                    }
                }
            }
        }
    }

    /// Join a room from the directory, flipping its row when the server
    /// confirms.
    fun joinExploreRoom(room: FfiPublicRoom) {
        thread {
            runBlocking {
                try {
                    app.joinRoom(room.roomId)
                    main.post {
                        exploreRooms = exploreRooms.map {
                            if (it.roomId == room.roomId) it.copy(isJoined = true) else it
                        }
                    }
                } catch (_: Exception) {
                }
            }
        }
    }

    /// Wake the send queue back up so failed messages go out again.
    fun retrySends() {
        thread { runBlocking { try { app.retrySends() } catch (_: Exception) {} } }
    }

    /// Save the given history events into the device's Downloads, fetching
    /// any that are not cached yet. Calls back with how many were saved.
    fun downloadHistoryEvents(events: List<FfiHistoryEvent>, onDone: (Int) -> Unit) {
        val room = openRoom ?: return
        thread {
            var saved = 0
            runBlocking {
                for (event in events) {
                    val path = historyMedia[event.eventId] ?: try {
                        app.getHistoryMedia(room.roomId, event.eventId)
                    } catch (_: Exception) {
                        null
                    } ?: continue
                    if (saveToDownloads(path, event.body, event.mimeType)) saved += 1
                }
            }
            main.post { onDone(saved) }
        }
    }

    /// Copy one media file into MediaStore's Downloads collection — the
    /// scoped-storage way, no permission needed on our minSdk.
    private fun saveToDownloads(path: String, name: String, mime: String?): Boolean = try {
        val values = android.content.ContentValues().apply {
            put(
                android.provider.MediaStore.MediaColumns.DISPLAY_NAME,
                name.ifBlank { "commune-media" },
            )
            mime?.let { put(android.provider.MediaStore.MediaColumns.MIME_TYPE, it) }
            put(android.provider.MediaStore.MediaColumns.IS_PENDING, 1)
        }
        val resolver = appContext.contentResolver
        val uri = resolver.insert(
            android.provider.MediaStore.Downloads.EXTERNAL_CONTENT_URI,
            values,
        )
        if (uri == null) {
            false
        } else {
            resolver.openOutputStream(uri)!!.use { out ->
                java.io.File(path).inputStream().use { it.copyTo(out) }
            }
            values.clear()
            values.put(android.provider.MediaStore.MediaColumns.IS_PENDING, 0)
            resolver.update(uri, values, null, null)
            true
        }
    } catch (_: Exception) {
        false
    }

    // How notifications arrive: UnifiedPush (ntfy) preferred, the
    // foreground sync as fallback. MODE_UNSET means the onboarding page
    // has not been answered yet.
    var pushMode by mutableStateOf(PushManager.mode(context.applicationContext))
        private set
    var pushError by mutableStateOf<String?>(null)
        private set
    var pushBusy by mutableStateOf(false)
        private set

    fun refreshPushMode() {
        pushMode = PushManager.mode(appContext)
    }

    /// Register with the distributor, then point the homeserver at its
    /// Matrix gateway.
    fun connectUnifiedPush() {
        pushBusy = true
        pushError = null
        PushManager.onEndpoint = { endpoint ->
            thread {
                runBlocking {
                    val result = try {
                        val url = java.net.URL(endpoint)
                        val gateway = "${url.protocol}://${url.authority}/_matrix/push/v1/notify"
                        app.setPushGateway(gateway, endpoint)
                        null
                    } catch (failure: Exception) {
                        failure.message ?: "Could not set up the pusher"
                    }
                    main.post {
                        pushBusy = false
                        if (result == null) {
                            PushManager.setMode(appContext, PushManager.MODE_UNIFIEDPUSH)
                            pushMode = PushManager.MODE_UNIFIEDPUSH
                            appContext.stopService(
                                android.content.Intent(appContext, SyncService::class.java)
                            )
                        } else {
                            pushError = result
                        }
                    }
                }
            }
        }
        PushManager.onFailed = { reason ->
            main.post {
                pushBusy = false
                pushError = "Registration failed: $reason"
            }
        }
        PushManager.connect(appContext)
    }

    /// Keep the foreground sync; it is already running.
    fun chooseBackgroundSync() {
        PushManager.setMode(appContext, PushManager.MODE_SYNC)
        pushMode = PushManager.MODE_SYNC
    }

    /// Re-open the onboarding page, from settings. Leaving UnifiedPush
    /// removes the pusher and restarts the sync first.
    fun reopenPushOnboarding() {
        val endpoint = PushManager.endpoint(appContext)
        if (pushMode == PushManager.MODE_UNIFIEDPUSH && endpoint != null) {
            thread {
                runBlocking {
                    try {
                        app.removePushGateway(endpoint)
                    } catch (_: Exception) {
                    }
                }
            }
            PushManager.disconnect(appContext)
            SyncService.start(appContext)
        }
        PushManager.setMode(appContext, PushManager.MODE_UNSET)
        pushMode = PushManager.MODE_UNSET
    }

    // The account: profile, and the way out.
    var profileName by mutableStateOf<String?>(null)
        private set
    var profileAvatarPath by mutableStateOf<String?>(null)
        private set

    fun loadProfile() {
        thread {
            runBlocking {
                val profile = try {
                    app.accountProfile()
                } catch (_: Exception) {
                    null
                }
                main.post { profileName = profile?.displayName }
                val avatarPath = profile?.avatarUrl?.let { url ->
                    try {
                        app.getMxcMedia(url)
                    } catch (_: Exception) {
                        null
                    }
                }
                main.post { profileAvatarPath = avatarPath }
            }
        }
    }

    fun setDisplayName(name: String, onDone: (String?) -> Unit) {
        thread {
            runBlocking {
                val error = try {
                    app.setDisplayName(name)
                    null
                } catch (failure: Exception) {
                    failure.message ?: "Could not change the display name"
                }
                main.post {
                    if (error == null) profileName = name.trim()
                    onDone(error)
                }
            }
        }
    }

    /// Set by the activity: opens the image picker for a new avatar.
    var pickAvatar: (() -> Unit)? = null

    fun setAvatarFromUri(uri: android.net.Uri) {
        val resolver = appContext.contentResolver
        val mime = resolver.getType(uri) ?: "image/jpeg"
        thread {
            try {
                val dir = java.io.File(appContext.cacheDir, "outgoing")
                dir.mkdirs()
                val file = java.io.File(dir, "avatar")
                resolver.openInputStream(uri)?.use { input ->
                    file.outputStream().use { output -> input.copyTo(output) }
                } ?: return@thread
                runBlocking { app.setAccountAvatar(file.absolutePath, mime) }
            } catch (_: Exception) {
            }
        }
    }

    /// Log out: pusher first (its removal needs the access token that
    /// logout invalidates), then the session, then back to the login page.
    fun logout() {
        thread {
            runBlocking {
                PushManager.endpoint(appContext)?.let { endpoint ->
                    try {
                        app.removePushGateway(endpoint)
                    } catch (_: Exception) {
                    }
                }
                if (pushMode == PushManager.MODE_UNIFIEDPUSH) {
                    PushManager.disconnect(appContext)
                }
                try {
                    app.logout()
                } catch (_: Exception) {
                }
                main.post {
                    PushManager.setMode(appContext, PushManager.MODE_UNSET)
                    pushMode = PushManager.MODE_UNSET
                    appContext.stopService(
                        android.content.Intent(appContext, SyncService::class.java)
                    )
                    openRoom = null
                    rooms = emptyList()
                    timeline = emptyList()
                    settingsOpen = false
                    phase = Phase.Login
                }
            }
        }
    }

    /// Invite a user to the open room.
    fun inviteUser(userId: String, onDone: (String?) -> Unit) {
        val room = openRoom ?: return
        thread {
            runBlocking {
                val error = try {
                    app.inviteUser(room.roomId, userId)
                    null
                } catch (failure: Exception) {
                    failure.message ?: "Could not invite"
                }
                main.post { onDone(error) }
            }
        }
    }

    /// Create a room and open it once it appears in the list.
    fun createRoom(
        name: String,
        topic: String,
        public: Boolean,
        encrypted: Boolean,
        alias: String,
        onDone: (String?) -> Unit,
    ) {
        thread {
            runBlocking {
                val result = try {
                    app.createRoom(
                        name,
                        topic.ifBlank { null },
                        public,
                        encrypted,
                        alias.ifBlank { null },
                    )
                } catch (failure: Exception) {
                    main.post { onDone(failure.message ?: "Could not create the room") }
                    return@runBlocking
                }
                main.post {
                    onDone(null)
                    openRoomWhenListed(result)
                }
            }
        }
    }

    // The account's sessions.
    var devicesOpen by mutableStateOf(false)
        private set
    var devices by mutableStateOf<List<FfiDevice>>(emptyList())
        private set
    var devicesBusy by mutableStateOf(false)
        private set

    fun openDevices() {
        devicesOpen = true
        refreshDevices()
    }

    fun closeDevices() {
        devicesOpen = false
        devices = emptyList()
    }

    fun refreshDevices() {
        devicesBusy = true
        thread {
            runBlocking {
                val list = try {
                    app.listDevices()
                } catch (_: Exception) {
                    emptyList()
                }
                main.post {
                    if (devicesOpen) {
                        devices = list
                        devicesBusy = false
                    }
                }
            }
        }
    }

    fun renameDevice(deviceId: String, name: String, onDone: (String?) -> Unit) {
        thread {
            runBlocking {
                val error = try {
                    app.renameDevice(deviceId, name)
                    null
                } catch (failure: Exception) {
                    failure.message ?: "Could not rename"
                }
                main.post {
                    onDone(error)
                    if (error == null) refreshDevices()
                }
            }
        }
    }

    fun signOutDevice(deviceId: String, password: String, onDone: (String?) -> Unit) {
        thread {
            runBlocking {
                val error = try {
                    app.signOutDevice(deviceId, password)
                    null
                } catch (failure: Exception) {
                    failure.message ?: "Could not sign out"
                }
                main.post {
                    onDone(error)
                    if (error == null) refreshDevices()
                }
            }
        }
    }

    // The open room's notification mode.
    var roomNotifMode by mutableStateOf(FfiRoomNotificationMode.DEFAULT)
        private set

    fun loadRoomNotificationMode() {
        val room = openRoom ?: return
        thread {
            runBlocking {
                val mode = try {
                    app.roomNotificationMode(room.roomId)
                } catch (_: Exception) {
                    FfiRoomNotificationMode.DEFAULT
                }
                main.post {
                    if (openRoom?.roomId == room.roomId) roomNotifMode = mode
                }
            }
        }
    }

    fun setRoomNotificationMode(mode: FfiRoomNotificationMode) {
        val room = openRoom ?: return
        roomNotifMode = mode
        thread {
            runBlocking {
                try {
                    app.setRoomNotificationMode(room.roomId, mode)
                } catch (_: Exception) {
                }
            }
        }
    }

    // Moderation, from the members page.
    fun kickUser(userId: String, onDone: (String?) -> Unit) {
        moderate(onDone) { app.kickUser(it, userId, null) }
    }

    fun banUser(userId: String, onDone: (String?) -> Unit) {
        moderate(onDone) { app.banUser(it, userId, null) }
    }

    fun setMemberPower(userId: String, level: Long, onDone: (String?) -> Unit) {
        moderate(onDone) { app.setMemberPowerLevel(it, userId, level) }
    }

    private fun moderate(onDone: (String?) -> Unit, action: suspend (String) -> Unit) {
        val room = openRoom ?: return
        thread {
            runBlocking {
                val error = try {
                    action(room.roomId)
                    null
                } catch (failure: Exception) {
                    failure.message ?: "Could not do that"
                }
                main.post { onDone(error) }
            }
        }
    }

    // Room-key export and import — the file goes through Downloads on the
    // way out and the document picker on the way in.
    fun exportKeys(passphrase: String, onDone: (String?) -> Unit) {
        thread {
            runBlocking {
                val error = try {
                    val dir = java.io.File(appContext.cacheDir, "outgoing")
                    dir.mkdirs()
                    val file = java.io.File(dir, "commune-keys.txt")
                    app.exportRoomKeys(file.absolutePath, passphrase)
                    if (!saveToDownloads(file.absolutePath, "commune-keys.txt", "text/plain")) {
                        throw RuntimeException("Could not write to Downloads")
                    }
                    file.delete()
                    null
                } catch (failure: Exception) {
                    failure.message ?: "Could not export the keys"
                }
                main.post { onDone(error) }
            }
        }
    }

    /// Set by the activity: opens the document picker for a key file.
    var pickKeyFile: (() -> Unit)? = null

    /// The passphrase the pending import will use, set before the picker.
    var pendingImportPassphrase: String? = null

    /// Where the last import ended up, for the settings page to show.
    var importResult by mutableStateOf<String?>(null)
        private set

    fun importKeysFromUri(uri: android.net.Uri) {
        val passphrase = pendingImportPassphrase ?: return
        pendingImportPassphrase = null
        thread {
            runBlocking {
                val result = try {
                    val dir = java.io.File(appContext.cacheDir, "outgoing")
                    dir.mkdirs()
                    val file = java.io.File(dir, "incoming-keys.txt")
                    appContext.contentResolver.openInputStream(uri)?.use { input ->
                        file.outputStream().use { output -> input.copyTo(output) }
                    } ?: throw RuntimeException("Could not read the file")
                    val count = app.importRoomKeys(file.absolutePath, passphrase)
                    file.delete()
                    "Imported $count keys"
                } catch (failure: Exception) {
                    failure.message ?: "Could not import the keys"
                }
                main.post { importResult = result }
            }
        }
    }

    fun clearImportResult() {
        importResult = null
    }

    // Sticker packs from the account's image packs, for the picker.
    var stickerPacks by mutableStateOf<List<FfiStickerPack>>(emptyList())
        private set
    var stickerPacksLoaded by mutableStateOf(false)
        private set
    val stickerMedia = mutableStateMapOf<String, String>()

    fun loadStickerPacks() {
        thread {
            runBlocking {
                val packs = try {
                    app.stickerPacks()
                } catch (_: Exception) {
                    emptyList()
                }
                main.post {
                    stickerPacks = packs
                    stickerPacksLoaded = true
                }
                for (pack in packs) {
                    for (sticker in pack.stickers) {
                        if (stickerMedia.containsKey(sticker.url)) continue
                        val path = try {
                            app.getMxcMedia(sticker.url)
                        } catch (_: Exception) {
                            null
                        } ?: continue
                        main.post { stickerMedia[sticker.url] = path }
                    }
                }
            }
        }
    }

    fun sendSticker(sticker: FfiSticker) {
        val room = openRoom ?: return
        closeGifPicker()
        thread {
            runBlocking {
                try {
                    app.sendSticker(room.roomId, sticker)
                } catch (_: Exception) {
                }
            }
        }
    }

    // Voice messages: MediaRecorder into the outgoing cache, then the
    // core sends it with the voice marker.
    var recordingVoice by mutableStateOf(false)
        private set
    private var recorder: android.media.MediaRecorder? = null
    private var recordingFile: java.io.File? = null
    var recordingStarted = 0L
        private set

    /// Set by the activity: asks for the microphone permission; calls
    /// back with whether it is granted.
    var ensureMicPermission: ((onResult: (Boolean) -> Unit) -> Unit)? = null

    fun startVoiceRecording() {
        if (recordingVoice) return
        val begin = {
            try {
                val dir = java.io.File(appContext.cacheDir, "outgoing")
                dir.mkdirs()
                val file = java.io.File(dir, "voice-${System.currentTimeMillis()}.ogg")
                @Suppress("DEPRECATION")
                val newRecorder = if (android.os.Build.VERSION.SDK_INT >= 31) {
                    android.media.MediaRecorder(appContext)
                } else {
                    android.media.MediaRecorder()
                }
                newRecorder.setAudioSource(android.media.MediaRecorder.AudioSource.MIC)
                // Ogg Opus, the codec the desktop app records voice in.
                newRecorder.setOutputFormat(android.media.MediaRecorder.OutputFormat.OGG)
                newRecorder.setAudioEncoder(android.media.MediaRecorder.AudioEncoder.OPUS)
                newRecorder.setAudioEncodingBitRate(64000)
                newRecorder.setAudioSamplingRate(48000)
                newRecorder.setOutputFile(file.absolutePath)
                newRecorder.prepare()
                newRecorder.start()
                recorder = newRecorder
                recordingFile = file
                recordingStarted = android.os.SystemClock.elapsedRealtime()
                recordingVoice = true
            } catch (_: Exception) {
                recorder = null
                recordingFile = null
            }
        }
        val ask = ensureMicPermission
        if (ask != null) {
            ask { granted -> if (granted) begin() }
        } else {
            begin()
        }
    }

    fun stopVoiceRecording(send: Boolean) {
        val activeRecorder = recorder ?: return
        val file = recordingFile
        val durationMs = android.os.SystemClock.elapsedRealtime() - recordingStarted
        recorder = null
        recordingFile = null
        recordingVoice = false
        try {
            activeRecorder.stop()
        } catch (_: Exception) {
            activeRecorder.release()
            file?.delete()
            return
        }
        activeRecorder.release()

        val room = openRoom
        if (!send || room == null || file == null || durationMs < 500) {
            file?.delete()
            return
        }
        thread {
            runBlocking {
                try {
                    app.sendVoiceMessage(
                        room.roomId,
                        file.absolutePath,
                        "audio/ogg",
                        durationMs.toULong(),
                    )
                } catch (_: Exception) {
                }
            }
        }
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
    /// Open the room with the given ID as soon as the list carries it —
    /// how a notification tap lands in its room.
    fun openRoomById(roomId: String) {
        // A notification names unread messages: land on the oldest one.
        openRoomWhenListed(roomId, toUnread = true)
    }

    private fun openRoomWhenListed(roomId: String, attempt: Int = 0, toUnread: Boolean = false) {
        val room = rooms.find { it.roomId == roomId }
        when {
            room != null -> openRoom(room, toUnread)
            attempt < 20 ->
                main.postDelayed({ openRoomWhenListed(roomId, attempt + 1, toUnread) }, 500)
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

    /// The stages of the one verification flow shown at a time.
    var verificationFlowId by mutableStateOf<String?>(null)
        private set
    var verificationUser by mutableStateOf<String?>(null)
        private set
    var verificationEmojis by mutableStateOf<List<FfiSasEmoji>>(emptyList())
        private set
    var verificationDone by mutableStateOf(false)
        private set
    var verificationOutgoing by mutableStateOf(false)
        private set

    /// Start following verification requests; harmless to call again.
    fun watchVerifications() {
        app.setVerificationListener(object : VerificationListener {
            override fun onRequest(flowId: String, userId: String) {
                main.post {
                    verificationFlowId = flowId
                    verificationUser = userId
                    verificationEmojis = emptyList()
                    verificationDone = false
                    verificationOutgoing = false
                }
            }

            override fun onEmojis(flowId: String, emojis: List<FfiSasEmoji>) {
                main.post {
                    if (verificationFlowId == flowId) verificationEmojis = emojis
                }
            }

            override fun onDone(flowId: String) {
                main.post {
                    if (verificationFlowId == flowId) verificationDone = true
                }
            }

            override fun onCancelled(flowId: String, reason: String) {
                main.post {
                    if (verificationFlowId == flowId) dismissVerification()
                }
            }
        })
    }

    /// Ask the account's other (verified) sessions to verify this one.
    fun requestVerification() {
        thread {
            runBlocking {
                try {
                    val flowId = app.requestVerification()
                    main.post {
                        verificationFlowId = flowId
                        verificationUser = null
                        verificationEmojis = emptyList()
                        verificationDone = false
                        verificationOutgoing = true
                    }
                } catch (_: Exception) {
                    // Nothing to wait for.
                }
            }
        }
    }

    /// Feed the scanned QR payload into the pending verification.
    fun submitScannedQr(data: ByteArray) {
        val flowId = verificationFlowId ?: return
        thread {
            runBlocking {
                try {
                    app.scanQr(flowId, data)
                } catch (_: Exception) {
                    // The listener reports the outcome either way.
                }
            }
        }
    }

    fun acceptVerification() {
        val flowId = verificationFlowId ?: return
        thread { runBlocking { app.acceptVerification(flowId) } }
    }

    fun confirmVerification() {
        val flowId = verificationFlowId ?: return
        thread { runBlocking { app.confirmVerification(flowId) } }
    }

    fun cancelVerification() {
        val flowId = verificationFlowId ?: return
        dismissVerification()
        thread { runBlocking { app.cancelVerification(flowId) } }
    }

    fun dismissVerification() {
        verificationFlowId = null
        verificationUser = null
        verificationEmojis = emptyList()
        verificationDone = false
        verificationOutgoing = false
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
                // The SDK settles its recovery state a moment after the
                // secrets arrive; look again until it does.
                for (delay in listOf(0L, 2000L, 5000L)) {
                    main.postDelayed({ refreshRecoveryState() }, delay)
                }
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

    fun openViewer(path: String, isVideo: Boolean = false) {
        viewerIsVideo = isVideo
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
        val mentions = composerMembers
            .filter { body.contains("@" + it.displayName) }
            .map { io.github.steeb_k.commune.core.FfiMention(it.userId, it.displayName) }
        setTyping(false)
        thread {
            runBlocking {
                try {
                    app.sendMessage(room.roomId, body, mentions)
                } catch (_: Exception) {
                    // The send queue retries; a failed-send surface comes
                    // with its own chunk.
                }
            }
        }
    }
}
