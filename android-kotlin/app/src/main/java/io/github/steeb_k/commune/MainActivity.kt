// The Kotlin variant of Commune. Jetpack Compose, Material 3 with dynamic
// color, layout mirroring the GTK app per doc/kotlin-plan.md's UI contract.
package io.github.steeb_k.commune

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.systemBarsPadding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import io.github.steeb_k.commune.ui.CommuneTheme
import io.github.steeb_k.commune.ui.LoadingScreen
import io.github.steeb_k.commune.ui.LoginFlow
import io.github.steeb_k.commune.ui.RoomScreen
import io.github.steeb_k.commune.ui.SettingsScreen
import io.github.steeb_k.commune.ui.SidebarScreen

class MainActivity : ComponentActivity() {
    private lateinit var state: CommuneState

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        state = CommuneState(this)

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
            val room = state.openRoom
            if (state.settingsOpen) {
                BackHandler { state.closeSettings() }
                SettingsScreen(state)
            } else if (room == null) {
                SidebarScreen(state)
            } else {
                BackHandler { state.closeRoom() }
                RoomScreen(state, room)
            }
        }
    }
}
