// The room: back + centered title in the header, the timeline as bubbles
// (on by default, per doc/kotlin-plan.md and doc/chat-bubbles.md: own
// messages accent-tinted on the right with the timestamp outermost, others
// neutral on the left), day dividers, and the composer at the bottom.
package io.github.steeb_k.commune.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
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
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Face
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.outlined.PushPin
import androidx.compose.material.icons.automirrored.outlined.Chat
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Button
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
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.unit.dp
import io.github.steeb_k.commune.CommuneState
import io.github.steeb_k.commune.core.FfiEventKind
import io.github.steeb_k.commune.core.FfiMediaKind
import io.github.steeb_k.commune.core.FfiMembershipChange
import io.github.steeb_k.commune.core.FfiRoom
import io.github.steeb_k.commune.core.FfiRoomCategory
import io.github.steeb_k.commune.core.FfiStateChange
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
        if (state.timelineLoading && state.timeline.isEmpty()) {
            LoadingFace(modifier = Modifier.weight(1f))
        } else {
            Timeline(
                state,
                room,
                items = state.timeline,
                modifier = Modifier.weight(1f),
                onOpenThread = { state.openThread(it) },
            )
        }
        TypingLine(state.typingUsers)
        if (room.category == FfiRoomCategory.INVITED) {
            InviteBanner(state)
        } else {
            ComposerActionBar(state)
            Composer(
                onSend = { state.sendFromComposer(it) },
                onTyping = { state.setTyping(it) },
                onAttach = state.pickAttachment,
                onGif = { state.openGifPicker() },
                members = state.composerMembers,
            )
        }
    }

    EventActionSheet(state)
    if (state.gifPickerOpen) {
        GifPickerSheet(state)
    }
}

/// Accept or decline, where the composer would be — an invite is a
/// question before it is a conversation.
@Composable
private fun InviteBanner(state: CommuneState) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 12.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(
            "You have been invited to this room",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(8.dp))
        Row {
            OutlinedButton(onClick = { state.declineInvite() }) {
                Text("Decline")
            }
            Spacer(Modifier.size(16.dp))
            Button(onClick = { state.acceptInvite() }) {
                Text("Accept")
            }
        }
    }
}

/// The bar above the composer naming the armed reply or edit.
@Composable
internal fun ComposerActionBar(state: CommuneState) {
    val reply = state.replyingTo
    val edit = state.editing
    if (reply == null && edit == null) return

    val label = if (reply != null) {
        "Replying to ${reply.senderDisplayName ?: localpart(reply.sender)}"
    } else {
        "Editing message"
    }

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 2.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text(
                label,
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.primary,
            )
            Text(
                (reply ?: edit)?.body.orEmpty(),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
            )
        }
        IconButton(onClick = { state.cancelComposerAction() }) {
            Icon(Icons.Filled.Close, contentDescription = "Cancel")
        }
    }
}

/// The quick reactions the GTK context menu offers first.
private val QUICK_REACTIONS = listOf("\uD83D\uDC4D", "\uD83D\uDC4E", "\u2764\uFE0F", "\uD83D\uDE02", "\uD83C\uDF89", "\uD83D\uDE2E")

/// The long-press action sheet: quick reactions, then the event actions.
@OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
@Composable
internal fun EventActionSheet(state: CommuneState) {
    val event = state.actionSheetEvent ?: return
    val clipboard = LocalClipboardManager.current

    ModalBottomSheet(onDismissRequest = { state.dismissActionSheet() }) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 8.dp),
            horizontalArrangement = Arrangement.SpaceEvenly,
        ) {
            for (key in QUICK_REACTIONS) {
                Text(
                    key,
                    style = MaterialTheme.typography.headlineSmall,
                    modifier = Modifier
                        .clip(CircleShape)
                        .clickable {
                            event.eventId?.let { state.toggleReaction(it, key) }
                            state.dismissActionSheet()
                        }
                        .padding(8.dp),
                )
            }
        }
        HorizontalDivider()

        SheetAction("Reply") {
            state.startReply(event)
            state.dismissActionSheet()
        }
        if (event.isOwn && event.kind is FfiEventKind.Text) {
            SheetAction("Edit") {
                state.startEdit(event)
                state.dismissActionSheet()
            }
        }
        SheetAction("Copy Text") {
            clipboard.setText(AnnotatedString(event.body))
            state.dismissActionSheet()
        }
        if (event.isOwn) {
            SheetAction("Remove", destructive = true) {
                event.eventId?.let { state.redact(it) }
                state.dismissActionSheet()
            }
        }
        Spacer(Modifier.height(24.dp))
    }
}

@Composable
private fun SheetAction(label: String, destructive: Boolean = false, onClick: () -> Unit) {
    Text(
        label,
        style = MaterialTheme.typography.bodyLarge,
        color = if (destructive) {
            MaterialTheme.colorScheme.error
        } else {
            MaterialTheme.colorScheme.onSurface
        },
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .padding(horizontal = 24.dp, vertical = 14.dp),
    )
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
        IconButton(onClick = { state.openRoomSearch() }) {
            Icon(
                androidx.compose.ui.res.painterResource(
                    io.github.steeb_k.commune.R.drawable.ic_system_search_symbolic
                ),
                contentDescription = "Search in room",
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        IconButton(onClick = { state.openPinned() }) {
            Icon(
                Icons.Outlined.PushPin,
                contentDescription = "Pinned messages",
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        IconButton(onClick = { state.openRoomDetails() }) {
            RoomAvatar(state, room, size = 32.dp)
        }
        Spacer(Modifier.size(4.dp))
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
        is FfiEventKind.OtherState -> when (val change = kind.change) {
            is FfiStateChange.Name ->
                change.name?.let { "$sender named the room \"$it\"." }
                    ?: "$sender removed the room name."
            is FfiStateChange.Topic ->
                change.topic?.let { "$sender set the topic to \"$it\"." }
                    ?: "$sender removed the topic."
            is FfiStateChange.Avatar -> "$sender changed the room's picture."
            is FfiStateChange.Create -> "$sender created this room."
            is FfiStateChange.Encryption -> "$sender enabled encryption."
            is FfiStateChange.JoinRules -> "$sender changed who can join."
            is FfiStateChange.HistoryVisibility ->
                "$sender changed who can read the history."
            is FfiStateChange.CanonicalAlias -> "$sender changed the room's address."
            is FfiStateChange.PinnedEvents -> "$sender changed the pinned messages."
            is FfiStateChange.Other -> "$sender changed the room's settings."
        }
        else -> "$sender updated the room."
    }
}

/// One message bubble, per doc/chat-bubbles.md: 12dp radius, own messages
/// accent at 25% on the right with the timestamp outermost, others neutral
/// on the left.
@Composable
@OptIn(ExperimentalFoundationApi::class)
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
        is FfiEventKind.Media -> event.body
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
                .combinedClickable(
                    onClick = {},
                    onLongClick = { state.showActionSheet(event) },
                )
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

            event.inReplyTo?.let { replyTo ->
                Column(
                    modifier = Modifier
                        .padding(bottom = 4.dp)
                        .clip(RoundedCornerShape(6.dp))
                        .background(MaterialTheme.colorScheme.surface.copy(alpha = 0.55f))
                        .padding(horizontal = 8.dp, vertical = 4.dp),
                ) {
                    Text(
                        replyTo.sender?.let(::localpart) ?: "In reply to",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.primary,
                    )
                    Text(
                        replyTo.body ?: "…",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 2,
                    )
                }
            }

            val mediaKind = (event.kind as? FfiEventKind.Media)?.kind
            if (mediaKind == FfiMediaKind.VIDEO || mediaKind == FfiMediaKind.AUDIO) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    modifier = Modifier
                        .padding(bottom = 4.dp)
                        .clip(RoundedCornerShape(8.dp))
                        .background(MaterialTheme.colorScheme.surfaceVariant)
                        .clickable { state.openMediaPlayer(event.uniqueId) }
                        .padding(horizontal = 12.dp, vertical = 10.dp),
                ) {
                    Icon(
                        androidx.compose.ui.res.painterResource(
                            io.github.steeb_k.commune.R.drawable.ic_play_symbolic
                        ),
                        contentDescription = "Play",
                        tint = MaterialTheme.colorScheme.primary,
                    )
                    Spacer(Modifier.size(8.dp))
                    Text(
                        if (mediaKind == FfiMediaKind.VIDEO) "Play video" else "Play audio",
                        style = MaterialTheme.typography.bodyMedium,
                    )
                }
            }
            if (mediaKind == FfiMediaKind.IMAGE) {
                var mediaPath by remember(event.uniqueId) {
                    mutableStateOf<String?>(null)
                }
                LaunchedEffect(event.uniqueId) {
                    mediaPath = state.app.getTimelineMedia(room.roomId, event.uniqueId)
                }

                if (mediaPath == null) {
                    (event.kind as? FfiEventKind.Media)?.blurhash?.let { hash ->
                        BlurhashImage(
                            hash,
                            contentDescription = event.body,
                            modifier = Modifier
                                .fillMaxWidth()
                                .height(200.dp)
                                .clip(RoundedCornerShape(8.dp))
                                .padding(bottom = 4.dp),
                        )
                    }
                }
                mediaPath?.let { path ->
                    // Media fills the bubble and scales up to it, as the
                    // GTK history presents it — small originals included.
                    MediaImage(
                        path,
                        contentDescription = event.body,
                        modifier = Modifier
                            .fillMaxWidth()
                            .heightIn(max = 420.dp)
                            .clip(RoundedCornerShape(8.dp))
                            .clickable { state.openViewer(path) }
                            .padding(bottom = 4.dp),
                    )
                }
            }

            Row(verticalAlignment = Alignment.Bottom) {
                Text(
                    body,
                    style = MaterialTheme.typography.bodyLarge,
                    fontStyle = if (muted) FontStyle.Italic else FontStyle.Normal,
                    color = if (muted) {
                        MaterialTheme.colorScheme.onSurfaceVariant
                    } else {
                        MaterialTheme.colorScheme.onSurface
                    },
                    modifier = Modifier.weight(1f, fill = false),
                )
                if (event.isEdited) {
                    Spacer(Modifier.size(4.dp))
                    Text(
                        "(edited)",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }

            when (event.sendState) {
                io.github.steeb_k.commune.core.FfiSendState.SENDING -> Text(
                    "Sending…",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                io.github.steeb_k.commune.core.FfiSendState.RECOVERABLE_ERROR -> Text(
                    "Not sent — tap to retry",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.error,
                    modifier = Modifier.clickable { state.retrySends() },
                )
                io.github.steeb_k.commune.core.FfiSendState.PERMANENT_ERROR -> Text(
                    "Could not be sent",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.error,
                )
                else -> {}
            }

            ReactionChips(state, event)

            if (event.isOwn && event.receipts.isNotEmpty()) {
                Row(modifier = Modifier.padding(top = 2.dp)) {
                    for (userId in event.receipts.take(5)) {
                        InitialsAvatar(
                            identifier = userId,
                            name = localpart(userId),
                            size = 14.dp,
                        )
                        Spacer(Modifier.size(2.dp))
                    }
                }
            }

            val eventId = event.eventId
            if (event.threadReplies > 0uL && eventId != null && onOpenThread != null) {
                val label = if (event.threadReplies == 1uL) "1 reply" else "${event.threadReplies} replies"
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    modifier = Modifier
                        .padding(top = 4.dp)
                        .clickable { onOpenThread(eventId) },
                ) {
                    Icon(
                        androidx.compose.ui.res.painterResource(
                            io.github.steeb_k.commune.R.drawable.ic_thread_symbolic
                        ),
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.size(14.dp),
                    )
                    Spacer(Modifier.size(4.dp))
                    Text(
                        label,
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.primary,
                    )
                }
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

/// The reactions on an event, as toggleable chips under the bubble body.
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun ReactionChips(state: CommuneState, event: FfiTimelineItem.Event) {
    if (event.reactions.isEmpty()) return
    val eventId = event.eventId ?: return

    FlowRow(modifier = Modifier.padding(top = 4.dp)) {
        for (reaction in event.reactions) {
            val background = if (reaction.isOwn) {
                MaterialTheme.colorScheme.primaryContainer
            } else {
                MaterialTheme.colorScheme.surfaceVariant
            }
            Text(
                "${reaction.key} ${reaction.count}",
                style = MaterialTheme.typography.labelMedium,
                modifier = Modifier
                    .padding(end = 6.dp, bottom = 2.dp)
                    .clip(RoundedCornerShape(12.dp))
                    .background(background)
                    .clickable { state.toggleReaction(eventId, reaction.key) }
                    .padding(horizontal = 8.dp, vertical = 4.dp),
            )
        }
    }
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
internal fun Composer(
    onSend: (String) -> Unit,
    onTyping: (Boolean) -> Unit,
    onAttach: (() -> Unit)? = null,
    onGif: (() -> Unit)? = null,
    members: List<io.github.steeb_k.commune.core.FfiMember> = emptyList(),
) {
    var draft by remember {
        mutableStateOf(androidx.compose.ui.text.input.TextFieldValue(""))
    }

    // Mention completion: the word being typed, when it starts with @.
    val currentWord = draft.text.substringAfterLast(' ').substringAfterLast('\n')
    val mentionQuery = currentWord.takeIf { it.startsWith("@") && it.length > 1 }?.drop(1)
    val matches = mentionQuery?.let { query ->
        members.filter { it.displayName.startsWith(query, ignoreCase = true) }.take(4)
    }.orEmpty()

    if (matches.isNotEmpty()) {
        Row(modifier = Modifier.padding(horizontal = 16.dp, vertical = 2.dp)) {
            for (member in matches) {
                Text(
                    member.displayName,
                    style = MaterialTheme.typography.labelLarge,
                    color = MaterialTheme.colorScheme.primary,
                    modifier = Modifier
                        .padding(end = 8.dp)
                        .clip(RoundedCornerShape(12.dp))
                        .background(MaterialTheme.colorScheme.surfaceVariant)
                        .clickable {
                            val text = draft.text.dropLast(currentWord.length) +
                                "@" + member.displayName + " "
                            draft = androidx.compose.ui.text.input.TextFieldValue(
                                text,
                                androidx.compose.ui.text.TextRange(text.length),
                            )
                        }
                        .padding(horizontal = 10.dp, vertical = 4.dp),
                )
            }
        }
    }

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 8.dp, vertical = 6.dp),
        verticalAlignment = Alignment.Bottom,
    ) {
        IconButton(onClick = { onAttach?.invoke() }, enabled = onAttach != null) {
            Icon(
                androidx.compose.ui.res.painterResource(
                    io.github.steeb_k.commune.R.drawable.ic_attachment_symbolic
                ),
                contentDescription = "Attach",
            )
        }
        // Emoji come from the keyboard on Android; the picker button is the
        // sticker/GIF one, as in the GTK message toolbar.
        IconButton(onClick = { onGif?.invoke() }, enabled = onGif != null) {
            Icon(
                androidx.compose.ui.res.painterResource(
                    io.github.steeb_k.commune.R.drawable.ic_sticker_symbolic
                ),
                contentDescription = "GIFs",
            )
        }
        OutlinedTextField(
            value = draft,
            onValueChange = {
                draft = it
                onTyping(it.text.isNotBlank())
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
                    val body = draft.text.trim()
                    if (body.isNotEmpty()) {
                        draft = androidx.compose.ui.text.input.TextFieldValue("")
                        onSend(body)
                    }
                },
            ) {
                Icon(
                    androidx.compose.ui.res.painterResource(
                        io.github.steeb_k.commune.R.drawable.ic_send_symbolic
                    ),
                    contentDescription = "Send",
                    tint = MaterialTheme.colorScheme.onPrimary,
                )
            }
        }
    }
}
