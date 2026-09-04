package io.github.steeb_k.commune.ui

import androidx.compose.runtime.Composable
import io.github.steeb_k.commune.CommuneState

/// Files shared from another app while no room is open: pick the room they
/// go to, then they queue for it like any picked attachment.
@Composable
fun SharePickerDialog(state: CommuneState) {
    if (state.pendingShare.isEmpty()) return

    RoomPickerDialog(
        state,
        title = "Share To",
        onPick = { roomId -> state.shareTo(roomId) },
        onDismiss = { state.cancelShare() },
    )
}
