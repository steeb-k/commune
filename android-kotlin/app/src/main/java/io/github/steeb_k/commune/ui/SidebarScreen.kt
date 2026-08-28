// The sidebar: avatar at the start of the header, search and menu at the
// end, then collapsible sections of room rows with unread badges — the
// GTK sidebar's layout in Material clothes.
package io.github.steeb_k.commune.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
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
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.filled.KeyboardArrowUp
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.Badge
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.background
import io.github.steeb_k.commune.CommuneState
import io.github.steeb_k.commune.core.FfiRoom
import io.github.steeb_k.commune.core.FfiRoomCategory
import io.github.steeb_k.commune.core.FfiRoomHighlight

/// The sidebar's section order and titles, as the GTK app shows them.
private val SECTIONS = listOf(
    FfiRoomCategory.KNOCKED to "Invite Requests",
    FfiRoomCategory.INVITED to "Invited",
    FfiRoomCategory.SERVER_NOTICE to "Server Notices",
    FfiRoomCategory.SPACE to "Spaces",
    FfiRoomCategory.FAVORITE to "Favorites",
    FfiRoomCategory.NORMAL to "Rooms",
    FfiRoomCategory.LOW_PRIORITY to "Low Priority",
    FfiRoomCategory.LEFT to "Historical",
)

@Composable
fun SidebarScreen(state: CommuneState) {
    val collapsed = remember { mutableStateMapOf<FfiRoomCategory, Boolean>() }

    Column(modifier = Modifier.fillMaxSize()) {
        SidebarHeader(state)

        LazyColumn(modifier = Modifier.fillMaxSize()) {
            for ((category, title) in SECTIONS) {
                val section = state.rooms
                    .filter { it.category == category }
                    .sortedByDescending { it.latestActivity }
                if (section.isEmpty()) continue

                val isCollapsed = collapsed[category] == true
                item(key = "section-$category") {
                    SectionHeader(
                        title = title,
                        collapsed = isCollapsed,
                        unreadInside = isCollapsed && section.any { !it.isRead },
                        onToggle = { collapsed[category] = !isCollapsed },
                    )
                }

                if (!isCollapsed) {
                    items(section.size, key = { section[it].roomId }) { index ->
                        RoomRow(state, section[index]) { state.openRoom(section[index]) }
                    }
                }
            }
        }
    }
}

/// Header bar: account avatar at the start, search and primary menu at the
/// end (both placeholders until their features arrive).
@Composable
private fun SidebarHeader(state: CommuneState) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 8.dp, vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        val userId = state.ownUserId ?: "?"
        val localpart = userId.removePrefix("@").substringBefore(':')
        IconButton(onClick = {}) {
            InitialsAvatar(identifier = userId, name = localpart, size = 32.dp)
        }
        Spacer(Modifier.weight(1f))
        HeaderIcon(Icons.Filled.Search)
        HeaderIcon(Icons.Filled.MoreVert)
    }
}

@Composable
private fun HeaderIcon(icon: ImageVector) {
    IconButton(onClick = {}, enabled = false) {
        Icon(icon, contentDescription = null)
    }
}

@Composable
private fun SectionHeader(
    title: String,
    collapsed: Boolean,
    unreadInside: Boolean,
    onToggle: () -> Unit,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onToggle)
            .padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            title,
            style = MaterialTheme.typography.labelLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        if (unreadInside) {
            Spacer(Modifier.size(8.dp))
            Box(
                Modifier
                    .size(8.dp)
                    .clip(CircleShape)
                    .background(MaterialTheme.colorScheme.primary)
            )
        }
        Spacer(Modifier.weight(1f))
        Icon(
            if (collapsed) Icons.Filled.KeyboardArrowDown else Icons.Filled.KeyboardArrowUp,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
private fun RoomRow(state: CommuneState, room: FfiRoom, onClick: () -> Unit) {
    val name = roomName(room)

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RoomAvatar(state, room, size = 40.dp)
        Spacer(Modifier.size(12.dp))
        Text(
            name,
            style = MaterialTheme.typography.bodyLarge,
            fontWeight = if (room.highlight != FfiRoomHighlight.NONE) {
                FontWeight.Bold
            } else {
                FontWeight.Normal
            },
            modifier = Modifier.weight(1f),
            maxLines = 1,
        )

        if (room.notificationCount > 0uL) {
            Badge { Text(room.notificationCount.toString()) }
        } else if (!room.isRead) {
            Box(
                Modifier
                    .size(8.dp)
                    .clip(CircleShape)
                    .background(MaterialTheme.colorScheme.primary)
            )
        }
    }
}
