// The sidebar: avatar at the start of the header, search and menu at the
// end, then collapsible sections of room rows with unread badges — the
// GTK sidebar's layout in Material clothes.
package io.github.steeb_k.commune.ui

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
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
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Badge
import androidx.compose.material3.Button
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.TextButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.background
import io.github.steeb_k.commune.CommuneState
import io.github.steeb_k.commune.core.FfiRoom
import io.github.steeb_k.commune.core.FfiRoomCategory
import io.github.steeb_k.commune.core.FfiRoomHighlight
import io.github.steeb_k.commune.core.FfiTargetRoomCategory

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
    var searchOpen by remember { mutableStateOf(false) }
    var query by remember { mutableStateOf("") }

    AccountSwitcherSheet(state)

    Column(modifier = Modifier.fillMaxSize()) {
        SidebarHeader(
            state,
            searchOpen = searchOpen,
            onToggleSearch = {
                searchOpen = !searchOpen
                if (!searchOpen) query = ""
            },
        )

        if (searchOpen) {
            OutlinedTextField(
                value = query,
                onValueChange = { query = it },
                placeholder = { Text("Search rooms") },
                singleLine = true,
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 4.dp),
            )
        }

        if (state.rooms.isEmpty()) {
            if (!state.roomsLoaded) {
                // The first sync is still on its way; never a blank list.
                LoadingFace(modifier = Modifier.fillMaxSize())
            } else {
                // An account can simply have no rooms yet.
                Box(
                    modifier = Modifier.fillMaxSize(),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        "No rooms yet",
                        style = MaterialTheme.typography.bodyLarge,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            return@Column
        }

        LazyColumn(modifier = Modifier.fillMaxSize()) {
            for ((category, title) in SECTIONS) {
                val section = state.rooms
                    .filter { it.category == category }
                    .filter {
                        query.isBlank() || roomName(it).contains(query.trim(), ignoreCase = true)
                    }
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

/// The account switcher: every session on this device, tap to switch,
/// Add Account into the login flow — the application's account_switcher.
@androidx.compose.runtime.Composable
@kotlin.OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
private fun AccountSwitcherSheet(state: CommuneState) {
    if (!state.accountSwitcherOpen) return

    androidx.compose.material3.ModalBottomSheet(
        onDismissRequest = { state.closeAccountSwitcher() },
    ) {
        for (account in state.accounts) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                modifier = Modifier
                    .fillMaxWidth()
                    .clickable(enabled = account.ready) {
                        state.switchAccount(account.sessionId)
                    }
                    .padding(horizontal = 24.dp, vertical = 10.dp),
            ) {
                InitialsAvatar(
                    identifier = account.userId.ifEmpty { account.sessionId },
                    name = account.userId.removePrefix("@").substringBefore(':')
                        .ifEmpty { "?" },
                    size = 36.dp,
                )
                Spacer(Modifier.size(12.dp))
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        account.userId.ifEmpty { "Restoring…" },
                        style = MaterialTheme.typography.bodyLarge,
                    )
                    Text(
                        account.homeserver,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                androidx.compose.material3.RadioButton(
                    selected = account.active,
                    onClick = { state.switchAccount(account.sessionId) },
                    enabled = account.ready,
                )
            }
        }
        androidx.compose.material3.HorizontalDivider(
            modifier = Modifier.padding(vertical = 4.dp)
        )
        Text(
            "Add Account",
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier
                .fillMaxWidth()
                .clickable { state.startAddAccount() }
                .padding(horizontal = 24.dp, vertical = 12.dp),
        )
        Text(
            "Account Settings",
            style = MaterialTheme.typography.bodyLarge,
            modifier = Modifier
                .fillMaxWidth()
                .clickable {
                    state.closeAccountSwitcher()
                    state.openSettings()
                }
                .padding(horizontal = 24.dp, vertical = 12.dp),
        )
        Spacer(Modifier.size(24.dp))
    }
}

/// Header bar: account avatar at the start, search and primary menu at the
/// end (both placeholders until their features arrive).
@Composable
private fun SidebarHeader(
    state: CommuneState,
    searchOpen: Boolean,
    onToggleSearch: () -> Unit,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 8.dp, vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        val userId = state.ownUserId ?: "?"
        val localpart = userId.removePrefix("@").substringBefore(':')
        // The account's real avatar and name, tappable into settings.
        Row(
            verticalAlignment = Alignment.CenterVertically,
            modifier = Modifier
                .clip(androidx.compose.foundation.shape.RoundedCornerShape(20.dp))
                .clickable { state.openAccountSwitcher() }
                .padding(horizontal = 6.dp, vertical = 4.dp),
        ) {
            val avatarPath = state.profileAvatarPath
            if (avatarPath != null) {
                MediaImage(
                    avatarPath,
                    contentDescription = null,
                    modifier = Modifier
                        .size(32.dp)
                        .clip(androidx.compose.foundation.shape.CircleShape),
                    targetSizePx = 96,
                    fill = true,
                )
            } else {
                InitialsAvatar(identifier = userId, name = localpart, size = 32.dp)
            }
            Spacer(Modifier.size(8.dp))
            Text(
                state.profileName ?: localpart,
                style = MaterialTheme.typography.titleMedium,
                maxLines = 1,
            )
        }
        Spacer(Modifier.weight(1f))
        IconButton(onClick = onToggleSearch) {
            Icon(
                androidx.compose.ui.res.painterResource(
                    io.github.steeb_k.commune.R.drawable.ic_system_search_symbolic
                ),
                contentDescription = "Search",
                tint = if (searchOpen) {
                    MaterialTheme.colorScheme.primary
                } else {
                    MaterialTheme.colorScheme.onSurface
                },
            )
        }
        PrimaryMenu(state)
    }
}

/// Which dialog of the primary menu is open.
private enum class MenuDialog { None, DirectChat, JoinRoom, NewRoom }

/// The primary menu: New Direct Chat and Join Room, as the GTK menu
/// leads; the rest of its entries arrive with their features.
@Composable
private fun PrimaryMenu(state: CommuneState) {
    var menuOpen by remember { mutableStateOf(false) }
    var dialog by remember { mutableStateOf(MenuDialog.None) }

    IconButton(onClick = { menuOpen = true }) {
        Icon(
            androidx.compose.ui.res.painterResource(
                io.github.steeb_k.commune.R.drawable.ic_menu_primary_symbolic
            ),
            contentDescription = "Menu",
        )
    }
    DropdownMenu(expanded = menuOpen, onDismissRequest = { menuOpen = false }) {
        DropdownMenuItem(
            text = { Text("New Direct Chat") },
            onClick = {
                menuOpen = false
                dialog = MenuDialog.DirectChat
            },
        )
        DropdownMenuItem(
            text = { Text("New Room") },
            onClick = {
                menuOpen = false
                dialog = MenuDialog.NewRoom
            },
        )
        DropdownMenuItem(
            text = { Text("Explore Rooms") },
            onClick = {
                menuOpen = false
                state.openExplore()
            },
        )
        DropdownMenuItem(
            text = { Text("Join Room") },
            onClick = {
                menuOpen = false
                dialog = MenuDialog.JoinRoom
            },
        )
    }

    when (dialog) {
        MenuDialog.DirectChat -> ConversationDialog(
            state = state,
            title = "New Direct Chat",
            placeholder = "@user:example.org",
            confirm = "Chat",
            suggestUsers = true,
            onConfirm = { input, done -> state.startDirectChat(input, done) },
            onDismiss = { dialog = MenuDialog.None },
        )
        MenuDialog.NewRoom -> CreateRoomDialog(state, onDismiss = { dialog = MenuDialog.None })
        MenuDialog.JoinRoom -> ConversationDialog(
            state = state,
            title = "Join Room",
            placeholder = "#room:example.org",
            confirm = "Join",
            onConfirm = { input, done -> state.joinRoom(input, done) },
            onDismiss = { dialog = MenuDialog.None },
        )
        MenuDialog.None -> {}
    }
}

@Composable
private fun ConversationDialog(
    state: CommuneState,
    title: String,
    placeholder: String,
    confirm: String,
    onConfirm: (String, () -> Unit) -> Unit,
    onDismiss: () -> Unit,
    // Whether what is typed searches the user directory, as the GTK
    // app's direct chat dialog does.
    suggestUsers: Boolean = false,
) {
    var input by remember { mutableStateOf("") }
    val close = {
        state.clearConversationError()
        onDismiss()
    }

    AlertDialog(
        onDismissRequest = close,
        title = { Text(title) },
        text = {
            Column {
                OutlinedTextField(
                    value = input,
                    onValueChange = { input = it },
                    placeholder = { Text(placeholder) },
                    singleLine = true,
                )
                if (suggestUsers) {
                    UserSuggestions(state, input, exclude = emptySet()) { input = it }
                }
                state.conversationError?.let { error ->
                    Text(
                        error,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.error,
                    )
                }
            }
        },
        confirmButton = {
            Button(
                enabled = input.isNotBlank() && !state.conversationBusy,
                onClick = { onConfirm(input) { close() } },
            ) {
                Text(if (state.conversationBusy) "…" else confirm)
            }
        },
        dismissButton = {
            TextButton(onClick = close) { Text("Cancel") }
        },
    )
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

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun RoomRow(state: CommuneState, room: FfiRoom, onClick: () -> Unit) {
    val name = roomName(room)
    var menuOpen by remember { mutableStateOf(false) }

    Box {
        RoomRowMenu(state, room, menuOpen, onDismiss = { menuOpen = false })
    }
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .combinedClickable(onClick = onClick, onLongClick = { menuOpen = true })
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


/// The sidebar row's long-press menu: the category moves and mark-as-read,
/// as the GTK sidebar's context menu offers them.
@Composable
private fun RoomRowMenu(
    state: CommuneState,
    room: FfiRoom,
    open: Boolean,
    onDismiss: () -> Unit,
) {
    DropdownMenu(expanded = open, onDismissRequest = onDismiss) {
        if (!room.isRead) {
            DropdownMenuItem(
                text = { Text("Mark as Read") },
                onClick = {
                    onDismiss()
                    state.markRead(room.roomId)
                },
            )
        }
        if (room.category == FfiRoomCategory.NORMAL || room.category == FfiRoomCategory.LOW_PRIORITY) {
            DropdownMenuItem(
                text = { Text("Move to Favorites") },
                onClick = {
                    onDismiss()
                    state.changeRoomCategory(room.roomId, FfiTargetRoomCategory.FAVORITE)
                },
            )
        }
        if (room.category == FfiRoomCategory.FAVORITE || room.category == FfiRoomCategory.LOW_PRIORITY) {
            DropdownMenuItem(
                text = { Text("Move to Rooms") },
                onClick = {
                    onDismiss()
                    state.changeRoomCategory(room.roomId, FfiTargetRoomCategory.NORMAL)
                },
            )
        }
        if (room.category == FfiRoomCategory.NORMAL || room.category == FfiRoomCategory.FAVORITE) {
            DropdownMenuItem(
                text = { Text("Move to Low Priority") },
                onClick = {
                    onDismiss()
                    state.changeRoomCategory(room.roomId, FfiTargetRoomCategory.LOW_PRIORITY)
                },
            )
        }
        DropdownMenuItem(
            text = { Text("Leave Room") },
            onClick = {
                onDismiss()
                state.changeRoomCategory(room.roomId, FfiTargetRoomCategory.LEFT)
            },
        )
    }
}


/// Name, optional topic, visibility, and encryption for private rooms —
/// the GTK create dialog's essentials.
@Composable
private fun CreateRoomDialog(state: CommuneState, onDismiss: () -> Unit) {
    var name by remember { mutableStateOf("") }
    var topic by remember { mutableStateOf("") }
    var isPublic by remember { mutableStateOf(false) }
    var encrypted by remember { mutableStateOf(false) }
    var alias by remember { mutableStateOf("") }
    var error by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }

    androidx.compose.material3.AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("New Room") },
        text = {
            Column {
                OutlinedTextField(
                    value = name,
                    onValueChange = { name = it },
                    label = { Text("Name") },
                    singleLine = true,
                )
                OutlinedTextField(
                    value = topic,
                    onValueChange = { topic = it },
                    label = { Text("Topic (optional)") },
                    singleLine = true,
                )
                Row(verticalAlignment = Alignment.CenterVertically) {
                    androidx.compose.material3.Switch(
                        checked = isPublic,
                        onCheckedChange = { isPublic = it },
                    )
                    Text("  Public room")
                }
                if (isPublic) {
                    OutlinedTextField(
                        value = alias,
                        onValueChange = { alias = it },
                        label = { Text("Address") },
                        placeholder = { Text("my-room") },
                        singleLine = true,
                    )
                } else {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        androidx.compose.material3.Switch(
                            checked = encrypted,
                            onCheckedChange = { encrypted = it },
                        )
                        Text("  End-to-end encrypted")
                    }
                }
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
                enabled = name.isNotBlank() && !busy,
                onClick = {
                    busy = true
                    error = null
                    state.createRoom(name, topic, isPublic, encrypted, alias) { failure ->
                        busy = false
                        if (failure == null) onDismiss() else error = failure
                    }
                },
            ) { Text(if (busy) "Creating…" else "Create") }
        },
        dismissButton = {
            androidx.compose.material3.TextButton(onClick = onDismiss) { Text("Cancel") }
        },
    )
}
