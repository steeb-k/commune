// Search inside a room: the server scans message bodies, most recent
// first — the GTK in-room search's server half (the local index for
// encrypted rooms comes later).
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
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
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
import io.github.steeb_k.commune.core.FfiRoom
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

private val WHEN = SimpleDateFormat("MMM d, HH:mm", Locale.getDefault())

@Composable
fun RoomSearchScreen(state: CommuneState, room: FfiRoom) {
    var query by remember { mutableStateOf("") }

    Column(modifier = Modifier.fillMaxSize()) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 4.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconButton(onClick = { state.closeRoomSearch() }) {
                Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
            }
            OutlinedTextField(
                value = query,
                onValueChange = { query = it },
                placeholder = { Text("Search in ${roomName(room)}") },
                singleLine = true,
                modifier = Modifier.weight(1f),
            )
            IconButton(
                enabled = query.isNotBlank() && !state.roomSearchBusy,
                onClick = { state.searchRoom(query.trim()) },
            ) {
                Icon(
                    painterResource(R.drawable.ic_system_search_symbolic),
                    contentDescription = "Search",
                )
            }
        }

        when {
            state.roomSearchBusy -> LoadingFace(modifier = Modifier.fillMaxSize())

            state.roomSearchResults.isEmpty() -> Column(modifier = Modifier.padding(16.dp)) {
                Text(
                    "No results yet — search message text above.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                // An encrypted room is searched on the device, and the
                // index only knows what arrived after it existed; the GTK
                // search page offers the same button in the same place.
                if (room.isEncrypted) {
                    Spacer(Modifier.size(8.dp))
                    androidx.compose.material3.OutlinedButton(
                        onClick = { state.reindexRoomSearch(query.trim()) },
                    ) { Text("Index the loaded messages") }
                }
            }

            else -> LazyColumn(modifier = Modifier.fillMaxSize()) {
                items(state.roomSearchResults.size, key = { state.roomSearchResults[it].eventId }) {
                    val result = state.roomSearchResults[it]
                    Column(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(horizontal = 16.dp, vertical = 8.dp),
                    ) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Text(
                                localpart(result.sender),
                                style = MaterialTheme.typography.labelLarge,
                                color = MaterialTheme.colorScheme.primary,
                            )
                            Spacer(Modifier.size(8.dp))
                            Text(
                                WHEN.format(Date(result.timestamp.toLong())),
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        Text(result.body, style = MaterialTheme.typography.bodyMedium)
                    }
                }
            }
        }
    }
}
