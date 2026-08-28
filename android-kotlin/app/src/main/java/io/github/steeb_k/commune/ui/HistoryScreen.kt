// The media history pages under room details — the GTK history viewers:
// Media is a grid of thumbnails, Files and Audio are lists. All three
// walk the room backward as the user scrolls. Long-pressing enters
// selection mode, where a save button copies the picks into the
// device's Downloads — an Android-first addition, not in the GTK app.
package io.github.steeb_k.commune.ui

import android.widget.Toast
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.border
import androidx.compose.foundation.combinedClickable
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
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Close
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
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.shape.CircleShape
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
    var selected by remember { mutableStateOf(setOf<String>()) }
    val context = LocalContext.current

    val toggle: (String) -> Unit = { eventId ->
        selected = if (eventId in selected) selected - eventId else selected + eventId
    }

    Column(modifier = Modifier.fillMaxSize()) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 4.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (selected.isEmpty()) {
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
            } else {
                IconButton(onClick = { selected = emptySet() }) {
                    Icon(Icons.Filled.Close, contentDescription = "Cancel selection")
                }
                Text(
                    "${selected.size} selected",
                    style = MaterialTheme.typography.titleMedium,
                    modifier = Modifier.weight(1f),
                )
                IconButton(onClick = {
                    val picks = state.historyEvents.filter { it.eventId in selected }
                    selected = emptySet()
                    state.downloadHistoryEvents(picks) { saved ->
                        Toast.makeText(
                            context,
                            if (saved == 1) "Saved 1 file to Downloads"
                            else "Saved $saved files to Downloads",
                            Toast.LENGTH_SHORT,
                        ).show()
                    }
                }) {
                    Icon(
                        painterResource(R.drawable.ic_save_symbolic),
                        contentDescription = "Save to Downloads",
                    )
                }
            }
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

            kind == FfiHistoryKind.MEDIA -> MediaGrid(state, selected, toggle)

            else -> FileList(state, selected, toggle)
        }
    }
}

@Composable
private fun MediaGrid(state: CommuneState, selected: Set<String>, toggle: (String) -> Unit) {
    LazyVerticalGrid(
        columns = GridCells.Fixed(3),
        modifier = Modifier.fillMaxSize().padding(horizontal = 4.dp),
    ) {
        items(state.historyEvents.size, key = { state.historyEvents[it].eventId }) { index ->
            val event = state.historyEvents[index]
            MediaCell(state, event, selected, toggle)
            LoadMoreAtEnd(state, index)
        }
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun MediaCell(
    state: CommuneState,
    event: FfiHistoryEvent,
    selected: Set<String>,
    toggle: (String) -> Unit,
) {
    val path = state.historyMedia[event.eventId]
    LaunchedEffect(event.eventId) { state.fetchHistoryMedia(event.eventId) }
    val isSelected = event.eventId in selected

    Box(
        modifier = Modifier
            .padding(2.dp)
            .fillMaxWidth()
            .aspectRatio(1f)
            .background(MaterialTheme.colorScheme.surfaceVariant)
            .then(
                if (isSelected) {
                    Modifier.border(3.dp, MaterialTheme.colorScheme.primary)
                } else {
                    Modifier
                }
            )
            .combinedClickable(
                onLongClick = { toggle(event.eventId) },
                onClick = {
                    if (selected.isNotEmpty()) {
                        toggle(event.eventId)
                    } else {
                        state.fetchHistoryMedia(event.eventId) { mediaPath ->
                            mediaPath?.let { state.openViewer(it, isVideo = event.isVideo) }
                        }
                    }
                },
            ),
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
        if (isSelected) {
            SelectionCheck(modifier = Modifier.align(Alignment.TopEnd).padding(6.dp))
        }
    }
}

@Composable
private fun FileList(state: CommuneState, selected: Set<String>, toggle: (String) -> Unit) {
    LazyColumn(modifier = Modifier.fillMaxSize()) {
        items(state.historyEvents.size, key = { state.historyEvents[it].eventId }) { index ->
            val event = state.historyEvents[index]
            FileRow(state, event, selected, toggle)
            LoadMoreAtEnd(state, index)
        }
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun FileRow(
    state: CommuneState,
    event: FfiHistoryEvent,
    selected: Set<String>,
    toggle: (String) -> Unit,
) {
    val isSelected = event.eventId in selected

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .then(
                if (isSelected) {
                    Modifier.background(MaterialTheme.colorScheme.surfaceVariant)
                } else {
                    Modifier
                }
            )
            .combinedClickable(
                onLongClick = { toggle(event.eventId) },
                onClick = {
                    if (selected.isNotEmpty()) {
                        toggle(event.eventId)
                    } else {
                        state.fetchHistoryMedia(event.eventId) { mediaPath ->
                            // ExoPlayer handles the audio; plain files are
                            // saved through selection instead.
                            if (event.kind == FfiHistoryKind.AUDIO) {
                                mediaPath?.let { state.openViewer(it, isVideo = true) }
                            }
                        }
                    }
                },
            )
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (isSelected) {
            SelectionCheck()
        } else {
            Icon(
                Icons.AutoMirrored.Filled.InsertDriveFile,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
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

/// The filled check that marks a selected item.
@Composable
private fun SelectionCheck(modifier: Modifier = Modifier) {
    Box(
        modifier = modifier
            .size(24.dp)
            .clip(CircleShape)
            .background(MaterialTheme.colorScheme.primary),
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            Icons.Filled.Check,
            contentDescription = "Selected",
            tint = MaterialTheme.colorScheme.onPrimary,
            modifier = Modifier.size(16.dp),
        )
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
