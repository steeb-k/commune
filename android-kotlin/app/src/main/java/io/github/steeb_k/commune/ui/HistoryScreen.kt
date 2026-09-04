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
import androidx.compose.ui.draw.clipToBounds
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
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.width
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import kotlinx.coroutines.launch
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

        SaveProgressBar(state)
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

/// One entry of the flattened media timeline: a month header or a cell.
private sealed class TimelineEntry {
    class Month(val label: String) : TimelineEntry()
    class Cell(val event: FfiHistoryEvent, val index: Int) : TimelineEntry()
}

private val MONTH = SimpleDateFormat("MMMM yyyy", Locale.getDefault())

/// The media grid as a timeline: month headers spanning the row, and a
/// draggable scrubber along the edge that names where in time you are.
@Composable
private fun MediaGrid(state: CommuneState, selected: Set<String>, toggle: (String) -> Unit) {
    // Flatten events (newest first) into headers + cells.
    val entries = remember(state.historyEvents) {
        val flat = mutableListOf<TimelineEntry>()
        var lastMonth: String? = null
        state.historyEvents.forEachIndexed { index, event ->
            val month = MONTH.format(Date(event.timestamp.toLong()))
            if (month != lastMonth) {
                flat.add(TimelineEntry.Month(month))
                lastMonth = month
            }
            flat.add(TimelineEntry.Cell(event, index))
        }
        flat
    }
    val gridState = androidx.compose.foundation.lazy.grid.rememberLazyGridState()
    val scope = androidx.compose.runtime.rememberCoroutineScope()
    var scrubLabel by remember { mutableStateOf<String?>(null) }

    Box(modifier = Modifier.fillMaxSize()) {
        LazyVerticalGrid(
            columns = GridCells.Fixed(3),
            state = gridState,
            modifier = Modifier.fillMaxSize().padding(horizontal = 4.dp),
        ) {
            items(
                entries.size,
                key = {
                    when (val entry = entries[it]) {
                        is TimelineEntry.Month -> "month:" + entry.label
                        is TimelineEntry.Cell -> entry.event.eventId
                    }
                },
                span = {
                    when (entries[it]) {
                        is TimelineEntry.Month ->
                            androidx.compose.foundation.lazy.grid.GridItemSpan(maxLineSpan)
                        is TimelineEntry.Cell ->
                            androidx.compose.foundation.lazy.grid.GridItemSpan(1)
                    }
                },
            ) { index ->
                when (val entry = entries[index]) {
                    is TimelineEntry.Month -> Text(
                        entry.label,
                        style = MaterialTheme.typography.titleSmall,
                        modifier = Modifier.padding(horizontal = 8.dp, vertical = 10.dp),
                    )
                    is TimelineEntry.Cell -> {
                        MediaCell(state, entry.event, selected, toggle)
                        LoadMoreAtEnd(state, entry.index)
                    }
                }
            }
        }

        TimeScrubber(
            entries = entries,
            onScrub = { fraction, label ->
                scrubLabel = label
                val target = ((entries.size - 1) * fraction).toInt().coerceIn(0, entries.size - 1)
                scope.launch { gridState.scrollToItem(target) }
            },
            onDone = { scrubLabel = null },
            modifier = Modifier.align(Alignment.CenterEnd),
        )

        scrubLabel?.let { label ->
            Text(
                label,
                style = MaterialTheme.typography.titleMedium,
                color = MaterialTheme.colorScheme.onPrimary,
                modifier = Modifier
                    .align(Alignment.CenterEnd)
                    .padding(end = 48.dp)
                    .clip(RoundedCornerShape(16.dp))
                    .background(MaterialTheme.colorScheme.primary)
                    .padding(horizontal = 16.dp, vertical = 8.dp),
            )
        }
    }
}

/// The drag handle along the edge. The fraction of the track maps to the
/// loaded span of the timeline; the label names the month under the
/// thumb. More history keeps loading as the bottom scrolls into view,
/// so the reachable span grows while scrubbing.
@Composable
private fun TimeScrubber(
    entries: List<TimelineEntry>,
    onScrub: (Float, String) -> Unit,
    onDone: () -> Unit,
    modifier: Modifier = Modifier,
) {
    if (entries.size < 12) return
    var trackHeight by remember { mutableStateOf(1) }
    var dragging by remember { mutableStateOf(false) }

    fun labelAt(fraction: Float): String {
        val index = ((entries.size - 1) * fraction).toInt().coerceIn(0, entries.size - 1)
        // Walk back to the month header covering this position.
        for (i in index downTo 0) {
            val entry = entries[i]
            if (entry is TimelineEntry.Month) return entry.label
        }
        return ""
    }

    Box(
        modifier = modifier
            .fillMaxHeight()
            .width(32.dp)
            .onSizeChanged { trackHeight = it.height }
            .pointerInput(entries.size) {
                detectVerticalDragGestures(
                    onDragStart = { offset ->
                        dragging = true
                        val fraction = (offset.y / trackHeight).coerceIn(0f, 1f)
                        onScrub(fraction, labelAt(fraction))
                    },
                    onDragEnd = {
                        dragging = false
                        onDone()
                    },
                    onDragCancel = {
                        dragging = false
                        onDone()
                    },
                ) { change, _ ->
                    val fraction = (change.position.y / trackHeight).coerceIn(0f, 1f)
                    onScrub(fraction, labelAt(fraction))
                }
            },
        contentAlignment = Alignment.CenterEnd,
    ) {
        Box(
            modifier = Modifier
                .padding(end = 4.dp)
                .width(4.dp)
                .fillMaxHeight(0.9f)
                .clip(RoundedCornerShape(2.dp))
                .background(
                    MaterialTheme.colorScheme.onSurfaceVariant.copy(
                        alpha = if (dragging) 0.6f else 0.25f
                    )
                ),
        )
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
    // A picture fetches itself; a video shows the still it carries, as the
    // GTK media history does, and never the video.
    val path = if (event.isVideo) {
        state.historyThumbnails[event.eventId]
    } else {
        state.historyMedia[event.eventId]
    }
    LaunchedEffect(event.eventId) {
        if (event.isVideo) {
            state.fetchHistoryThumbnail(event.eventId)
        } else {
            state.fetchHistoryMedia(event.eventId)
        }
    }
    val isSelected = event.eventId in selected

    Box(
        modifier = Modifier
            .padding(2.dp)
            .fillMaxWidth()
            .aspectRatio(1f)
            .clipToBounds()
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
        if (path != null) {
            MediaImage(
                path,
                contentDescription = event.body,
                modifier = Modifier.fillMaxSize(),
                targetSizePx = 360,
                fill = true,
            )
        }
        if (event.isVideo) {
            Icon(
                painterResource(R.drawable.ic_play_symbolic),
                contentDescription = event.body,
                tint = if (path != null) {
                    androidx.compose.ui.graphics.Color.White
                } else {
                    MaterialTheme.colorScheme.onSurfaceVariant
                },
                modifier = Modifier.size(36.dp),
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
