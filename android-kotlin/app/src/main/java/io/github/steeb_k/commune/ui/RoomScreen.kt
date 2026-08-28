// The room: back + centered title in the header, the timeline as bubbles
// (on by default, per doc/kotlin-plan.md and doc/chat-bubbles.md: own
// messages accent-tinted on the right with the timestamp outermost, others
// neutral on the left), day dividers, and the composer at the bottom.
package io.github.steeb_k.commune.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
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
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.heightIn
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
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.unit.dp
import io.github.steeb_k.commune.CommuneState
import io.github.steeb_k.commune.core.FfiEventKind
import io.github.steeb_k.commune.core.FfiMembershipChange
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
        RoomHeader(state, room, onBack = { state.closeRoom() })
        Timeline(
            state,
            room,
            items = state.timeline,
            modifier = Modifier.weight(1f),
            onOpenThread = { state.openThread(it) },
        )
        TypingLine(state.typingUsers)
        Composer(
            onSend = { state.send(it) },
            onTyping = { state.setTyping(it) },
        )
    }
}

/// "bob is typing…" — the slide-up typing row, minimally.
@Composable
private fun TypingLine(userIds: List<String>) {
    if (userIds.isEmpty()) return

    val names = userIds.map { localpart(it) }
    val text = when (names.size) {
        1 -> "${names[0]} is typing…"
        2 -> "${names[0]} and ${names[1]} are typing…"
        else -> "${names.size} people are typing…"
    }

    Text(
        text,
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(horizontal = 20.dp, vertical = 2.dp),
    )
}

/// Back at the start, the room name centered — the GTK room header without
/// its call/search/pin/thread buttons, which arrive with their features.
@Composable
private fun RoomHeader(state: CommuneState, room: FfiRoom, onBack: () -> Unit) {
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
        RoomAvatar(state, room, size = 32.dp)
        Spacer(Modifier.size(8.dp))
    }
}

@Composable
internal fun Timeline(
    state: CommuneState,
    room: FfiRoom,
    items: List<FfiTimelineItem>,
    modifier: Modifier,
    onOpenThread: ((String) -> Unit)? = null,
) {
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
                        MessageBubble(
                            state,
                            room,
                            item,
                            showHeader = item.sender != previousSender,
                            onOpenThread = onOpenThread,
                        )
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

internal fun FfiEventKind.isMessageLike(): Boolean = when (this) {
    is FfiEventKind.Text,
    is FfiEventKind.Media,
    is FfiEventKind.Sticker,
    is FfiEventKind.UnableToDecrypt,
    is FfiEventKind.Redacted -> true
    is FfiEventKind.Membership,
    is FfiEventKind.ProfileChange,
    is FfiEventKind.OtherState,
    is FfiEventKind.Unsupported -> false
}

/// A short name for a Matrix user ID: the localpart.
internal fun localpart(userId: String): String =
    userId.removePrefix("@").substringBefore(':')

/// The sentence for a state event — the words the GTK app's state rows
/// speak, minimally.
internal fun stateSentence(event: FfiTimelineItem.Event): String {
    val sender = event.senderDisplayName ?: localpart(event.sender)

    return when (val kind = event.kind) {
        is FfiEventKind.Membership -> {
            val user = localpart(kind.user)
            when (kind.change) {
                FfiMembershipChange.JOINED -> "$user joined this room."
                FfiMembershipChange.LEFT -> "$user left this room."
                FfiMembershipChange.BANNED -> "$user was banned by $sender."
                FfiMembershipChange.UNBANNED -> "$user was unbanned by $sender."
                FfiMembershipChange.KICKED -> "$user was removed by $sender."
                FfiMembershipChange.INVITED -> "$sender invited $user."
                FfiMembershipChange.KICKED_AND_BANNED -> "$user was removed and banned by $sender."
                FfiMembershipChange.INVITATION_ACCEPTED -> "$user accepted the invite."
                FfiMembershipChange.INVITATION_REJECTED -> "$user declined the invite."
                FfiMembershipChange.INVITATION_REVOKED -> "The invite for $user was retracted."
                FfiMembershipChange.KNOCKED -> "$user requested an invite."
                FfiMembershipChange.KNOCK_ACCEPTED -> "The invite request of $user was accepted."
                FfiMembershipChange.KNOCK_RETRACTED -> "$user retracted their invite request."
                FfiMembershipChange.KNOCK_DENIED -> "The invite request of $user was declined."
                FfiMembershipChange.UNKNOWN -> "The membership of $user changed."
            }
        }
        is FfiEventKind.ProfileChange -> "${localpart(kind.user)} changed their profile."
        is FfiEventKind.OtherState -> "$sender changed the room's settings."
        else -> "$sender updated the room."
    }
}

/// One message bubble, per doc/chat-bubbles.md: 12dp radius, own messages
/// accent at 25% on the right with the timestamp outermost, others neutral
/// on the left.
@Composable
internal fun MessageBubble(
    state: CommuneState,
    room: FfiRoom,
    event: FfiTimelineItem.Event,
    showHeader: Boolean,
    onOpenThread: ((String) -> Unit)? = null,
) {
    val own = event.isOwn
    val bubbleColor = if (own) {
        MaterialTheme.colorScheme.primary.copy(alpha = 0.25f)
    } else {
        MaterialTheme.colorScheme.onSurface.copy(alpha = 0.08f)
    }

    val body = when (event.kind) {
        is FfiEventKind.Text -> event.body
        is FfiEventKind.Media -> "📎 ${event.body}"
        is FfiEventKind.Sticker -> "🏷 Sticker"
        is FfiEventKind.UnableToDecrypt -> "Could not decrypt this message"
        is FfiEventKind.Redacted -> "Message removed"
        else -> return
    }
    val muted = event.kind !is FfiEventKind.Text && event.kind !is FfiEventKind.Media

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

            val mediaKind = event.kind as? FfiEventKind.Media
            if (mediaKind?.isImage == true) {
                var bitmap by remember(event.uniqueId) {
                    mutableStateOf<android.graphics.Bitmap?>(null)
                }
                var mediaPath by remember(event.uniqueId) {
                    mutableStateOf<String?>(null)
                }
                LaunchedEffect(event.uniqueId) {
                    val path = state.app.getTimelineMedia(room.roomId, event.uniqueId)
                    if (path != null) {
                        mediaPath = path
                        bitmap = android.graphics.BitmapFactory.decodeFile(path)
                    }
                }

                bitmap?.let {
                    Image(
                        it.asImageBitmap(),
                        contentDescription = event.body,
                        contentScale = ContentScale.Fit,
                        modifier = Modifier
                            .widthIn(max = 280.dp)
                            .heightIn(max = 280.dp)
                            .clip(RoundedCornerShape(8.dp))
                            .clickable { mediaPath?.let(state::openViewer) }
                            .padding(bottom = 4.dp),
                    )
                }
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

            val eventId = event.eventId
            if (event.threadReplies > 0uL && eventId != null && onOpenThread != null) {
                val label = if (event.threadReplies == 1uL) "1 reply" else "${event.threadReplies} replies"
                Text(
                    "\uD83D\uDCAC $label",
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.primary,
                    modifier = Modifier
                        .padding(top = 4.dp)
                        .clickable { onOpenThread(eventId) },
                )
            }
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
internal fun StateLine(event: FfiTimelineItem.Event) {
    Text(
        stateSentence(event),
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 4.dp),
        textAlign = androidx.compose.ui.text.style.TextAlign.Center,
    )
}

@Composable
internal fun CenteredDivider(label: String) {
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
internal fun Composer(onSend: (String) -> Unit, onTyping: (Boolean) -> Unit) {
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
            onValueChange = {
                draft = it
                onTyping(it.isNotBlank())
            },
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
