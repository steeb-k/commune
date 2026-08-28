// The members of a room: avatar, name, user ID, and a role tag where the
// role is worth a word — the GTK members page's list, grouped by
// membership the way its subpages split them.
package io.github.steeb_k.commune.ui

import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.PersonAdd
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.dp
import io.github.steeb_k.commune.CommuneState
import io.github.steeb_k.commune.core.FfiMember
import io.github.steeb_k.commune.core.FfiMemberRole
import io.github.steeb_k.commune.core.FfiMembership
import io.github.steeb_k.commune.core.FfiRoom

/// The membership groups shown, in order, with their section titles. Joined
/// members head the list untitled, as on the GTK members page.
private val GROUPS = listOf(
    FfiMembership.INVITE to "Invited",
    FfiMembership.KNOCK to "Requested to Join",
    FfiMembership.BAN to "Banned",
)

@Composable
fun MembersScreen(state: CommuneState, room: FfiRoom) {
    Column(modifier = Modifier.fillMaxSize()) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 4.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconButton(onClick = { state.closeMembers() }) {
                Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
            }
            Column(modifier = Modifier.weight(1f)) {
                Text("Members", style = MaterialTheme.typography.titleMedium)
                Text(
                    roomName(room),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                )
            }
            var inviteOpen by remember { mutableStateOf(false) }
            IconButton(onClick = { inviteOpen = true }) {
                Icon(
                    Icons.Filled.PersonAdd,
                    contentDescription = "Invite a user",
                )
            }
            if (inviteOpen) {
                InviteDialog(state, onDismiss = { inviteOpen = false })
            }
        }

        val joined = state.members
            .filter { it.membership == FfiMembership.JOIN }
            .sortedByDescending { it.powerLevel }

        LazyColumn(modifier = Modifier.fillMaxSize()) {
            items(joined.size, key = { joined[it].userId }) { index ->
                MemberRow(state, joined[index])
            }

            for ((membership, title) in GROUPS) {
                val group = state.members.filter { it.membership == membership }
                if (group.isEmpty()) continue

                item(key = "group-$membership") {
                    Text(
                        title,
                        style = MaterialTheme.typography.labelLarge,
                        color = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.padding(start = 16.dp, top = 16.dp, bottom = 4.dp),
                    )
                }
                items(group.size, key = { group[it].userId }) { index ->
                    MemberRow(state, group[index])
                }
            }
        }
    }
}

@Composable
private fun MemberRow(state: CommuneState, member: FfiMember) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        MemberAvatar(state, member, size = 40.dp)
        Spacer(Modifier.size(12.dp))
        Column(modifier = Modifier.weight(1f)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    member.displayName,
                    style = MaterialTheme.typography.bodyLarge,
                    maxLines = 1,
                )
                roleLabel(member.role)?.let { label ->
                    Spacer(Modifier.size(8.dp))
                    Text(
                        label,
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.primary,
                    )
                }
            }
            // The ID resolves an ambiguous name, and identifies everyone
            // else on demand.
            Text(
                member.userId,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
            )
        }
    }
}

/// The word for a role, when it carries one — default members go untagged.
private fun roleLabel(role: FfiMemberRole): String? = when (role) {
    FfiMemberRole.CREATOR -> "Creator"
    FfiMemberRole.ADMINISTRATOR -> "Admin"
    FfiMemberRole.MODERATOR -> "Moderator"
    FfiMemberRole.MUTED -> "Muted"
    FfiMemberRole.DEFAULT, FfiMemberRole.CUSTOM -> null
}

/// A member avatar: the picture when there is one, initials otherwise.
@Composable
private fun MemberAvatar(state: CommuneState, member: FfiMember, size: androidx.compose.ui.unit.Dp) {
    val avatarUrl = member.avatarUrl

    if (avatarUrl == null) {
        InitialsAvatar(identifier = member.userId, name = member.displayName, size = size)
        return
    }

    var bitmap by remember(avatarUrl) { mutableStateOf<android.graphics.Bitmap?>(null) }
    LaunchedEffect(avatarUrl) {
        val path = state.app.getAvatar(avatarUrl, 96u)
        if (path != null) {
            bitmap = android.graphics.BitmapFactory.decodeFile(path)
        }
    }

    val loaded = bitmap
    if (loaded == null) {
        InitialsAvatar(identifier = member.userId, name = member.displayName, size = size)
    } else {
        Image(
            loaded.asImageBitmap(),
            contentDescription = null,
            contentScale = ContentScale.Crop,
            modifier = Modifier.size(size).clip(CircleShape),
        )
    }
}


/// Ask for a user ID and send the invite.
@Composable
private fun InviteDialog(state: CommuneState, onDismiss: () -> Unit) {
    var userId by remember { mutableStateOf("") }
    var error by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }

    androidx.compose.material3.AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Invite a User") },
        text = {
            Column {
                androidx.compose.material3.OutlinedTextField(
                    value = userId,
                    onValueChange = { userId = it },
                    placeholder = { Text("@user:example.org") },
                    singleLine = true,
                )
                error?.let {
                    Text(
                        it,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.error,
                    )
                }
            }
        },
        confirmButton = {
            androidx.compose.material3.TextButton(
                enabled = userId.isNotBlank() && !busy,
                onClick = {
                    busy = true
                    error = null
                    state.inviteUser(userId.trim()) { failure ->
                        busy = false
                        if (failure == null) onDismiss() else error = failure
                    }
                },
            ) { Text("Invite") }
        },
        dismissButton = {
            androidx.compose.material3.TextButton(onClick = onDismiss) { Text("Cancel") }
        },
    )
}
