// The public room directory — the GTK Explore page: a search field over
// the homeserver's directory, rows with a Join button that flips to
// Joined when the server confirms.
package io.github.steeb_k.commune.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.unit.dp
import io.github.steeb_k.commune.CommuneState
import io.github.steeb_k.commune.R
import io.github.steeb_k.commune.core.FfiPublicRoom

@Composable
fun ExploreScreen(state: CommuneState) {
    var query by remember { mutableStateOf("") }

    Column(modifier = Modifier.fillMaxSize()) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 4.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconButton(onClick = { state.closeExplore() }) {
                Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
            }
            OutlinedTextField(
                value = query,
                onValueChange = { query = it },
                placeholder = { Text("Search the room directory") },
                singleLine = true,
                modifier = Modifier.weight(1f),
            )
            IconButton(
                enabled = !state.exploreBusy,
                onClick = { state.searchExplore(query.trim()) },
            ) {
                Icon(
                    painterResource(R.drawable.ic_system_search_symbolic),
                    contentDescription = "Search",
                )
            }
        }

        when {
            state.exploreRooms.isEmpty() && state.exploreBusy ->
                LoadingFace(modifier = Modifier.fillMaxSize())

            state.exploreRooms.isEmpty() -> Text(
                "No public rooms found.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(16.dp),
            )

            else -> LazyColumn(modifier = Modifier.fillMaxSize()) {
                items(state.exploreRooms.size, key = { state.exploreRooms[it].roomId }) { index ->
                    PublicRoomRow(state, state.exploreRooms[index])
                    if (index == state.exploreRooms.size - 1) {
                        LaunchedEffect(index) { state.loadMoreExplore() }
                    }
                }
            }
        }
    }
}

@Composable
private fun PublicRoomRow(state: CommuneState, room: FfiPublicRoom) {
    val title = room.name ?: room.alias ?: room.roomId

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        InitialsAvatar(room.roomId, title, size = 40.dp)
        Spacer(Modifier.size(12.dp))
        Column(modifier = Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.bodyLarge, maxLines = 1)
            Text(
                listOfNotNull(
                    "${room.joinedMembers} members",
                    room.topic?.takeIf { it.isNotBlank() },
                ).joinToString(" · "),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 2,
            )
        }
        Spacer(Modifier.size(8.dp))
        if (room.isJoined) {
            Text(
                "Joined",
                style = MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        } else {
            OutlinedButton(onClick = { state.joinExploreRoom(room) }) {
                Text("Join")
            }
        }
    }
}
