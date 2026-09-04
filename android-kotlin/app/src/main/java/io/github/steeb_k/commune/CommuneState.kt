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
    /// room notifies like any other. False until a window says otherwise:
    /// the process can now exist with no activity at all, woken by a push.
    var uiVisible: Boolean = false
        set(value) {
            field = value
            notifier.visibleRoomId = if (value) openRoom?.roomId else null
            // Coming back to the foreground is the moment the session's
            // connectivity knowledge went stale: the background froze the
            // process mid-claim, and the claim would otherwise be sleeping
            // out a backoff against a network that no longer exists.
            if (value) {
                app.recheckConnectivity()
                // On screen now, a pending verification shows as its sheet,
                // so its notification has done its job.
                if (verificationFlowId != null) VerificationNotification.dismiss(appContext)
            }
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
    /// Whether the room list has been delivered for the current
    /// session: an account with no rooms is loaded, not loading.
    var roomsLoaded by mutableStateOf(false)
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

    /// A Matrix link waiting on the person: a room to join or a user to
    /// chat with, as the GTK app's room preview and profile dialog ask.
    var pendingLink by mutableStateOf<io.github.steeb_k.commune.core.FfiMatrixLink?>(null)
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
                // The list arrived, so this session is ready — even
                // when it carries no rooms at all.
                roomsLoaded = true
                if (ownUserId == null) ownUserId = app.sessionUserId()
                if (settings == null) settings = app.sessionSettings()
                // The first delivery proves the session is ready; the
                // profile fetch and the encryption check at startup can
                // both have been too early.
                if (profileName == null) loadProfile()
                if (securityState == null) checkSessionSetup()
                notifier.enabled = settings?.notificationsEnabled != false
                notifier.update(rooms)
            }
        }
    }

    init {
        Native.seed(context.applicationContext)
        initCore(
            FfiCoreConfig(
                // The id this build actually has: the debug one carries a
                // .skeleton suffix and the release one does not. Hardcoding
                // the debug value meant every release build registered its
                // pusher under an application id no release build uses —
                // set_push_gateway retires that old registration.
                appId = context.packageName,
                // Only the Linux secret backend reads this; on Android it
                // is inert.
                profile = "skeleton",
                dataDir = context.noBackupFilesDir.resolve("commune").absolutePath,
                cacheDir = context.cacheDir.resolve("commune").absolutePath,
                // From the `communeKlipyApiKey` Gradle property, which is
                // empty unless the person building set one. Null rather than
                // empty so the core reads it the same way the desktop's
                // empty Meson option reads: the GIF search is not available.
                klipyApiKey = BuildConfig.KLIPY_API_KEY.ifEmpty { null },
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
                        watchCalls()
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
                    main.post { finishLogin() }
                } catch (failure: Exception) {
                    main.post {
                        loginBusy = false
                        loginError = failure.message ?: "Could not log in"
                    }
                }
            }
        }
    }

    // The post-login setup: where this session stands on encryption,
    // and the screen that offers the way out when it is not settled.
    var securityState by mutableStateOf<io.github.steeb_k.commune.core.FfiSecurityState?>(null)
        private set
    var setupNeeded by mutableStateOf(false)
        private set
    var setupBusy by mutableStateOf(false)
        private set
    var setupError by mutableStateOf<String?>(null)
        private set
    private var setupDismissed = false

    /// Read the encryption state and decide whether to ask, mirroring
    /// the application's session_setup_view: verified plus recovery
    /// enabled needs nothing, and an unknown state waits rather than
    /// nagging.
    fun checkSessionSetup() {
        thread {
            runBlocking {
                val security = try {
                    app.securityState()
                } catch (_: Exception) {
                    null
                }
                main.post {
                    securityState = security
                    val verified = security?.verification ==
                        io.github.steeb_k.commune.core.FfiVerificationState.VERIFIED
                    val recovered = security?.recovery ==
                        io.github.steeb_k.commune.core.FfiRecoveryState.ENABLED
                    val unknown = security == null ||
                        security.verification ==
                        io.github.steeb_k.commune.core.FfiVerificationState.UNKNOWN
                    setupNeeded = !(verified && recovered) && !unknown && !setupDismissed
                }
            }
        }
    }

    fun skipSessionSetup() {
        setupDismissed = true
        setupNeeded = false
        setupError = null
    }

    fun bootstrapCrossSigning(password: String) {
        setupBusy = true
        setupError = null
        thread {
            runBlocking {
                val error = try {
                    app.bootstrapCrossSigning(password)
                    null
                } catch (failure: Exception) {
                    coreMessage(failure, "Could not set up encryption")
                }
                main.post {
                    setupBusy = false
                    setupError = error
                    if (error == null) checkSessionSetup()
                }
            }
        }
    }

    /// The setup screen's recovery-key entry: the same recover() the
    /// settings page uses, with the setup check afterwards.
    fun submitRecoveryKey(key: String) {
        recover(key)
        for (delay in listOf(3000L, 6000L)) {
            main.postDelayed({ checkSessionSetup() }, delay)
        }
    }

    /// The tail of every successful login, whatever authenticated it.
    private fun finishLogin() {
        loginBusy = false
        addingAccount = false
        phase = Phase.Session
        rooms = emptyList()
        roomsLoaded = false
        ownUserId = null
        profileName = null
        profileAvatarPath = null
        settings = null
        savedScroll.clear()
        // The old listener task died with the old session.
        app.setRoomListListener(roomListListener)
        watchVerifications()
        watchCalls()
        loadProfile()
        refreshAccounts()
        setupDismissed = false
        checkSessionSetup()
    }

    // The account switcher: every session on the device, the active one
    // marked, switching re-arming the listeners like a login does.
    var accountSwitcherOpen by mutableStateOf(false)
        private set
    var accounts by mutableStateOf<List<io.github.steeb_k.commune.core.FfiSessionInfo>>(
        emptyList()
    )
        private set
    var addingAccount by mutableStateOf(false)
        private set

    /// Forget a session that could not be restored — the GTK account
    /// switcher's way out of a broken one.
    fun removeSession(sessionId: String) {
        thread {
            runBlocking {
                try {
                    app.removeSession(sessionId)
                } catch (_: Exception) {
                }
            }
            refreshAccounts()
        }
    }

    fun refreshAccounts() {
        thread {
            val list = app.sessions()
            main.post {
                accounts = list
                // Notification bookkeeping is per-account.
                notifier.accountKey = list.find { it.active }?.sessionId.orEmpty()
            }
        }
    }

    fun openAccountSwitcher() {
        refreshAccounts()
        accountSwitcherOpen = true
    }

    fun closeAccountSwitcher() {
        accountSwitcherOpen = false
    }

    fun switchAccount(sessionId: String) {
        closeAccountSwitcher()
        closeRoom()
        app.setActiveSession(sessionId)
        // Everything session-scoped belongs to the account that was
        // active; the new one's arrives with its first room list.
        rooms = emptyList()
        roomsLoaded = false
        ownUserId = null
        profileName = null
        profileAvatarPath = null
        settings = null
        savedScroll.clear()
        app.setRoomListListener(roomListListener)
        watchVerifications()
        watchCalls()
        loadProfile()
        refreshAccounts()
        setupDismissed = false
        securityState = null
        setupNeeded = false
        checkSessionSetup()
    }

    fun startAddAccount() {
        closeAccountSwitcher()
        addingAccount = true
    }

    fun cancelAddAccount() {
        addingAccount = false
    }

    /// What the homeserver said it offers, after discovery.
    var loginMethods by mutableStateOf<io.github.steeb_k.commune.core.FfiLoginMethods?>(null)
        private set

    fun discoverLogin(homeserver: String, onDone: (Boolean) -> Unit) {
        loginBusy = true
        loginError = null
        thread {
            runBlocking {
                try {
                    val methods = app.discoverLogin(homeserver)
                    main.post {
                        loginBusy = false
                        loginMethods = methods
                        onDone(true)
                    }
                } catch (failure: Exception) {
                    main.post {
                        loginBusy = false
                        loginError = coreMessage(failure, "Could not reach the homeserver")
                        onDone(false)
                    }
                }
            }
        }
    }

    /// Open the SSO (or OAuth) page in the browser; the redirect comes
    /// back through the app's custom scheme.
    fun startBrowserLogin(oauth: Boolean) {
        thread {
            runBlocking {
                try {
                    val url = if (oauth) app.oauthLoginUrl() else app.ssoLoginUrl()
                    main.post {
                        val intent = android.content.Intent(
                            android.content.Intent.ACTION_VIEW,
                            android.net.Uri.parse(url),
                        ).addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK)
                        appContext.startActivity(intent)
                    }
                } catch (e: Exception) {
                    toast(coreMessage(e, "Could not set up login"))
                }
            }
        }
    }

    /// A redirect landed on the app's login scheme: a Matrix SSO login
    /// token, or an OAuth authorization response.
    fun handleLoginRedirect(uri: android.net.Uri) {
        val loginToken = uri.getQueryParameter("loginToken")
        loginBusy = true
        loginError = null
        thread {
            runBlocking {
                try {
                    if (loginToken != null) {
                        app.finishSsoLogin(loginToken)
                    } else {
                        app.finishOauthLogin(uri.encodedQuery.orEmpty())
                    }
                    main.post { finishLogin() }
                } catch (failure: Exception) {
                    main.post {
                        loginBusy = false
                        loginError = coreMessage(
                            failure,
                            "Could not log in",
                        )
                    }
                }
            }
        }
    }

    fun register(username: String, password: String) {
        loginBusy = true
        loginError = null
        thread {
            runBlocking {
                try {
                    app.registerUser(username, password)
                    main.post { finishLogin() }
                } catch (failure: Exception) {
                    main.post {
                        loginBusy = false
                        loginError = coreMessage(failure, "Could not create account")
                    }
                }
            }
        }
    }

    /// The reset flow's server-side session, once the email was asked.
    var resetHandle by mutableStateOf<io.github.steeb_k.commune.core.FfiResetHandle?>(null)
        private set

    fun requestPasswordReset(email: String, onDone: (String?) -> Unit) {
        thread {
            runBlocking {
                val error = try {
                    val handle = app.requestPasswordReset(email)
                    main.post { resetHandle = handle }
                    null
                } catch (failure: Exception) {
                    coreMessage(failure, "Could not send the email")
                }
                main.post { onDone(error) }
            }
        }
    }

    fun resetPassword(newPassword: String, onDone: (String?) -> Unit) {
        val handle = resetHandle ?: return
        thread {
            runBlocking {
                val error = try {
                    app.resetPassword(newPassword, handle)
                    null
                } catch (failure: Exception) {
                    coreMessage(failure, "Could not reset the password")
                }
                main.post {
                    if (error == null) resetHandle = null
                    onDone(error)
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
        loadComposerEmoticons()
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
        clearSelection()
        closeEventSource()
        closeAddresses()
        closeServerAcl()
        closePermissions()
        joinRuleInfo = null
        historyVisibilityInfo = null
        upgradeInfo = null
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

    /// A brief user-facing notice — refusals the timeline cannot show,
    /// like the upload-size preflight turning a file down.
    fun toast(message: String) {
        main.post {
            android.widget.Toast
                .makeText(appContext, message, android.widget.Toast.LENGTH_LONG)
                .show()
        }
    }

    private fun coreMessage(error: Exception, fallback: String): String =
        (error as? io.github.steeb_k.commune.core.CoreException.Failed)?.msg ?: fallback

    // The event menu extras: link, source, report, forward, discard,
    // save, and the selection mode.
    fun copyEventLink(eventId: String) {
        val room = openRoom ?: return
        thread {
            runBlocking {
                try {
                    val link = app.eventPermalink(room.roomId, eventId)
                    main.post {
                        val clipboard = appContext
                            .getSystemService(android.content.ClipboardManager::class.java)
                        clipboard.setPrimaryClip(
                            android.content.ClipData.newPlainText("Message link", link)
                        )
                        toast("Message link copied to clipboard")
                    }
                } catch (e: Exception) {
                    toast(coreMessage(e, "Could not build the link"))
                }
            }
        }
    }

    /// The raw JSON on show in the properties dialog; null when closed.
    var eventSource by mutableStateOf<String?>(null)
        private set

    fun openEventSource(eventId: String) {
        val room = openRoom ?: return
        eventSource = "Loading…"
        thread {
            runBlocking {
                val source = try {
                    app.eventSource(room.roomId, eventId)
                } catch (e: Exception) {
                    coreMessage(e, "Could not fetch the event")
                }
                main.post { if (eventSource != null) eventSource = source }
            }
        }
    }

    fun closeEventSource() {
        eventSource = null
    }

    fun reportEvent(eventId: String, reason: String, onDone: (String?) -> Unit) {
        val room = openRoom ?: return
        thread {
            runBlocking {
                val error = try {
                    app.reportEvent(room.roomId, eventId, reason.ifBlank { null })
                    null
                } catch (e: Exception) {
                    coreMessage(e, "Could not report the event")
                }
                main.post { onDone(error) }
            }
        }
    }

    fun forwardEvent(eventId: String, targetRoomId: String) {
        val room = openRoom ?: return
        thread {
            runBlocking {
                try {
                    app.forwardEvent(room.roomId, eventId, targetRoomId)
                    toast("Message forwarded")
                } catch (e: Exception) {
                    toast(coreMessage(e, "Could not forward the message"))
                }
            }
        }
    }

    fun discardEcho(uniqueId: String) {
        val room = openRoom ?: return
        thread {
            runBlocking {
                try {
                    app.discardLocalEcho(room.roomId, uniqueId)
                } catch (e: Exception) {
                    toast(coreMessage(e, "Could not discard the message"))
                }
            }
        }
    }

    fun saveEventMedia(event: FfiTimelineItem.Event) {
        val room = openRoom ?: return
        thread {
            runBlocking {
                beginSave(event.body, 0, 1)
                val path = try {
                    app.getTimelineMedia(room.roomId, event.uniqueId)
                } catch (_: Exception) {
                    null
                }
                val saved = path != null && saveToDownloads(path, event.body, null)
                endSave()
                if (saved) {
                    toast("Saved to Downloads")
                } else {
                    toast("Could not save the file")
                }
            }
        }
    }

    /// Where a save to Downloads stands: how many files are done, how
    /// many there are, and which one is being fetched. Null when nothing
    /// is saving. A single file counts as one of one, and its bar moves
    /// without a fraction, so a long fetch is visibly alive.
    data class SaveProgress(val done: Int, val total: Int, val name: String)

    var saveProgress by mutableStateOf<SaveProgress?>(null)
        private set

    @Volatile
    private var saveCancelled = false

    /// Stop a batch after the file in flight; that one cannot be cut short.
    fun cancelSave() {
        saveCancelled = true
    }

    private fun beginSave(name: String, done: Int, total: Int) {
        main.post { saveProgress = SaveProgress(done, total, name) }
    }

    private fun endSave() {
        main.post { saveProgress = null }
    }

    /// The timeline's selection mode: the set of selected unique IDs.
    var selectedIds by mutableStateOf<Set<String>>(emptySet())
        private set
    var selectMode by mutableStateOf(false)
        private set

    fun startSelection(uniqueId: String) {
        selectMode = true
        selectedIds = setOf(uniqueId)
    }

    fun toggleSelected(uniqueId: String) {
        selectedIds = if (uniqueId in selectedIds) {
            selectedIds - uniqueId
        } else {
            selectedIds + uniqueId
        }
        if (selectedIds.isEmpty()) selectMode = false
    }

    fun clearSelection() {
        selectMode = false
        selectedIds = emptySet()
    }

    /// The selected events, in timeline order.
    fun selectedEvents(): List<FfiTimelineItem.Event> = timeline
        .filterIsInstance<FfiTimelineItem.Event>()
        .filter { it.uniqueId in selectedIds }

    fun copySelectedText() {
        val text = selectedEvents().joinToString("\n") { it.body }
        val clipboard = appContext
            .getSystemService(android.content.ClipboardManager::class.java)
        clipboard.setPrimaryClip(
            android.content.ClipData.newPlainText("Messages", text)
        )
        toast("Copied")
        clearSelection()
    }

    fun removeSelected() {
        val ids = selectedEvents().mapNotNull { it.eventId }
        clearSelection()
        thread {
            runBlocking {
                for (eventId in ids) {
                    try {
                        app.redactEvent(openRoom?.roomId ?: return@runBlocking, eventId)
                    } catch (e: Exception) {
                        toast(coreMessage(e, "Could not remove a message"))
                    }
                }
            }
        }
    }

    fun forwardSelected(targetRoomId: String) {
        val room = openRoom ?: return
        val ids = selectedEvents().mapNotNull { it.eventId }
        clearSelection()
        thread {
            runBlocking {
                for (eventId in ids) {
                    try {
                        app.forwardEvent(room.roomId, eventId, targetRoomId)
                    } catch (e: Exception) {
                        toast(coreMessage(e, "Could not forward a message"))
                    }
                }
                toast("Forwarded")
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
                } catch (e: Exception) {
                    toast(coreMessage(e, "Could not send the GIF"))
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

    /// The pictures video events carry, by event ID, once fetched; an
    /// event without one is remembered as absent so it is not asked twice.
    val historyThumbnails = mutableStateMapOf<String, String?>()

    fun fetchHistoryThumbnail(eventId: String) {
        val room = openRoom ?: return
        if (historyThumbnails.containsKey(eventId)) return
        thread {
            runBlocking {
                val path = try {
                    app.getHistoryMediaThumbnail(room.roomId, eventId, 360u)
                } catch (_: Exception) {
                    null
                }
                main.post { historyThumbnails[eventId] = path }
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
            saveCancelled = false
            runBlocking {
                for ((index, event) in events.withIndex()) {
                    if (saveCancelled) break
                    beginSave(event.body, index, events.size)
                    val path = historyMedia[event.eventId] ?: try {
                        app.getHistoryMedia(room.roomId, event.eventId)
                    } catch (_: Exception) {
                        null
                    } ?: continue
                    if (saveToDownloads(path, event.body, event.mimeType)) saved += 1
                }
            }
            endSave()
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

    /// Set by the activity: opens the image picker for a new room avatar.
    var pickRoomAvatar: (() -> Unit)? = null

    fun setRoomAvatarFromUri(uri: android.net.Uri) {
        val room = openRoom ?: return
        val resolver = appContext.contentResolver
        val mime = resolver.getType(uri) ?: "image/jpeg"
        thread {
            try {
                val dir = java.io.File(appContext.cacheDir, "outgoing")
                dir.mkdirs()
                val file = java.io.File(dir, "room-avatar")
                resolver.openInputStream(uri)?.use { input ->
                    file.outputStream().use { output -> input.copyTo(output) }
                } ?: return@thread
                // The event's info carries the picture's dimensions.
                val bounds = android.graphics.BitmapFactory.Options()
                    .apply { inJustDecodeBounds = true }
                android.graphics.BitmapFactory.decodeFile(file.absolutePath, bounds)
                runBlocking {
                    app.setRoomAvatar(
                        room.roomId,
                        file.absolutePath,
                        mime,
                        bounds.outWidth.takeIf { it > 0 }?.toUInt(),
                        bounds.outHeight.takeIf { it > 0 }?.toUInt(),
                    )
                }
            } catch (e: Exception) {
                toast(coreMessage(e, "Could not change the avatar"))
            }
        }
    }

    fun removeRoomAvatar() {
        val room = openRoom ?: return
        thread {
            runBlocking {
                try {
                    app.removeRoomAvatar(room.roomId)
                } catch (e: Exception) {
                    toast(coreMessage(e, "Could not remove the avatar"))
                }
            }
        }
    }

    // The room-settings subpages and dialogs under room details.
    var joinRuleInfo by mutableStateOf<io.github.steeb_k.commune.core.FfiJoinRuleInfo?>(null)
        private set

    fun loadJoinRule() {
        val room = openRoom ?: return
        thread {
            runBlocking {
                val info = try {
                    app.roomJoinRule(room.roomId)
                } catch (_: Exception) {
                    null
                }
                main.post { joinRuleInfo = info }
            }
        }
    }

    fun setJoinRule(
        value: io.github.steeb_k.commune.core.FfiJoinRuleValue,
        allowSpaceId: String?,
        onDone: (String?) -> Unit,
    ) {
        val room = openRoom ?: return
        thread {
            runBlocking {
                val error = try {
                    app.setRoomJoinRule(room.roomId, value, allowSpaceId)
                    null
                } catch (e: Exception) {
                    coreMessage(e, "Could not change who can join")
                }
                main.post {
                    onDone(error)
                    if (error == null) loadJoinRule()
                }
            }
        }
    }

    var historyVisibilityInfo by mutableStateOf<
        io.github.steeb_k.commune.core.FfiHistoryVisibilityInfo?,
    >(null)
        private set

    fun loadHistoryVisibility() {
        val room = openRoom ?: return
        thread {
            runBlocking {
                val info = try {
                    app.roomHistoryVisibility(room.roomId)
                } catch (_: Exception) {
                    null
                }
                main.post { historyVisibilityInfo = info }
            }
        }
    }

    fun setHistoryVisibility(
        value: io.github.steeb_k.commune.core.FfiHistoryVisibility,
        onDone: (String?) -> Unit,
    ) {
        val room = openRoom ?: return
        thread {
            runBlocking {
                val error = try {
                    app.setRoomHistoryVisibility(room.roomId, value)
                    null
                } catch (e: Exception) {
                    coreMessage(e, "Could not change the history visibility")
                }
                main.post {
                    onDone(error)
                    if (error == null) loadHistoryVisibility()
                }
            }
        }
    }

    var addressesOpen by mutableStateOf(false)
        private set
    var addresses by mutableStateOf<io.github.steeb_k.commune.core.FfiRoomAddresses?>(null)
        private set

    fun openAddresses() {
        addressesOpen = true
        refreshAddresses()
    }

    fun closeAddresses() {
        addressesOpen = false
        addresses = null
    }

    fun refreshAddresses() {
        val room = openRoom ?: return
        thread {
            runBlocking {
                val current = try {
                    app.roomAddresses(room.roomId)
                } catch (_: Exception) {
                    null
                }
                main.post { if (addressesOpen) addresses = current }
            }
        }
    }

    fun addressAction(action: io.github.steeb_k.commune.core.FfiAddressAction) {
        val room = openRoom ?: return
        thread {
            runBlocking {
                try {
                    app.setRoomAddress(room.roomId, action)
                } catch (e: Exception) {
                    toast(coreMessage(e, "Could not change the addresses"))
                }
                main.post { refreshAddresses() }
            }
        }
    }

    var serverAclOpen by mutableStateOf(false)
        private set
    var serverAcl by mutableStateOf<io.github.steeb_k.commune.core.FfiServerAcl?>(null)
        private set

    fun openServerAcl() {
        serverAclOpen = true
        val room = openRoom ?: return
        thread {
            runBlocking {
                val current = try {
                    app.roomServerAcl(room.roomId)
                } catch (_: Exception) {
                    null
                }
                main.post { if (serverAclOpen) serverAcl = current }
            }
        }
    }

    fun closeServerAcl() {
        serverAclOpen = false
        serverAcl = null
    }

    fun saveServerAcl(
        allow: List<String>,
        deny: List<String>,
        allowIpLiterals: Boolean,
        onDone: (String?) -> Unit,
    ) {
        val room = openRoom ?: return
        thread {
            runBlocking {
                val error = try {
                    app.setRoomServerAcl(room.roomId, allow, deny, allowIpLiterals)
                    null
                } catch (e: Exception) {
                    coreMessage(e, "Could not change the server ACL")
                }
                main.post { onDone(error) }
            }
        }
    }

    var permissionsOpen by mutableStateOf(false)
        private set
    var permissionsMatrix by mutableStateOf<
        io.github.steeb_k.commune.core.FfiPowerLevelsMatrix?,
    >(null)
        private set

    fun openPermissions() {
        permissionsOpen = true
        val room = openRoom ?: return
        thread {
            runBlocking {
                val current = try {
                    app.roomPermissionsMatrix(room.roomId)
                } catch (_: Exception) {
                    null
                }
                main.post { if (permissionsOpen) permissionsMatrix = current }
            }
        }
    }

    fun closePermissions() {
        permissionsOpen = false
        permissionsMatrix = null
    }

    fun savePermissions(
        matrix: io.github.steeb_k.commune.core.FfiPowerLevelsMatrix,
        onDone: (String?) -> Unit,
    ) {
        val room = openRoom ?: return
        thread {
            runBlocking {
                val error = try {
                    app.setRoomPermissionsMatrix(room.roomId, matrix)
                    null
                } catch (e: Exception) {
                    coreMessage(e, "Could not save the permissions")
                }
                main.post { onDone(error) }
            }
        }
    }

    var upgradeInfo by mutableStateOf<io.github.steeb_k.commune.core.FfiUpgradeInfo?>(null)
        private set

    fun loadUpgradeInfo() {
        val room = openRoom ?: return
        upgradeInfo = null
        thread {
            runBlocking {
                val info = try {
                    app.roomUpgradeInfo(room.roomId)
                } catch (_: Exception) {
                    null
                }
                main.post { upgradeInfo = info }
            }
        }
    }

    fun upgradeRoom(version: String, onDone: (String?) -> Unit) {
        val room = openRoom ?: return
        thread {
            runBlocking {
                val error = try {
                    app.upgradeRoom(room.roomId, version)
                    null
                } catch (e: Exception) {
                    coreMessage(e, "Could not upgrade the room")
                }
                main.post { onDone(error) }
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
        isSpace: Boolean,
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
                        isSpace,
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

    // The users the account ignores, and the safety subpage showing them.
    var ignoredUsersOpen by mutableStateOf(false)
        private set
    var ignoredUsers by mutableStateOf<List<String>>(emptyList())
        private set

    fun openIgnoredUsers() {
        ignoredUsersOpen = true
        refreshIgnoredUsers()
    }

    fun closeIgnoredUsers() {
        ignoredUsersOpen = false
    }

    fun refreshIgnoredUsers() {
        thread {
            runBlocking {
                val list = try {
                    app.ignoredUsers()
                } catch (_: Exception) {
                    emptyList()
                }
                main.post { ignoredUsers = list }
            }
        }
    }

    fun ignoreUser(userId: String) {
        thread {
            runBlocking {
                try {
                    app.ignoreUser(userId)
                } catch (e: Exception) {
                    toast(coreMessage(e, "Could not ignore the user"))
                }
                main.post { refreshIgnoredUsers() }
            }
        }
    }

    fun unignoreUser(userId: String) {
        thread {
            runBlocking {
                try {
                    app.unignoreUser(userId)
                } catch (e: Exception) {
                    toast(coreMessage(e, "Could not stop ignoring the user"))
                }
                main.post { refreshIgnoredUsers() }
            }
        }
    }

    // The keywords that trigger notifications.
    var notificationKeywords by mutableStateOf<List<String>>(emptyList())
        private set

    fun loadNotificationKeywords() {
        thread {
            runBlocking {
                val list = try {
                    app.notificationKeywords()
                } catch (_: Exception) {
                    emptyList()
                }
                main.post { notificationKeywords = list }
            }
        }
    }

    fun addNotificationKeyword(keyword: String) {
        thread {
            runBlocking {
                try {
                    val list = app.addNotificationKeyword(keyword)
                    main.post { notificationKeywords = list }
                } catch (e: Exception) {
                    toast(coreMessage(e, "Could not add the keyword"))
                }
            }
        }
    }

    fun removeNotificationKeyword(keyword: String) {
        thread {
            runBlocking {
                try {
                    val list = app.removeNotificationKeyword(keyword)
                    main.post { notificationKeywords = list }
                } catch (e: Exception) {
                    toast(coreMessage(e, "Could not remove the keyword"))
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
                } catch (e: Exception) {
                    toast(coreMessage(e, "Could not send the voice message"))
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

    /// Pin the event, or unpin it if it is pinned.
    fun togglePin(event: FfiTimelineItem.Event, onDone: (String?) -> Unit) {
        val eventId = event.eventId ?: return
        moderate(onDone) {
            if (event.isPinned) app.unpinEvent(it, eventId) else app.pinEvent(it, eventId)
        }
    }

    /// Enable encryption in the open room. It cannot be disabled later.
    fun enableEncryption(onDone: (String?) -> Unit) {
        moderate(onDone) { app.enableRoomEncryption(it) }
    }

    /// Lift a ban in the open room.
    fun unbanUser(userId: String, onDone: (String?) -> Unit) {
        moderate(onDone) { app.unbanUser(it, userId, null) }
    }

    /// The joined spaces holding the open room.
    fun parentSpaces(onDone: (List<FfiRoom>) -> Unit) {
        val room = openRoom ?: return
        thread {
            runBlocking {
                val spaces = try {
                    app.parentSpaces(room.roomId)
                } catch (_: Exception) {
                    emptyList()
                }
                main.post { onDone(spaces) }
            }
        }
    }

    /// Put the open room inside the given space.
    fun addRoomToSpace(spaceId: String, onDone: (String?) -> Unit) {
        moderate(onDone) { app.addRoomToSpace(it, spaceId) }
    }

    /// Take the open room out of the given space.
    fun removeRoomFromSpace(spaceId: String, onDone: (String?) -> Unit) {
        moderate(onDone) { app.removeRoomFromSpace(it, spaceId) }
    }

    /// Reset the crypto identity, the password answering the homeserver.
    fun resetCrossSigning(password: String, onDone: (String?) -> Unit) {
        thread {
            runBlocking {
                val error = try {
                    app.resetCrossSigning(password)
                    null
                } catch (failure: Exception) {
                    failure.message?.removePrefix("msg=") ?: "Could not reset the crypto identity"
                }
                main.post { onDone(error) }
            }
        }
    }

    /// Change the account's password; the current one answers the
    /// homeserver's password stage.
    fun changePassword(newPassword: String, currentPassword: String, onDone: (String?) -> Unit) {
        thread {
            runBlocking {
                val error = try {
                    app.changePassword(newPassword, currentPassword)
                    null
                } catch (failure: Exception) {
                    failure.message?.removePrefix("msg=") ?: "Could not change password"
                }
                main.post { onDone(error) }
            }
        }
    }

    /// Deactivate the account, then leave the session the way a logout
    /// does: the homeserver has already forgotten it.
    fun deactivateAccount(currentPassword: String, onDone: (String?) -> Unit) {
        thread {
            runBlocking {
                val error = try {
                    app.deactivateAccount(currentPassword)
                    null
                } catch (failure: Exception) {
                    failure.message?.removePrefix("msg=") ?: "Could not deactivate account"
                }
                main.post {
                    onDone(error)
                    if (error == null) {
                        toast("Account successfully deactivated")
                        PushManager.setMode(appContext, PushManager.MODE_UNSET)
                        pushMode = PushManager.MODE_UNSET
                        appContext.stopService(
                            android.content.Intent(appContext, SyncService::class.java)
                        )
                        openRoom = null
                        rooms = emptyList()
                        timeline = emptyList()
                        settingsOpen = false
                        ownUserId = null
                        phase = Phase.Login
                    }
                }
            }
        }
    }

    /// The email addresses and phone numbers on the account.
    fun thirdPartyIds(
        onDone: (io.github.steeb_k.commune.core.FfiThirdPartyIds?, String?) -> Unit,
    ) {
        thread {
            runBlocking {
                val result = try {
                    Pair(app.thirdPartyIds(), null)
                } catch (failure: Exception) {
                    Pair(null, failure.message?.removePrefix("msg=") ?: "Could not load the addresses")
                }
                main.post { onDone(result.first, result.second) }
            }
        }
    }

    /// Remove an address from the account.
    fun deleteThirdPartyId(address: String, isEmail: Boolean, onDone: (String?) -> Unit) {
        thread {
            runBlocking {
                val error = try {
                    app.deleteThirdPartyId(address, isEmail)
                    null
                } catch (failure: Exception) {
                    failure.message?.removePrefix("msg=") ?: "Could not remove address"
                }
                main.post { onDone(error) }
            }
        }
    }

    /// Ask for the validation link of an email address; passing the
    /// previous answer resends it.
    fun requestEmailValidation(
        address: String,
        previous: io.github.steeb_k.commune.core.FfiPendingEmail?,
        onDone: (io.github.steeb_k.commune.core.FfiPendingEmail?, String?) -> Unit,
    ) {
        thread {
            runBlocking {
                val result = try {
                    Pair(app.requestEmailValidation(address, previous), null)
                } catch (failure: Exception) {
                    Pair(null, failure.message?.removePrefix("msg=") ?: "Could not send the validation email")
                }
                main.post { onDone(result.first, result.second) }
            }
        }
    }

    /// Add the validated email address, the current password answering
    /// the homeserver's password stage.
    fun addPendingEmail(
        pending: io.github.steeb_k.commune.core.FfiPendingEmail,
        currentPassword: String,
        onDone: (String?) -> Unit,
    ) {
        thread {
            runBlocking {
                val error = try {
                    app.addPendingEmail(pending, currentPassword)
                    null
                } catch (failure: Exception) {
                    failure.message?.removePrefix("msg=") ?: "Could not add the address"
                }
                main.post { onDone(error) }
            }
        }
    }

    /// Add the loaded messages of the open room to its search index, then
    /// search again — how an encrypted room's history becomes searchable.
    fun reindexRoomSearch(query: String) {
        val room = openRoom ?: return
        roomSearchBusy = true
        thread {
            runBlocking {
                try {
                    app.reindexRoomSearch(room.roomId)
                } catch (_: Exception) {
                }
                main.post {
                    if (query.isNotBlank()) searchRoom(query) else roomSearchBusy = false
                }
            }
        }
    }

    /// Open what the given Matrix link points at: a room we are in opens
    /// in place; anything else waits on the person in a dialog. Returns
    /// `false` if the string is not a Matrix link at all.
    fun openMatrixLink(uri: String): Boolean {
        val link = io.github.steeb_k.commune.core.parseMatrixLink(uri) ?: return false

        if (link is io.github.steeb_k.commune.core.FfiMatrixLink.Room) {
            val room = rooms.find { it.roomId == link.roomIdOrAlias }
            if (room != null) {
                closeSettings()
                closeImagePacks()
                closeIgnoredUsers()
                closeDevices()
                closeExplore()
                closeAccountSwitcher()
                pendingLink = null
                openRoom(room)
                return true
            }
        }

        pendingLink = link
        return true
    }

    /// Put the pending Matrix link away.
    fun dismissLink() {
        pendingLink = null
        clearConversationError()
    }

    /// Search the user directory for the given term, as the GTK app's
    /// invite page and direct chat dialog do while one types.
    fun searchUsers(
        term: String,
        onResult: (List<io.github.steeb_k.commune.core.FfiUserSearchResult>) -> Unit,
    ) {
        thread {
            runBlocking {
                val results = try {
                    app.searchUsers(term, 10u)
                } catch (_: Exception) {
                    emptyList()
                }
                main.post { onResult(results) }
            }
        }
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
        // A notification tap outranks whatever page happens to be open:
        // the routing chain is first-match, so those have to close.
        closeSettings()
        closeImagePacks()
        closeIgnoredUsers()
        closeDevices()
        closeExplore()
        closeAccountSwitcher()
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
        queueAttachmentsFromUris(listOf(uri))
    }

    /// Copy the picked files under the cache and queue them for the open
    /// room: the first is previewed, the rest wait behind it, each to be
    /// its own message in the order picked — the GTK toolbar's batch.
    fun queueAttachmentsFromUris(uris: List<android.net.Uri>) {
        if (openRoom == null) return
        val resolver = appContext.contentResolver

        thread {
            val picked = mutableListOf<PendingAttachment>()
            for (uri in uris) {
                try {
                    val mime = resolver.getType(uri) ?: "application/octet-stream"
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
                    // Two picks with one name must not overwrite each other
                    // while both wait in the queue.
                    var file = java.io.File(dir, name)
                    var attempt = 1
                    while (file.exists() || picked.any { it.path == file.absolutePath }) {
                        file = java.io.File(dir, "${attempt++}-$name")
                    }
                    resolver.openInputStream(uri)?.use { input ->
                        file.outputStream().use { output -> input.copyTo(output) }
                    } ?: continue

                    picked += PendingAttachment(
                        path = file.absolutePath,
                        name = name,
                        mime = mime,
                        size = file.length(),
                    )
                } catch (e: Exception) {
                    toast(coreMessage(e, "Could not read the file"))
                }
            }

            // Show the preview first; sending is its confirm.
            main.post {
                attachmentQueue = attachmentQueue + picked
                pendingAttachment = attachmentQueue.firstOrNull()
            }
        }
    }

    /// Files shared from another app, waiting for a room when none is open.
    var pendingShare by mutableStateOf<List<android.net.Uri>>(emptyList())
        private set

    /// Take files shared from another app: into the open room, or into the
    /// room picked next.
    fun receiveShare(uris: List<android.net.Uri>) {
        if (openRoom != null) {
            queueAttachmentsFromUris(uris)
        } else {
            pendingShare = uris
        }
    }

    /// The room the shared files go to.
    fun shareTo(roomId: String) {
        val uris = pendingShare
        pendingShare = emptyList()
        val room = rooms.find { it.roomId == roomId } ?: return
        openRoom(room)
        queueAttachmentsFromUris(uris)
    }

    fun cancelShare() {
        pendingShare = emptyList()
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

    /// Whether we accepted the request and wait for the other side to
    /// pick a method — when our QR code is worth showing.
    var verificationAccepted by mutableStateOf(false)
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
                    verificationAccepted = false
                    // The sheet is already showing when the app is on
                    // screen; when it is not, this is the only sign the
                    // request arrived, the way the desktop posts one.
                    if (!uiVisible) {
                        VerificationNotification.show(appContext, userId)
                    }
                }
            }

            override fun onEmojis(flowId: String, emojis: List<FfiSasEmoji>) {
                main.post {
                    if (verificationFlowId == flowId) verificationEmojis = emojis
                }
            }

            override fun onDone(flowId: String) {
                main.post {
                    if (verificationFlowId == flowId) {
                        verificationDone = true
                        VerificationNotification.dismiss(appContext)
                    }
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
    fun requestUserVerification(userId: String) {
        thread {
            runBlocking {
                try {
                    val flowId = app.requestUserVerification(userId)
                    main.post {
                        verificationFlowId = flowId
                        verificationUser = userId
                        verificationEmojis = emptyList()
                        verificationDone = false
                        verificationOutgoing = true
                    }
                } catch (e: Exception) {
                    toast(coreMessage(e, "Could not request verification"))
                }
            }
        }
    }

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
        verificationAccepted = true
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
        VerificationNotification.dismiss(appContext)
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
    // The account's own image packs, and their editing.
    var imagePacksOpen by mutableStateOf(false)
        private set
    var ownedPacks by mutableStateOf<List<io.github.steeb_k.commune.core.FfiOwnedPack>>(
        emptyList()
    )
        private set
    var packsError by mutableStateOf<String?>(null)
        private set

    /// Set by the activity: opens the image picker for a pack image.
    var pickImagePackFile: (() -> Unit)? = null

    /// The picked file waiting for the shortcode that names it.
    var pendingPackImage by mutableStateOf<PendingPackImage?>(null)
        private set

    data class PendingPackImage(
        val stateKey: String,
        val path: String,
        val mime: String,
        val name: String,
    )

    private var packTarget: String? = null

    fun openImagePacks() {
        imagePacksOpen = true
        loadOwnedPacks()
    }

    fun closeImagePacks() {
        imagePacksOpen = false
        packsError = null
    }

    fun loadOwnedPacks() {
        thread {
            runBlocking {
                val packs = try {
                    app.myImagePacks()
                } catch (_: Exception) {
                    emptyList()
                }
                main.post { ownedPacks = packs }
            }
        }
    }

    /// Reload after an edit: the state event we just sent reaches the
    /// local store through sync, a moment after the send returns.
    private fun reloadPacksSoon() {
        loadOwnedPacks()
        for (delay in listOf(1500L, 4000L)) {
            main.postDelayed({ if (imagePacksOpen) loadOwnedPacks() }, delay)
        }
    }

    fun createImagePack(name: String) {
        thread {
            runBlocking {
                val error = try {
                    app.createImagePack(name)
                    null
                } catch (failure: Exception) {
                    coreMessage(failure, "Could not create the pack")
                }
                main.post {
                    packsError = error
                    reloadPacksSoon()
                }
            }
        }
    }

    fun renameImagePack(stateKey: String, name: String) {
        thread {
            runBlocking {
                val error = try {
                    app.renameImagePack(stateKey, name)
                    null
                } catch (failure: Exception) {
                    coreMessage(failure, "Could not rename the pack")
                }
                main.post {
                    packsError = error
                    reloadPacksSoon()
                }
            }
        }
    }

    fun deleteImagePack(stateKey: String) {
        thread {
            runBlocking {
                val error = try {
                    app.deleteImagePack(stateKey)
                    null
                } catch (failure: Exception) {
                    coreMessage(failure, "Could not delete the pack")
                }
                main.post {
                    packsError = error
                    reloadPacksSoon()
                }
            }
        }
    }

    fun removePackImage(stateKey: String, shortcode: String) {
        thread {
            runBlocking {
                val error = try {
                    app.removePackImage(stateKey, shortcode)
                    null
                } catch (failure: Exception) {
                    coreMessage(failure, "Could not remove the image")
                }
                main.post {
                    packsError = error
                    reloadPacksSoon()
                }
            }
        }
    }

    /// Pick a picture for the given pack; the shortcode is asked for
    /// once the file is in hand.
    fun pickPackImage(stateKey: String) {
        packTarget = stateKey
        pickImagePackFile?.invoke()
    }

    fun packImagePicked(uri: android.net.Uri) {
        val stateKey = packTarget ?: return
        val resolver = appContext.contentResolver
        val mime = resolver.getType(uri) ?: "image/png"
        thread {
            try {
                var name = "image"
                resolver.query(uri, null, null, null, null)?.use { cursor ->
                    val index =
                        cursor.getColumnIndex(android.provider.OpenableColumns.DISPLAY_NAME)
                    if (index >= 0 && cursor.moveToFirst()) {
                        name = cursor.getString(index) ?: name
                    }
                }
                val dir = java.io.File(appContext.cacheDir, "outgoing")
                dir.mkdirs()
                val file = java.io.File(dir, "pack-image")
                resolver.openInputStream(uri)?.use { input ->
                    file.outputStream().use { output -> input.copyTo(output) }
                } ?: return@thread
                main.post {
                    pendingPackImage = PendingPackImage(
                        stateKey = stateKey,
                        path = file.absolutePath,
                        mime = mime,
                        name = name.substringBeforeLast('.'),
                    )
                }
            } catch (e: Exception) {
                toast(coreMessage(e, "Could not read the file"))
            }
        }
    }

    fun confirmPackImage(shortcode: String) {
        val pending = pendingPackImage ?: return
        pendingPackImage = null
        thread {
            runBlocking {
                val error = try {
                    app.addPackImage(
                        pending.stateKey,
                        shortcode,
                        pending.name,
                        pending.path,
                        pending.mime,
                    )
                    null
                } catch (failure: Exception) {
                    coreMessage(failure, "Could not add the image")
                }
                main.post {
                    packsError = error
                    reloadPacksSoon()
                    // The composer's completion should see it too.
                    loadComposerEmoticons()
                }
            }
        }
    }

    fun cancelPackImage() {
        pendingPackImage = null
    }

    /// A pack image's local file, fetched by its mxc URI.
    suspend fun fetchMxcPath(mxcUri: String): String? = try {
        app.getMxcMedia(mxcUri)
    } catch (_: Exception) {
        null
    }

    // Calls. The core carries the m.call.* events; CallEngine carries
    // the sound. This holds the one call a phone can be in at a time.
    private val callState = mutableStateOf<ActiveCall?>(null)

    var call: ActiveCall?
        get() = callState.value
        private set(value) {
            callState.value = value
            // A call announces itself; the room it is in must not also
            // announce the same event as unread messages.
            notifier.callRoomId = value?.roomId
        }

    /// Whether the call has been pushed aside to read the room behind it.
    /// The call carries on; only its screen steps out of the way, and the
    /// banner at the top of every page brings it back.
    var callMinimized by mutableStateOf(false)
        private set

    fun minimizeCall() {
        callMinimized = true
    }

    fun restoreCall() {
        callMinimized = false
    }

    data class ActiveCall(
        val callId: String,
        val roomId: String,
        val peer: String,
        val outgoing: Boolean,
        val state: CallPhase,
        val muted: Boolean = false,
        /// Whether this end is sending pictures.
        val cameraOn: Boolean = false,
        /// Whether the other end is.
        val remoteVideo: Boolean = false,
    )

    enum class CallPhase { Ringing, Dialing, Connecting, Connected, Ended }

    private var engine: CallEngine? = null

    /// Set by the activity: asks for the microphone before a call.
    /// (Shared with voice messages.)
    private fun withMicrophone(onGranted: () -> Unit) {
        val ensure = ensureMicPermission
        if (ensure == null) {
            onGranted()
            return
        }
        ensure { granted ->
            if (granted) {
                onGranted()
            } else {
                toast("A call needs the microphone")
            }
        }
    }

    /// Follow calls: an invite arriving is a phone ringing.
    fun watchCalls() {
        app.setCallListener(object : io.github.steeb_k.commune.core.CallListener {
            override fun onIncoming(
                callId: String,
                roomId: String,
                caller: String,
                sdp: String,
            ) {
                main.post {
                    // The same invite arriving twice is one call, not
                    // two; only a genuinely different call is declined.
                    if (call?.callId == callId) return@post
                    // One call at a time: a second invite is declined
                    // rather than silently dropped.
                    if (call != null) {
                        thread { runBlocking { try { app.rejectCall(callId) } catch (_: Exception) {} } }
                        return@post
                    }
                    pendingOffer = sdp
                    call = ActiveCall(
                        callId = callId,
                        roomId = roomId,
                        peer = caller,
                        outgoing = false,
                        state = CallPhase.Ringing,
                    )
                    Ringtone.start(appContext)
                    // The app may not be on screen; Android's own
                    // incoming-call treatment reaches the person there.
                    IncomingCallNotification.show(appContext, caller)
                }
            }

            override fun onAnswer(callId: String, sdp: String) {
                main.post {
                    if (call?.callId != callId) return@post
                    call = call?.copy(state = CallPhase.Connecting)
                    engine?.acceptAnswer(sdp)
                }
            }

            override fun onCandidates(
                callId: String,
                candidates: List<io.github.steeb_k.commune.core.FfiIceCandidate>,
            ) {
                main.post {
                    if (call?.callId != callId) return@post
                    engine?.addRemoteCandidates(candidates)
                }
            }

            override fun onNegotiate(callId: String, sdp: String, sessionType: String) {
                main.post {
                    if (call?.callId != callId) return@post
                    engine?.acceptRenegotiation(sdp, sessionType) { answer ->
                        thread {
                            runBlocking {
                                try {
                                    app.sendCallNegotiate(callId, answer, "answer")
                                } catch (_: Exception) {
                                }
                            }
                        }
                    }
                }
            }

            override fun onEnded(
                callId: String,
                reason: io.github.steeb_k.commune.core.FfiCallEnd,
            ) {
                main.post {
                    if (call?.callId != callId) return@post
                    val message = when (reason) {
                        io.github.steeb_k.commune.core.FfiCallEnd.DECLINED -> "Call declined"
                        io.github.steeb_k.commune.core.FfiCallEnd.ANSWERED_ELSEWHERE ->
                            "Answered on another session"
                        else -> null
                    }
                    message?.let { toast(it) }
                    tearDownCall()
                }
            }
        })
    }

    /// The offer an incoming call arrived with, until it is answered.
    private var pendingOffer: String? = null

    /// The candidates gathered before the call had an ID to send under.
    private val gatheredCandidates =
        mutableListOf<io.github.steeb_k.commune.core.FfiIceCandidate>()

    /// Gathering that finished before the call had an ID.
    private var pendingGatheringDone = false

    /// Send what was gathered while the call had no ID yet.
    private fun flushGatheredCandidates(callId: String) {
        if (gatheredCandidates.isNotEmpty()) {
            sendCandidates(callId, gatheredCandidates.toList(), false)
            gatheredCandidates.clear()
        }
        if (pendingGatheringDone) {
            pendingGatheringDone = false
            sendCandidates(callId, emptyList(), true)
        }
    }

    private fun buildEngine(onReady: (CallEngine) -> Unit) {
        thread {
            val servers = try {
                runBlocking { app.turnServers() }
            } catch (_: Exception) {
                null
            }
            val iceServers = buildList {
                servers?.uris?.forEach { uri ->
                    add(
                        org.webrtc.PeerConnection.IceServer.builder(uri)
                            .setUsername(servers.username)
                            .setPassword(servers.password)
                            .createIceServer()
                    )
                }
            }
            main.post {
                val built = CallEngine(
                    context = appContext,
                    iceServers = iceServers,
                    onCandidate = { candidate ->
                        main.post {
                            // An outgoing call has no ID until the
                            // invite comes back; candidates gathered
                            // before then wait for it.
                            val callId = call?.callId
                            if (callId.isNullOrEmpty()) {
                                gatheredCandidates.add(candidate)
                            } else {
                                sendCandidates(callId, listOf(candidate), false)
                            }
                        }
                    },
                    onGatheringDone = {
                        main.post {
                            val callId = call?.callId
                            if (callId.isNullOrEmpty()) {
                                pendingGatheringDone = true
                            } else {
                                sendCandidates(callId, emptyList(), true)
                            }
                        }
                    },
                    onConnected = {
                        main.post {
                            engine?.armRenegotiation()
                            if (call?.state != CallPhase.Ended) {
                                call = call?.copy(state = CallPhase.Connected)
                            }
                        }
                    },
                    onFailed = {
                        main.post {
                            toast("The call could not connect")
                            hangUp()
                        }
                    },
                    onRemoteVideo = {
                        main.post { call = call?.copy(remoteVideo = true) }
                    },
                    onNeedsRenegotiation = {
                        main.post { renegotiate() }
                    },
                )
                engine = built
                onReady(built)
            }
        }
    }

    private fun sendCandidates(
        callId: String,
        candidates: List<io.github.steeb_k.commune.core.FfiIceCandidate>,
        end: Boolean,
    ) {
        thread {
            runBlocking {
                try {
                    app.sendCallCandidates(callId, candidates, end)
                } catch (_: Exception) {
                    // A candidate that cannot be sent is one path fewer.
                }
            }
        }
    }

    /// Call the other member of a direct chat.
    fun placeCallInRoom(room: FfiRoom, video: Boolean = false) {
        thread {
            val members = try {
                runBlocking { app.roomMembers(room.roomId) }
            } catch (_: Exception) {
                emptyList()
            }
            val peer = members.map { it.userId }.firstOrNull { it != ownUserId }
            main.post {
                if (peer == null) {
                    toast("Nobody to call in this room")
                } else {
                    placeCall(room.roomId, peer, video)
                }
            }
        }
    }

    /// Place a call to the person in the open direct chat.
    fun placeCall(roomId: String, peer: String, video: Boolean = false) {
        if (call != null) return
        withMicrophone {
            call = ActiveCall(
                callId = "",
                roomId = roomId,
                peer = peer,
                outgoing = true,
                state = CallPhase.Dialing,
            )
            audioForCall(true)
            buildEngine { engine ->
                // A video call offers pictures from the start.
                if (video && engine.setCameraEnabled(true)) {
                    call = call?.copy(cameraOn = true)
                }
                engine.createOffer(
                    onSdp = { sdp ->
                        thread {
                            runBlocking {
                                try {
                                    val callId = app.placeCall(roomId, peer, sdp)
                                    main.post {
                                        call = call?.copy(callId = callId)
                                        CallService.start(appContext, peer)
                                        // Anything gathered before the
                                        // invite had an ID goes now.
                                        flushGatheredCandidates(callId)
                                    }
                                } catch (failure: Exception) {
                                    main.post {
                                        toast(coreMessage(failure, "Could not place the call"))
                                        tearDownCall()
                                    }
                                }
                            }
                        }
                    },
                    onError = { error ->
                        main.post {
                            toast(error)
                            tearDownCall()
                        }
                    },
                )
            }
        }
    }

    /// Answer the call that is ringing.
    fun answerCall() {
        val current = call ?: return
        val offer = pendingOffer ?: return
        Ringtone.stop()
        IncomingCallNotification.dismiss(appContext)
        withMicrophone {
            call = current.copy(state = CallPhase.Connecting)
            audioForCall(true)
            buildEngine { engine ->
                engine.acceptOffer(
                    sdp = offer,
                    onSdp = { sdp ->
                        thread {
                            runBlocking {
                                try {
                                    app.answerCall(current.callId, sdp)
                                    main.post {
                                        CallService.start(appContext, current.peer)
                                        flushGatheredCandidates(current.callId)
                                    }
                                } catch (failure: Exception) {
                                    main.post {
                                        toast(coreMessage(failure, "Could not answer"))
                                        tearDownCall()
                                    }
                                }
                            }
                        }
                    },
                    onError = { error ->
                        main.post {
                            toast(error)
                            tearDownCall()
                        }
                    },
                )
            }
        }
    }

    /// Decline a ringing call.
    fun declineCall() {
        val current = call ?: return
        Ringtone.stop()
        thread {
            runBlocking {
                try {
                    app.rejectCall(current.callId)
                } catch (_: Exception) {
                }
            }
        }
        tearDownCall()
    }

    /// Hang up a call in progress.
    fun hangUp() {
        val current = call ?: return
        thread {
            runBlocking {
                try {
                    if (current.callId.isNotEmpty()) app.hangupCall(current.callId)
                } catch (_: Exception) {
                }
            }
        }
        tearDownCall()
    }

    /// This end changed what it sends: offer the other end a new
    /// description, as the application does when a camera comes on.
    private fun renegotiate() {
        val current = call ?: return
        val engine = engine ?: return
        if (current.callId.isEmpty()) return
        engine.createRenegotiationOffer { sdp ->
            thread {
                runBlocking {
                    try {
                        app.sendCallNegotiate(current.callId, sdp, "offer")
                    } catch (_: Exception) {
                        // The call carries on with what it had.
                    }
                }
            }
        }
    }

    /// Turn this end's camera on or off. Adding the track is what makes
    /// WebRTC ask for the renegotiation that tells the other end.
    fun toggleCamera() {
        val current = call ?: return
        val engine = engine ?: return
        withCamera {
            val wanted = !current.cameraOn
            if (!engine.setCameraEnabled(wanted)) {
                toast("No camera to use")
                return@withCamera
            }
            call = current.copy(cameraOn = wanted)
            sendStreamMetadata()
        }
    }

    fun switchCamera() {
        engine?.switchCamera()
    }

    /// Tell the other end what is muted here.
    private fun sendStreamMetadata() {
        val current = call ?: return
        val engine = engine ?: return
        if (current.callId.isEmpty()) return
        thread {
            runBlocking {
                try {
                    app.sendCallStreamMetadata(
                        current.callId,
                        engine.localStreamId,
                        current.muted,
                        !current.cameraOn,
                    )
                } catch (_: Exception) {
                }
            }
        }
    }

    /// Wire a renderer to the far end's pictures.
    fun initRemoteRenderer(renderer: org.webrtc.SurfaceViewRenderer) {
        val engine = engine ?: return
        renderer.init(engine.eglContext, null)
        renderer.setEnableHardwareScaler(true)
        engine.attachRemoteVideo(renderer)
    }

    /// Wire a renderer to this end's own picture.
    fun initLocalRenderer(renderer: org.webrtc.SurfaceViewRenderer) {
        val engine = engine ?: return
        renderer.init(engine.eglContext, null)
        renderer.setEnableHardwareScaler(true)
        renderer.setZOrderMediaOverlay(true)
        engine.attachLocalVideo(renderer)
    }

    /// Set by the activity: asks for the camera before showing a face.
    var ensureCameraPermission: (((Boolean) -> Unit) -> Unit)? = null

    private fun withCamera(onGranted: () -> Unit) {
        val ensure = ensureCameraPermission
        if (ensure == null) {
            onGranted()
            return
        }
        ensure { granted ->
            if (granted) onGranted() else toast("A video call needs the camera")
        }
    }

    fun toggleMute() {
        val current = call ?: return
        val muted = !current.muted
        engine?.setMicrophoneEnabled(!muted)
        call = current.copy(muted = muted)
        sendStreamMetadata()
    }

    fun setSpeakerphone(on: Boolean) {
        val manager = appContext.getSystemService(android.media.AudioManager::class.java)
        manager.isSpeakerphoneOn = on
    }

    /// Put the audio stack in and out of call mode.
    private fun audioForCall(active: Boolean) {
        val manager = appContext.getSystemService(android.media.AudioManager::class.java)
        manager.mode = if (active) {
            android.media.AudioManager.MODE_IN_COMMUNICATION
        } else {
            android.media.AudioManager.MODE_NORMAL
        }
    }

    private fun tearDownCall() {
        Ringtone.stop()
        IncomingCallNotification.dismiss(appContext)
        engine?.release()
        engine = null
        pendingOffer = null
        gatheredCandidates.clear()
        pendingGatheringDone = false
        call = null
        callMinimized = false
        audioForCall(false)
        CallService.stop(appContext)
    }

    /// Fetch one timeline media file and hand back its local path —
    /// what the in-bubble audio player plays from.
    fun fetchMediaPath(uniqueId: String, onReady: (String?) -> Unit) {
        val room = openRoom ?: return
        thread {
            runBlocking {
                val path = try {
                    app.getTimelineMedia(room.roomId, uniqueId)
                } catch (_: Exception) {
                    null
                }
                main.post { onReady(path) }
            }
        }
    }

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

    fun setSharePresence(share: Boolean) {
        thread {
            runBlocking {
                try {
                    app.setSharePresence(share)
                } catch (_: Exception) {
                }
                val fresh = app.sessionSettings()
                main.post { settings = fresh }
            }
        }
    }

    fun setUrlPreviewsEnabled(enabled: Boolean) {
        app.setUrlPreviewsEnabled(enabled)
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
        // A completed emoticon reads `:shortcode:` in the draft; the core
        // turns each one into the application's image tag.
        val emoticons = composerEmoticons
            .filter { body.contains(":" + it.shortcode + ":") }
        setTyping(false)
        thread {
            runBlocking {
                try {
                    app.sendMessage(room.roomId, body, mentions, emoticons)
                } catch (_: Exception) {
                    // The send queue retries; a failed-send surface comes
                    // with its own chunk.
                }
            }
        }
    }

    /// The emoticons of the account's packs, for the composer's
    /// `:shortcode:` completion; loaded when a room opens.
    var composerEmoticons by mutableStateOf<List<FfiSticker>>(emptyList())
        private set

    fun loadComposerEmoticons() {
        thread {
            runBlocking {
                val emoticons = try {
                    app.emoticonPacks().flatMap { it.stickers }
                } catch (_: Exception) {
                    emptyList()
                }
                main.post { composerEmoticons = emoticons }
            }
        }
    }

    /// The pending attachment preview: picked, not yet sent — the first of
    /// the queue.
    var pendingAttachment by mutableStateOf<PendingAttachment?>(null)
        private set

    /// Everything picked and not yet sent, the previewed one first.
    private var attachmentQueue by mutableStateOf<List<PendingAttachment>>(emptyList())

    /// How many files wait behind the previewed one.
    val remainingAttachments: Int
        get() = (attachmentQueue.size - 1).coerceAtLeast(0)

    data class PendingAttachment(
        val path: String,
        val name: String,
        val mime: String,
        val size: Long,
    )

    /// Send the previewed file, or with `all` every file waiting behind it
    /// too, in order — each awaited before the next so the messages land
    /// in the order the files were picked.
    fun confirmPendingAttachment(all: Boolean = false) {
        val room = openRoom ?: return
        val batch = if (all) attachmentQueue else attachmentQueue.take(1)
        if (batch.isEmpty()) return
        attachmentQueue = attachmentQueue.drop(batch.size)
        pendingAttachment = attachmentQueue.firstOrNull()
        thread {
            for (pending in batch) {
                try {
                    runBlocking { app.sendAttachment(room.roomId, pending.path, pending.mime) }
                } catch (e: Exception) {
                    toast(coreMessage(e, "Could not send the file"))
                }
            }
        }
    }

    /// Drop the previewed file and everything waiting behind it.
    fun cancelPendingAttachment() {
        attachmentQueue = emptyList()
        pendingAttachment = null
    }

    /// Set by the activity: asks for the location permission.
    var ensureLocationPermission: (((Boolean) -> Unit) -> Unit)? = null

    /// The location preview dialog's state: null closed, blank loading,
    /// a geo URI once a fix arrives.
    var pendingLocation by mutableStateOf<String?>(null)
        private set

    fun shareLocation() {
        val ensure = ensureLocationPermission ?: return
        ensure { granted ->
            if (!granted) {
                toast("Location permission was not given")
                return@ensure
            }
            pendingLocation = ""
            val manager = appContext
                .getSystemService(android.location.LocationManager::class.java)
            val provider = when {
                manager.isProviderEnabled(android.location.LocationManager.GPS_PROVIDER) ->
                    android.location.LocationManager.GPS_PROVIDER
                manager.isProviderEnabled(
                    android.location.LocationManager.NETWORK_PROVIDER
                ) -> android.location.LocationManager.NETWORK_PROVIDER
                else -> {
                    pendingLocation = null
                    toast("Location is unavailable")
                    return@ensure
                }
            }
            try {
                manager.getCurrentLocation(
                    provider,
                    null,
                    appContext.mainExecutor,
                ) { location ->
                    if (pendingLocation == null) return@getCurrentLocation
                    if (location == null) {
                        pendingLocation = null
                        toast("Could not find your location")
                    } else {
                        // The geo URI the application sends: coordinates,
                        // with the accuracy when it is known.
                        val uri = "geo:${location.latitude},${location.longitude}" +
                            if (location.hasAccuracy()) ";u=${location.accuracy}" else ""
                        pendingLocation = uri
                    }
                }
            } catch (_: SecurityException) {
                pendingLocation = null
                toast("Location permission was not given")
            }
        }
    }

    fun confirmPendingLocation() {
        val geoUri = pendingLocation?.takeIf { it.isNotBlank() } ?: return
        val room = openRoom ?: return
        pendingLocation = null
        thread {
            runBlocking {
                try {
                    app.sendLocation(room.roomId, geoUri)
                } catch (e: Exception) {
                    toast(coreMessage(e, "Could not send the location"))
                }
            }
        }
    }

    fun cancelPendingLocation() {
        pendingLocation = null
    }
}
