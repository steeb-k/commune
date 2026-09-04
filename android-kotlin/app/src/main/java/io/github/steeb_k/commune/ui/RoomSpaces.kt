package io.github.steeb_k.commune.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import io.github.steeb_k.commune.CommuneState
import io.github.steeb_k.commune.core.FfiRoom
import io.github.steeb_k.commune.core.FfiRoomCategory

// The room's encryption and its spaces, as the GTK room details' general
// page carries them.

@Composable
private fun DetailsActionRow(title: String, caption: String, action: String, onClick: () -> Unit) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.bodyLarge)
            Text(
                caption,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        TextButton(onClick = onClick) { Text(action) }
    }
}

/// Encryption: on, or the switch to turn it on, which the GTK page guards
/// with the same warning. It cannot be turned off again.
@Composable
fun EncryptionRow(state: CommuneState, room: FfiRoom) {
    var confirmOpen by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }

    if (room.isEncrypted) {
        DetailsActionRow("Encryption", "Messages are end-to-end encrypted", "On") {}
        return
    }

    DetailsActionRow("Encryption", "Messages are not encrypted", "Enable") { confirmOpen = true }
    error?.let {
        Text(
            it,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.error,
            modifier = Modifier.padding(horizontal = 16.dp),
        )
    }

    if (confirmOpen) {
        AlertDialog(
            onDismissRequest = { confirmOpen = false },
            title = { Text("Enable Encryption?") },
            text = {
                Text(
                    "Enabling encryption will prevent new members to read the history " +
                        "before they arrived. This cannot be disabled later.",
                )
            },
            confirmButton = {
                TextButton(onClick = {
                    confirmOpen = false
                    state.enableEncryption { failure -> error = failure }
                }) { Text("Enable") }
            },
            dismissButton = {
                TextButton(onClick = { confirmOpen = false }) { Text("Cancel") }
            },
        )
    }
}

/// The joined spaces holding this room, each with its way out, and the
/// way into another: the GTK page's spaces group and space picker.
@Composable
fun SpacesRows(state: CommuneState, room: FfiRoom) {
    var parents by remember(room.roomId) { mutableStateOf<List<FfiRoom>?>(null) }
    var pickerOpen by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var reloads by remember { mutableStateOf(0) }

    LaunchedEffect(room.roomId, reloads) {
        state.parentSpaces { found -> parents = found }
    }

    Text(
        "Spaces",
        style = MaterialTheme.typography.labelLarge,
        color = MaterialTheme.colorScheme.primary,
        modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
    )
    val loaded = parents
    when {
        loaded == null -> Text(
            "Loading…",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(horizontal = 16.dp),
        )
        loaded.isEmpty() -> Text(
            "This room is in none of your spaces",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(horizontal = 16.dp),
        )
        else -> for (space in loaded) {
            DetailsActionRow(roomName(space), "Space", "Remove") {
                state.removeRoomFromSpace(space.roomId) { failure ->
                    if (failure == null) reloads += 1 else error = failure
                }
            }
        }
    }
    DetailsActionRow("Add to a Space", "One of the spaces you are in", "Add") { pickerOpen = true }
    error?.let {
        Text(
            it,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.error,
            modifier = Modifier.padding(horizontal = 16.dp),
        )
    }

    if (pickerOpen) {
        // A space cannot be put inside itself, and one already holding the
        // room is not offered again.
        val holding = loaded.orEmpty().map { it.roomId }.toSet()
        val spaces = state.rooms.filter {
            it.category == FfiRoomCategory.SPACE && it.roomId != room.roomId && it.roomId !in holding
        }
        AlertDialog(
            onDismissRequest = { pickerOpen = false },
            title = { Text("Add to a Space") },
            text = {
                Column {
                    if (spaces.isEmpty()) {
                        Text("No space to add this room to", color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    for (space in spaces) {
                        Text(
                            roomName(space),
                            style = MaterialTheme.typography.bodyLarge,
                            modifier = Modifier
                                .fillMaxWidth()
                                .clickable {
                                    pickerOpen = false
                                    state.addRoomToSpace(space.roomId) { failure ->
                                        if (failure == null) reloads += 1 else error = failure
                                    }
                                }
                                .padding(vertical = 10.dp),
                        )
                    }
                }
            },
            confirmButton = {},
            dismissButton = {
                TextButton(onClick = { pickerOpen = false }) { Text("Cancel") }
            },
        )
    }
}
