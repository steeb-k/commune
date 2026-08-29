// The Kotlin variant of Commune. Jetpack Compose, Material 3 with dynamic
// color, layout mirroring the GTK app per doc/kotlin-plan.md's UI contract.
package io.github.steeb_k.commune

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.systemBarsPadding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import io.github.steeb_k.commune.ui.CommuneTheme
import io.github.steeb_k.commune.ui.LoadingScreen
import io.github.steeb_k.commune.ui.LoginFlow
import io.github.steeb_k.commune.ui.MediaViewerScreen
import io.github.steeb_k.commune.ui.MembersScreen
import io.github.steeb_k.commune.ui.PinnedScreen
import io.github.steeb_k.commune.ui.DevicesScreen
import io.github.steeb_k.commune.ui.ExploreScreen
import io.github.steeb_k.commune.ui.PushOnboardingScreen
import io.github.steeb_k.commune.ui.HistoryScreen
import io.github.steeb_k.commune.ui.RoomSearchScreen
import io.github.steeb_k.commune.ui.RoomDetailsScreen
import io.github.steeb_k.commune.ui.RoomScreen
import io.github.steeb_k.commune.ui.SettingsScreen
import io.github.steeb_k.commune.ui.ThreadScreen
import io.github.steeb_k.commune.ui.VerificationDialog
import io.github.steeb_k.commune.ui.SidebarScreen
import io.github.steeb_k.commune.ui.SpaceScreen

class MainActivity : ComponentActivity() {
    private lateinit var state: CommuneState

    private val notificationPermission =
        registerForActivityResult(ActivityResultContracts.RequestPermission()) {}

    private val qrScanner =
        registerForActivityResult(com.journeyapps.barcodescanner.ScanContract()) { result ->
            // Matrix QR payloads are binary; ISO-8859-1 keeps the bytes.
            result.contents?.let {
                state.submitScannedQr(it.toByteArray(Charsets.ISO_8859_1))
            }
        }

    private val attachmentPicker =
        registerForActivityResult(ActivityResultContracts.GetContent()) { uri ->
            uri?.let { state.sendAttachmentFromUri(it) }
        }

    private val avatarPicker =
        registerForActivityResult(ActivityResultContracts.GetContent()) { uri ->
            uri?.let { state.setAvatarFromUri(it) }
        }

    private val packImagePicker =
        registerForActivityResult(ActivityResultContracts.GetContent()) { uri ->
            uri?.let { state.packImagePicked(it) }
        }

    private val roomAvatarPicker =
        registerForActivityResult(ActivityResultContracts.GetContent()) { uri ->
            uri?.let { state.setRoomAvatarFromUri(it) }
        }

    private var micResult: ((Boolean) -> Unit)? = null
    private var locationResult: ((Boolean) -> Unit)? = null

    private val locationPermission =
        registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
            locationResult?.invoke(granted)
            locationResult = null
        }
    private val micPermission =
        registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
            micResult?.invoke(granted)
            micResult = null
        }

    private val keyFilePicker =
        registerForActivityResult(ActivityResultContracts.GetContent()) { uri ->
            uri?.let { state.importKeysFromUri(it) }
        }

    override fun onNewIntent(intent: android.content.Intent) {
        super.onNewIntent(intent)
        intent.getStringExtra("room_id")?.let { state.openRoomById(it) }
        handleRedirect(intent)
        handleCallAction(intent)
    }

    /// The answer and decline buttons on an incoming-call notification.
    private fun handleCallAction(intent: android.content.Intent?) {
        when (intent?.action) {
            IncomingCallNotification.ACTION_ANSWER -> state.answerCall()
            IncomingCallNotification.ACTION_DECLINE -> state.declineCall()
        }
    }

    /// A browser login coming back on the app's custom scheme.
    private fun handleRedirect(intent: android.content.Intent?) {
        val uri = intent?.data ?: return
        if (uri.scheme == "io.github.steeb-k.commune") {
            state.handleLoginRedirect(uri)
        }
    }

    override fun onStart() {
        super.onStart()
        if (::state.isInitialized) state.uiVisible = true
    }

    override fun onStop() {
        super.onStop()
        if (::state.isInitialized) state.uiVisible = false
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        state = CommuneState(this)
        state.pickAttachment = { attachmentPicker.launch("*/*") }
        state.pickAvatar = { avatarPicker.launch("image/*") }
        state.pickRoomAvatar = { roomAvatarPicker.launch("image/*") }
        state.pickImagePackFile = { packImagePicker.launch("image/*") }
        state.pickKeyFile = { keyFilePicker.launch("*/*") }
        state.ensureMicPermission = { onResult ->
            if (checkSelfPermission(android.Manifest.permission.RECORD_AUDIO) ==
                android.content.pm.PackageManager.PERMISSION_GRANTED
            ) {
                onResult(true)
            } else {
                micResult = onResult
                micPermission.launch(android.Manifest.permission.RECORD_AUDIO)
            }
        }
        state.ensureLocationPermission = { onResult ->
            if (checkSelfPermission(android.Manifest.permission.ACCESS_FINE_LOCATION) ==
                android.content.pm.PackageManager.PERMISSION_GRANTED
            ) {
                onResult(true)
            } else {
                locationResult = onResult
                locationPermission.launch(android.Manifest.permission.ACCESS_FINE_LOCATION)
            }
        }
        state.scanQrCode = {
            qrScanner.launch(
                com.journeyapps.barcodescanner.ScanOptions()
                    .setDesiredBarcodeFormats(com.journeyapps.barcodescanner.ScanOptions.QR_CODE)
                    .setPrompt("Scan the QR code shown on your other session")
                    .setBeepEnabled(false)
                    // The library's own capture activity is declared
                    // sensorLandscape; ours follows the app, portrait.
                    .setCaptureActivity(PortraitCaptureActivity::class.java)
                    .setOrientationLocked(false)
            )
        }

        if (android.os.Build.VERSION.SDK_INT >= 33) {
            notificationPermission.launch(android.Manifest.permission.POST_NOTIFICATIONS)
        }
        if (PushManager.mode(this) != PushManager.MODE_UNIFIEDPUSH) {
            SyncService.start(this)
        }
        intent?.getStringExtra("room_id")?.let { state.openRoomById(it) }
        handleRedirect(intent)
        handleCallAction(intent)

        setContent {
            CommuneTheme {
                Surface(
                    modifier = Modifier.fillMaxSize(),
                    color = MaterialTheme.colorScheme.background,
                ) {
                    androidx.compose.foundation.layout.Box(
                        Modifier.systemBarsPadding()
                    ) {
                        CommuneApp(state)
                    }
                }
            }
        }
    }
}

@Composable
private fun CommuneApp(state: CommuneState) {
    when (state.phase) {
        Phase.Loading -> LoadingScreen()
        Phase.Login -> LoginFlow(state)
        Phase.Session -> {
            // A call outranks every page: it is the thing happening.
            if (state.call != null) {
                io.github.steeb_k.commune.ui.CallScreen(state)
                return
            }
            VerificationDialog(state)
            val room = state.openRoom
            val viewerPath = state.viewerImagePath
            // A key produced from the setup screen takes over the
            // screen; one produced from settings shows in settings.
            val recoveryKey = state.recoveryKey?.takeIf { state.setupNeeded }
            if (state.addingAccount) {
                BackHandler { state.cancelAddAccount() }
                LoginFlow(state)
            } else if (recoveryKey != null) {
                io.github.steeb_k.commune.ui.RecoveryKeyScreen(state, recoveryKey)
            } else if (state.setupNeeded) {
                io.github.steeb_k.commune.ui.SessionSetupScreen(state)
            } else if (viewerPath != null) {
                BackHandler { state.closeViewer() }
                MediaViewerScreen(
                    viewerPath,
                    isVideo = state.viewerIsVideo,
                    onClose = { state.closeViewer() },
                )
            } else if (state.pushMode == PushManager.MODE_UNSET) {
                PushOnboardingScreen(state)
            } else if (state.exploreOpen) {
                BackHandler { state.closeExplore() }
                ExploreScreen(state)
            } else if (state.devicesOpen) {
                BackHandler { state.closeDevices() }
                DevicesScreen(state)
            } else if (state.imagePacksOpen) {
                BackHandler { state.closeImagePacks() }
                io.github.steeb_k.commune.ui.ImagePacksScreen(state)
            } else if (state.ignoredUsersOpen) {
                BackHandler { state.closeIgnoredUsers() }
                io.github.steeb_k.commune.ui.IgnoredUsersScreen(state)
            } else if (state.settingsOpen) {
                BackHandler { state.closeSettings() }
                SettingsScreen(state)
            } else if (state.openSpace != null) {
                BackHandler { state.closeSpace() }
                SpaceScreen(state, state.openSpace!!)
            } else if (room == null) {
                SidebarScreen(state)
            } else if (state.membersOpen) {
                BackHandler { state.closeMembers() }
                MembersScreen(state, room)
            } else if (state.historyKind != null) {
                BackHandler { state.closeHistory() }
                HistoryScreen(state, state.historyKind!!)
            } else if (state.addressesOpen) {
                BackHandler { state.closeAddresses() }
                io.github.steeb_k.commune.ui.AddressesScreen(state)
            } else if (state.serverAclOpen) {
                BackHandler { state.closeServerAcl() }
                io.github.steeb_k.commune.ui.ServerAclScreen(state)
            } else if (state.permissionsOpen) {
                BackHandler { state.closePermissions() }
                io.github.steeb_k.commune.ui.PermissionsScreen(state)
            } else if (state.roomDetailsOpen) {
                BackHandler { state.closeRoomDetails() }
                RoomDetailsScreen(state, room)
            } else if (state.roomSearchOpen) {
                BackHandler { state.closeRoomSearch() }
                RoomSearchScreen(state, room)
            } else if (state.pinnedOpen) {
                BackHandler { state.closePinned() }
                PinnedScreen(state, room)
            } else if (state.openThreadRoot != null) {
                BackHandler { state.closeThread() }
                ThreadScreen(state, room)
            } else {
                BackHandler { state.closeRoom() }
                RoomScreen(state, room)
            }
        }
    }
}
