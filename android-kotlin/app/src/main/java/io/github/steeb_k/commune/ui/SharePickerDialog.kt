package io.github.steeb_k.commune.ui

import androidx.compose.runtime.Composable
import io.github.steeb_k.commune.CommuneState

/// Something shared from another app without a room chosen on the share
/// sheet: pick the room it goes to, whatever page is open — the share sheet
/// said nothing about where, so nothing is assumed. Files then queue for
/// the room like any picked attachment; text lands in its composer.
@Composable
fun SharePickerDialog(state: CommuneState) {
    if (!state.sharePickerNeeded) return

    RoomPickerDialog(
        state,
        title = "Share To",
        onPick = { roomId -> state.shareTo(roomId) },
        onDismiss = { state.cancelShare() },
    )
}
