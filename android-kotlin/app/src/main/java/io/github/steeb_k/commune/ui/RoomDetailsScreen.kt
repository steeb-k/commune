// Room details: the avatar header with the room's name, then rows to the
// detail pages — Members first, as the GTK details sheet has it.
package io.github.steeb_k.commune.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.InsertDriveFile
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.Block
import androidx.compose.material.icons.filled.Image
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.MusicNote
import androidx.compose.material.icons.filled.Notifications
import androidx.compose.material.icons.filled.Person
import androidx.compose.material.icons.filled.Shield
import androidx.compose.material.icons.filled.Tag
import androidx.compose.material.icons.filled.Upgrade
import androidx.compose.material.icons.filled.Visibility
import io.github.steeb_k.commune.core.FfiHistoryKind
import io.github.steeb_k.commune.core.FfiRoomNotificationMode
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

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState()),
    ) {
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
        var notifOpen by remember { mutableStateOf(false) }
        DetailsRow(
            icon = { Icon(Icons.Filled.Notifications, contentDescription = null) },
            title = "Notifications",
            value = when (state.roomNotifMode) {
                FfiRoomNotificationMode.DEFAULT -> "Default"
                FfiRoomNotificationMode.ALL -> "All messages"
                FfiRoomNotificationMode.MENTIONS_ONLY -> "Mentions only"
                FfiRoomNotificationMode.MUTE -> "Muted"
            },
            onClick = { notifOpen = true },
        )
        if (notifOpen) {
            NotificationModeDialog(state, onDismiss = { notifOpen = false })
        }
        DetailsRow(
            icon = { Icon(Icons.Filled.Image, contentDescription = null) },
            title = "Media",
            value = "",
            onClick = { state.openHistory(FfiHistoryKind.MEDIA) },
        )
        DetailsRow(
            icon = { Icon(Icons.AutoMirrored.Filled.InsertDriveFile, contentDescription = null) },
            title = "Files",
            value = "",
            onClick = { state.openHistory(FfiHistoryKind.FILE) },
        )
        DetailsRow(
            icon = { Icon(Icons.Filled.MusicNote, contentDescription = null) },
            title = "Audio",
            value = "",
            onClick = { state.openHistory(FfiHistoryKind.AUDIO) },
        )

        Text(
            "Room Settings",
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
        )
        var avatarOpen by remember { mutableStateOf(false) }
        DetailsRow(
            icon = { Icon(Icons.Filled.Image, contentDescription = null) },
            title = "Avatar",
            value = "",
            onClick = { avatarOpen = true },
        )
        if (avatarOpen) {
            RoomAvatarDialog(state, onDismiss = { avatarOpen = false })
        }
        var joinRuleOpen by remember { mutableStateOf(false) }
        DetailsRow(
            icon = { Icon(Icons.Filled.Lock, contentDescription = null) },
            title = "Who Can Join",
            value = "",
            onClick = {
                state.loadJoinRule()
                joinRuleOpen = true
            },
        )
        if (joinRuleOpen) {
            JoinRuleDialog(state, onDismiss = { joinRuleOpen = false })
        }
        var historyVisibilityOpen by remember { mutableStateOf(false) }
        DetailsRow(
            icon = { Icon(Icons.Filled.Visibility, contentDescription = null) },
            title = "History Visibility",
            value = "",
            onClick = {
                state.loadHistoryVisibility()
                historyVisibilityOpen = true
            },
        )
        if (historyVisibilityOpen) {
            HistoryVisibilityDialog(state, onDismiss = { historyVisibilityOpen = false })
        }
        DetailsRow(
            icon = { Icon(Icons.Filled.Tag, contentDescription = null) },
            title = "Addresses",
            value = "",
            onClick = { state.openAddresses() },
        )
        DetailsRow(
            icon = { Icon(Icons.Filled.Block, contentDescription = null) },
            title = "Server ACL",
            value = "",
            onClick = { state.openServerAcl() },
        )
        DetailsRow(
            icon = { Icon(Icons.Filled.Shield, contentDescription = null) },
            title = "Permissions",
            value = "",
            onClick = { state.openPermissions() },
        )
        EncryptionRow(state, room)
        if (!room.isDirect) {
            SpacesRows(state, room)
        }
        var upgradeOpen by remember { mutableStateOf(false) }
        DetailsRow(
            icon = { Icon(Icons.Filled.Upgrade, contentDescription = null) },
            title = "Upgrade Room",
            value = "",
            onClick = {
                state.loadUpgradeInfo()
                upgradeOpen = true
            },
        )
        if (upgradeOpen) {
            UpgradeRoomDialog(state, onDismiss = { upgradeOpen = false })
        }
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


/// Pick how this room notifies.
@Composable
private fun NotificationModeDialog(state: CommuneState, onDismiss: () -> Unit) {
    val options = listOf(
        FfiRoomNotificationMode.DEFAULT to "Default",
        FfiRoomNotificationMode.ALL to "All messages",
        FfiRoomNotificationMode.MENTIONS_ONLY to "Mentions and keywords only",
        FfiRoomNotificationMode.MUTE to "Mute",
    )

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Notifications") },
        text = {
            Column {
                for ((mode, label) in options) {
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .clickable {
                                state.setRoomNotificationMode(mode)
                                onDismiss()
                            }
                            .padding(vertical = 10.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        androidx.compose.material3.RadioButton(
                            selected = state.roomNotifMode == mode,
                            onClick = {
                                state.setRoomNotificationMode(mode)
                                onDismiss()
                            },
                        )
                        Text(label, style = MaterialTheme.typography.bodyLarge)
                    }
                }
            }
        },
        confirmButton = {
            TextButton(onClick = onDismiss) { Text("Done") }
        },
    )
}