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
        state.scanQrCode = {
            qrScanner.launch(
                com.journeyapps.barcodescanner.ScanOptions()
                    .setDesiredBarcodeFormats(com.journeyapps.barcodescanner.ScanOptions.QR_CODE)
                    .setPrompt("Scan the QR code shown on your other session")
                    .setBeepEnabled(false)
            )
        }

        if (android.os.Build.VERSION.SDK_INT >= 33) {
            notificationPermission.launch(android.Manifest.permission.POST_NOTIFICATIONS)
        }
        SyncService.start(this)

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
            VerificationDialog(state)
            val room = state.openRoom
            val viewerPath = state.viewerImagePath
            if (viewerPath != null) {
                BackHandler { state.closeViewer() }
                MediaViewerScreen(
                    viewerPath,
                    isVideo = state.viewerIsVideo,
                    onClose = { state.closeViewer() },
                )
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
