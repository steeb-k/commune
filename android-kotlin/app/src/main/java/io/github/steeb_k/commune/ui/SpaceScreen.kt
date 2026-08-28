// A space: the rooms it gathers, from the server's hierarchy — joined
// ones open, the rest offer a Join, the GTK space view's job.
package io.github.steeb_k.commune.ui

import androidx.compose.foundation.clickable
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
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import io.github.steeb_k.commune.CommuneState
import io.github.steeb_k.commune.core.FfiRoom
import io.github.steeb_k.commune.core.FfiSpaceChild

@Composable
fun SpaceScreen(state: CommuneState, space: FfiRoom) {
    Column(modifier = Modifier.fillMaxSize()) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 4.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconButton(onClick = { state.closeSpace() }) {
                Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
            }
            Column {
                Text(roomName(space), style = MaterialTheme.typography.titleMedium)
                Text(
                    "Space",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }

        when {
            state.spaceLoading -> Row(
                modifier = Modifier.fillMaxWidth().padding(24.dp),
                horizontalArrangement = androidx.compose.foundation.layout.Arrangement.Center,
            ) {
                CircularProgressIndicator()
            }

            state.spaceChildren.isEmpty() -> Text(
                "This space has no rooms",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(16.dp),
            )

            else -> LazyColumn(modifier = Modifier.fillMaxSize()) {
                items(state.spaceChildren.size, key = { state.spaceChildren[it].roomId }) {
                    SpaceChildRow(state, state.spaceChildren[it])
                }
            }
        }
    }
}

@Composable
private fun SpaceChildRow(state: CommuneState, child: FfiSpaceChild) {
    val name = child.name ?: child.roomId

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(enabled = child.isJoined) {
                state.rooms.find { it.roomId == child.roomId }?.let { state.openRoom(it) }
            }
            .padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        InitialsAvatar(identifier = child.roomId, name = name, size = 40.dp)
        Spacer(Modifier.size(12.dp))
        Column(modifier = Modifier.weight(1f)) {
            Text(name, style = MaterialTheme.typography.bodyLarge, maxLines = 1)
            Text(
                "${child.numJoinedMembers} members" +
                    (child.topic?.let { " · $it" } ?: ""),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
            )
        }
        if (!child.isJoined) {
            TextButton(onClick = { state.joinRoom(child.roomId) {} }) { Text("Join") }
        }
    }
}
