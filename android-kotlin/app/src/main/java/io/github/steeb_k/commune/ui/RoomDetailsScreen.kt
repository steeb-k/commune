// Room details: the avatar header with the room's name, then rows to the
// detail pages — Members first, as the GTK details sheet has it.
package io.github.steeb_k.commune.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.Person
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.TextButton
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import io.github.steeb_k.commune.CommuneState
import io.github.steeb_k.commune.core.FfiRoom

@Composable
fun RoomDetailsScreen(state: CommuneState, room: FfiRoom) {
    var editOpen by remember { mutableStateOf(false) }

    Column(modifier = Modifier.fillMaxSize()) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 4.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconButton(onClick = { state.closeRoomDetails() }) {
                Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
            }
            Text("Room Details", style = MaterialTheme.typography.titleMedium)
        }

        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(vertical = 16.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            RoomAvatar(state, room, size = 88.dp)
            Spacer(Modifier.size(12.dp))
            Text(roomName(room), style = MaterialTheme.typography.titleLarge)
            room.topic?.let { topic ->
                Text(
                    topic,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 24.dp),
                )
            }
            Text(
                room.roomId,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            TextButton(onClick = { editOpen = true }) { Text("Edit Details") }
        }

        if (editOpen) {
            EditDetailsDialog(state, room, onDismiss = { editOpen = false })
        }

        DetailsRow(
            icon = { Icon(Icons.Filled.Person, contentDescription = null) },
            title = "Members",
            value = room.joinedMembersCount.toString(),
            onClick = { state.openMembers() },
        )
    }
}

/// Name and topic, saved together — the GTK Edit Details page's core.
@Composable
private fun EditDetailsDialog(state: CommuneState, room: FfiRoom, onDismiss: () -> Unit) {
    var name by remember { mutableStateOf(roomName(room)) }
    var topic by remember { mutableStateOf(room.topic.orEmpty()) }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Edit Details") },
        text = {
            Column {
                OutlinedTextField(
                    value = name,
                    onValueChange = { name = it },
                    label = { Text("Room name") },
                    singleLine = true,
                )
                Spacer(Modifier.size(8.dp))
                OutlinedTextField(
                    value = topic,
                    onValueChange = { topic = it },
                    label = { Text("Topic") },
                )
                state.detailsError?.let { error ->
                    Text(
                        error,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.error,
                    )
                }
            }
        },
        confirmButton = {
            TextButton(onClick = { state.setRoomDetails(name, topic) { onDismiss() } }) {
                Text("Save")
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("Cancel") }
        },
    )
}

@Composable
private fun DetailsRow(
    icon: @Composable () -> Unit,
    title: String,
    value: String,
    onClick: () -> Unit,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 14.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        icon()
        Spacer(Modifier.size(16.dp))
        Text(title, style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f))
        Text(
            value,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Icon(
            Icons.AutoMirrored.Filled.KeyboardArrowRight,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}
