// The media history pages under room details — the GTK history viewers:
// Media is a grid of thumbnails, Files and Audio are lists. All three
// walk the room backward as the user scrolls.
package io.github.steeb_k.commune.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.background
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.InsertDriveFile
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.unit.dp
import io.github.steeb_k.commune.CommuneState
import io.github.steeb_k.commune.R
import io.github.steeb_k.commune.core.FfiHistoryEvent
import io.github.steeb_k.commune.core.FfiHistoryKind
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

private val WHEN = SimpleDateFormat("MMM d, yyyy", Locale.getDefault())

@Composable
fun HistoryScreen(state: CommuneState, kind: FfiHistoryKind) {
    Column(modifier = Modifier.fillMaxSize()) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 4.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconButton(onClick = { state.closeHistory() }) {
                Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
            }
            Text(
                when (kind) {
                    FfiHistoryKind.MEDIA -> "Media"
                    FfiHistoryKind.FILE -> "Files"
                    FfiHistoryKind.AUDIO -> "Audio"
                },
                style = MaterialTheme.typography.titleMedium,
            )
        }

        when {
            state.historyEvents.isEmpty() && state.historyBusy ->
                LoadingFace(modifier = Modifier.fillMaxSize())

            state.historyEvents.isEmpty() -> Text(
                "Nothing here yet.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(16.dp),
            )

            kind == FfiHistoryKind.MEDIA -> MediaGrid(state)

            else -> FileList(state)
        }
    }
}

@Composable
private fun MediaGrid(state: CommuneState) {
    LazyVerticalGrid(
        columns = GridCells.Fixed(3),
        modifier = Modifier.fillMaxSize().padding(horizontal = 4.dp),
    ) {
        items(state.historyEvents.size, key = { state.historyEvents[it].eventId }) { index ->
            val event = state.historyEvents[index]
            MediaCell(state, event)
            LoadMoreAtEnd(state, index)
        }
    }
}

@Composable
private fun MediaCell(state: CommuneState, event: FfiHistoryEvent) {
    val path = state.historyMedia[event.eventId]
    LaunchedEffect(event.eventId) { state.fetchHistoryMedia(event.eventId) }

    Box(
        modifier = Modifier
            .padding(2.dp)
            .fillMaxWidth()
            .aspectRatio(1f)
            .background(MaterialTheme.colorScheme.surfaceVariant)
            .clickable {
                state.fetchHistoryMedia(event.eventId) { mediaPath ->
                    mediaPath?.let { state.openViewer(it, isVideo = event.isVideo) }
                }
            },
        contentAlignment = Alignment.Center,
    ) {
        when {
            event.isVideo -> Icon(
                painterResource(R.drawable.ic_play_symbolic),
                contentDescription = event.body,
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.size(36.dp),
            )

            path != null -> MediaImage(
                path,
                contentDescription = event.body,
                modifier = Modifier.fillMaxSize(),
            )
        }
    }
}

@Composable
private fun FileList(state: CommuneState) {
    LazyColumn(modifier = Modifier.fillMaxSize()) {
        items(state.historyEvents.size, key = { state.historyEvents[it].eventId }) { index ->
            val event = state.historyEvents[index]
            FileRow(state, event)
            LoadMoreAtEnd(state, index)
        }
    }
}

@Composable
private fun FileRow(state: CommuneState, event: FfiHistoryEvent) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable {
                state.fetchHistoryMedia(event.eventId) { mediaPath ->
                    // ExoPlayer handles the audio; plain files have nowhere
                    // to go yet, opening them arrives with sharing.
                    if (event.kind == FfiHistoryKind.AUDIO) {
                        mediaPath?.let { state.openViewer(it, isVideo = true) }
                    }
                }
            }
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            Icons.AutoMirrored.Filled.InsertDriveFile,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.size(16.dp))
        Column(modifier = Modifier.weight(1f)) {
            Text(event.body, style = MaterialTheme.typography.bodyLarge, maxLines = 1)
            Text(
                listOfNotNull(
                    WHEN.format(Date(event.timestamp.toLong())),
                    event.size?.let { formatSize(it.toLong()) },
                ).joinToString(" · "),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

/// Ask for another page when the last loaded row comes into view.
@Composable
private fun LoadMoreAtEnd(state: CommuneState, index: Int) {
    if (index == state.historyEvents.size - 1) {
        LaunchedEffect(index) { state.loadMoreHistory() }
    }
}

private fun formatSize(bytes: Long): String = when {
    bytes >= 1024 * 1024 -> "%.1f MB".format(bytes / (1024.0 * 1024.0))
    bytes >= 1024 -> "%.0f KB".format(bytes / 1024.0)
    else -> "$bytes B"
}
