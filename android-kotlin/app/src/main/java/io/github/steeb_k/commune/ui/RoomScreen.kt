// The room: back + centered title in the header, the timeline as bubbles
// (on by default, per doc/kotlin-plan.md and doc/chat-bubbles.md: own
// messages accent-tinted on the right with the timestamp outermost, others
// neutral on the left), day dividers, and the composer at the bottom.
package io.github.steeb_k.commune.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Face
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
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
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.unit.dp
import io.github.steeb_k.commune.CommuneState
import io.github.steeb_k.commune.core.FfiEventKind
import io.github.steeb_k.commune.core.FfiRoom
import io.github.steeb_k.commune.core.FfiTimelineItem
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

private val TIME = SimpleDateFormat("HH:mm", Locale.getDefault())
private val DATE = SimpleDateFormat("EEEE, MMMM d", Locale.getDefault())

@Composable
fun RoomScreen(state: CommuneState, room: FfiRoom) {
    Column(modifier = Modifier.fillMaxSize().imePadding()) {
        RoomHeader(room, onBack = { state.closeRoom() })
        Timeline(state.timeline, modifier = Modifier.weight(1f))
        Composer(onSend = { state.send(it) })
    }
}

/// Back at the start, the room name centered — the GTK room header without
/// its call/search/pin/thread buttons, which arrive with their features.
@Composable
private fun RoomHeader(room: FfiRoom, onBack: () -> Unit) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 4.dp, vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        IconButton(onClick = onBack) {
            Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
        }
        Text(
            roomName(room),
            style = MaterialTheme.typography.titleMedium,
            modifier = Modifier.weight(1f),
            maxLines = 1,
        )
        InitialsAvatar(identifier = room.roomId, name = roomName(room), size = 32.dp)
        Spacer(Modifier.size(8.dp))
    }
}

@Composable
private fun Timeline(items: List<FfiTimelineItem>, modifier: Modifier) {
    val listState = rememberLazyListState()

    // Open at the newest message, and follow it.
    LaunchedEffect(items.size) {
        if (items.isNotEmpty()) listState.scrollToItem(items.size - 1)
    }

    LazyColumn(
        state = listState,
        modifier = modifier.fillMaxWidth(),
        contentPadding = androidx.compose.foundation.layout.PaddingValues(vertical = 8.dp),
    ) {
        items(items.size) { index ->
            val item = items[index]
            val previous = items.getOrNull(index - 1)

            when (item) {
                is FfiTimelineItem.Event -> {
                    val previousSender = (previous as? FfiTimelineItem.Event)
                        ?.takeIf { it.kind.isMessageLike() }
                        ?.sender
                    if (item.kind.isMessageLike()) {
                        MessageBubble(item, showHeader = item.sender != previousSender)
                    } else {
                        StateLine(item)
                    }
                }
                is FfiTimelineItem.DateDivider ->
                    CenteredDivider(DATE.format(Date(item.timestamp.toLong())))
                is FfiTimelineItem.ReadMarker -> {}
                is FfiTimelineItem.TimelineStart ->
                    CenteredDivider("The conversation starts here")
            }
        }
    }
}

private fun FfiEventKind.isMessageLike(): Boolean = when (this) {
    FfiEventKind.TEXT,
    FfiEventKind.MEDIA,
    FfiEventKind.STICKER,
    FfiEventKind.UNABLE_TO_DECRYPT,
    FfiEventKind.REDACTED -> true
    FfiEventKind.MEMBERSHIP, FfiEventKind.OTHER_STATE, FfiEventKind.UNSUPPORTED -> false
}

/// One message bubble, per doc/chat-bubbles.md: 12dp radius, own messages
/// accent at 25% on the right with the timestamp outermost, others neutral
/// on the left.
@Composable
private fun MessageBubble(event: FfiTimelineItem.Event, showHeader: Boolean) {
    val own = event.isOwn
    val bubbleColor = if (own) {
        MaterialTheme.colorScheme.primary.copy(alpha = 0.25f)
    } else {
        MaterialTheme.colorScheme.onSurface.copy(alpha = 0.08f)
    }

    val body = when (event.kind) {
        FfiEventKind.TEXT -> event.body
        FfiEventKind.MEDIA -> "📎 ${event.body}"
        FfiEventKind.STICKER -> "🏷 Sticker"
        FfiEventKind.UNABLE_TO_DECRYPT -> "Could not decrypt this message"
        FfiEventKind.REDACTED -> "Message removed"
        else -> return
    }
    val muted = event.kind != FfiEventKind.TEXT && event.kind != FfiEventKind.MEDIA

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 12.dp, vertical = 2.dp),
        horizontalArrangement = if (own) Arrangement.End else Arrangement.Start,
    ) {
        Column(
            modifier = Modifier
                .widthIn(max = 320.dp)
                .clip(RoundedCornerShape(12.dp))
                .background(bubbleColor)
                .padding(horizontal = 12.dp, vertical = 8.dp),
            horizontalAlignment = if (own) Alignment.End else Alignment.Start,
        ) {
            if (showHeader) {
                val name = event.senderDisplayName ?: event.sender
                val time = TIME.format(Date(event.timestamp.toLong()))
                Row {
                    // The timestamp sits outermost: left of the name for own
                    // messages, right of it for others.
                    if (own) {
                        BubbleTimestamp(time)
                        Spacer(Modifier.size(6.dp))
                    }
                    Text(
                        name,
                        style = MaterialTheme.typography.labelMedium,
                        fontWeight = FontWeight.Bold,
                        color = MaterialTheme.colorScheme.primary,
                    )
                    if (!own) {
                        Spacer(Modifier.size(6.dp))
                        BubbleTimestamp(time)
                    }
                }
                Spacer(Modifier.height(2.dp))
            }
            Text(
                body,
                style = MaterialTheme.typography.bodyLarge,
                fontStyle = if (muted) FontStyle.Italic else FontStyle.Normal,
                color = if (muted) {
                    MaterialTheme.colorScheme.onSurfaceVariant
                } else {
                    MaterialTheme.colorScheme.onSurface
                },
            )
        }
    }
}

@Composable
private fun BubbleTimestamp(time: String) {
    Text(
        time,
        style = MaterialTheme.typography.labelSmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}

/// A state event, as a dim centered line — the sentence itself arrives with
/// the state-event humanization chunk.
@Composable
private fun StateLine(event: FfiTimelineItem.Event) {
    val who = event.senderDisplayName ?: event.sender
    Text(
        "$who updated the room",
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 4.dp),
        textAlign = androidx.compose.ui.text.style.TextAlign.Center,
    )
}

@Composable
private fun CenteredDivider(label: String) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        HorizontalDivider(modifier = Modifier.weight(1f))
        Text(
            label,
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(horizontal = 12.dp),
        )
        HorizontalDivider(modifier = Modifier.weight(1f))
    }
}

/// The composer: attach and emoji at the start (placeholders until their
/// chunks), the entry, and the round send button — the GTK toolbar row.
@Composable
private fun Composer(onSend: (String) -> Unit) {
    var draft by remember { mutableStateOf("") }

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 8.dp, vertical = 6.dp),
        verticalAlignment = Alignment.Bottom,
    ) {
        IconButton(onClick = {}, enabled = false) {
            Icon(Icons.Filled.Add, contentDescription = "Attach")
        }
        IconButton(onClick = {}, enabled = false) {
            Icon(Icons.Filled.Face, contentDescription = "Emoji")
        }
        OutlinedTextField(
            value = draft,
            onValueChange = { draft = it },
            placeholder = { Text("Message") },
            modifier = Modifier.weight(1f),
            maxLines = 5,
            shape = RoundedCornerShape(24.dp),
        )
        Spacer(Modifier.size(6.dp))
        Box(
            modifier = Modifier
                .size(48.dp)
                .clip(CircleShape)
                .background(MaterialTheme.colorScheme.primary),
            contentAlignment = Alignment.Center,
        ) {
            IconButton(
                onClick = {
                    val body = draft.trim()
                    if (body.isNotEmpty()) {
                        draft = ""
                        onSend(body)
                    }
                },
            ) {
                Icon(
                    Icons.AutoMirrored.Filled.Send,
                    contentDescription = "Send",
                    tint = MaterialTheme.colorScheme.onPrimary,
                )
            }
        }
    }
}
