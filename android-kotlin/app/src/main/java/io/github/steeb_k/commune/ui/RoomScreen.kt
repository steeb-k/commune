// The room: back + centered title in the header, the timeline as bubbles
// (on by default, per doc/kotlin-plan.md and doc/chat-bubbles.md: own
// messages accent-tinted on the right with the timestamp outermost, others
// neutral on the left), day dividers, and the composer at the bottom.
package io.github.steeb_k.commune.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.InsertDriveFile
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Call
import androidx.compose.material.icons.filled.Videocam
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Face
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Place
import androidx.compose.material.icons.filled.Mic
import androidx.compose.material.icons.filled.Pause
import androidx.compose.material.icons.outlined.PushPin
import androidx.compose.material.icons.outlined.GppBad
import androidx.compose.material.icons.outlined.GppMaybe
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
import androidx.compose.runtime.DisposableEffect
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
    // This screen is composed only while it is the one on the display,
    // so its presence is what "mapped" means for read receipts.
    DisposableEffect(room.roomId) {
        state.roomScreenShown(true)
        onDispose { state.roomScreenShown(false) }
    }
    Column(modifier = Modifier.fillMaxSize().imePadding()) {
        if (state.selectMode) {
            SelectionBar(state)
        } else {
            RoomHeader(state, room, onBack = { state.closeRoom() })
        }
        if (state.timelineLoading && state.timeline.isEmpty()) {
            LoadingFace(modifier = Modifier.weight(1f))
        } else {
            Timeline(
                state,
                room,
                items = state.timeline,
                modifier = Modifier.weight(1f),
                onOpenThread = { state.openThread(it) },
                live = true,
            )
        }
        TypingLine(state.typingUsers)
        if (room.category == FfiRoomCategory.INVITED) {
            InviteBanner(state)
        } else {
            SaveProgressBar(state)
            ComposerActionBar(state)
            if (state.recordingVoice) {
                RecordingBar(state)
            } else {
                Composer(
                    onSend = { state.sendFromComposer(it) },
                    onTyping = { state.setTyping(it) },
                    onAttach = state.pickAttachment,
                    onGif = { state.openGifPicker() },
                    onVoice = { state.startVoiceRecording() },
                    onLocation = { state.shareLocation() },
                    members = state.composerMembers,
                    emoticons = state.composerEmoticons,
                    loadDraft = { state.app.loadDraft(room.roomId) },
                    saveDraft = { text -> state.app.saveDraft(room.roomId, text) },
                    editBody = state.editing?.body,
                    prefill = state.pendingComposerText,
                    onPrefillTaken = { state.takeComposerText() },
                )
            }
        }
    }

    EventActionSheet(state)
    EventSourceDialog(state)
    AttachmentPreviewDialog(state)
    LocationPreviewDialog(state)
    if (state.gifPickerOpen) {
        GifPickerSheet(state)
    }
}

/// The picked file, shown before anything is sent — the GTK attachment
/// dialog: a preview for pictures, the name and size for the rest.
@Composable
private fun AttachmentPreviewDialog(state: CommuneState) {
    val pending = state.pendingAttachment ?: return
    val remaining = state.remainingAttachments

    androidx.compose.material3.AlertDialog(
        onDismissRequest = { state.cancelPendingAttachment() },
        title = {
            Text(if (remaining > 0) "Send File? (${remaining + 1} picked)" else "Send File?")
        },
        text = {
            Column {
                if (pending.mime.startsWith("image/")) {
                    MediaImage(
                        pending.path,
                        contentDescription = pending.name,
                        modifier = Modifier
                            .fillMaxWidth()
                            .heightIn(max = 280.dp),
                        targetSizePx = 720,
                    )
                    Spacer(Modifier.height(8.dp))
                }
                Text(pending.name, style = MaterialTheme.typography.bodyLarge)
                Text(
                    android.text.format.Formatter
                        .formatFileSize(androidx.compose.ui.platform.LocalContext.current,
                            pending.size),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        },
        confirmButton = {
            Row {
                // With more behind this one, the GTK dialog offers to send
                // them all rather than ask about each.
                if (remaining > 0) {
                    androidx.compose.material3.TextButton(
                        onClick = { state.confirmPendingAttachment(all = true) },
                    ) { Text("Send All (${remaining + 1})") }
                }
                androidx.compose.material3.TextButton(
                    onClick = { state.confirmPendingAttachment() },
                ) { Text("Send") }
            }
        },
        dismissButton = {
            androidx.compose.material3.TextButton(
                onClick = { state.cancelPendingAttachment() },
            ) { Text("Cancel") }
        },
    )
}

/// Your location before it goes anywhere: the fix as it converges, sent
/// only on confirmation — the GTK location dialog.
@Composable
private fun LocationPreviewDialog(state: CommuneState) {
    val pending = state.pendingLocation ?: return

    androidx.compose.material3.AlertDialog(
        onDismissRequest = { state.cancelPendingLocation() },
        title = { Text("Your Location") },
        text = {
            if (pending.isBlank()) {
                Text("Finding your location…")
            } else {
                Text(pending, fontFamily = androidx.compose.ui.text.font.FontFamily.Monospace)
            }
        },
        confirmButton = {
            androidx.compose.material3.TextButton(
                enabled = pending.isNotBlank(),
                onClick = { state.confirmPendingLocation() },
            ) { Text("Send") }
        },
        dismissButton = {
            androidx.compose.material3.TextButton(
                onClick = { state.cancelPendingLocation() },
            ) { Text("Cancel") }
        },
    )
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
        if (event.kind is FfiEventKind.Media) {
            SheetAction("Save to Downloads") {
                state.saveEventMedia(event)
                state.dismissActionSheet()
            }
        }
        var forwardOpen by remember { mutableStateOf(false) }
        var reportOpen by remember { mutableStateOf(false) }
        if (event.eventId != null) {
            SheetAction("Forward…") { forwardOpen = true }
            SheetAction("Copy Message Link") {
                event.eventId?.let { state.copyEventLink(it) }
                state.dismissActionSheet()
            }
            SheetAction("Select") {
                state.startSelection(event.uniqueId)
                state.dismissActionSheet()
            }
            SheetAction(if (event.isPinned) "Unpin" else "Pin") {
                state.togglePin(event) {}
                state.dismissActionSheet()
            }
            SheetAction("Properties") {
                event.eventId?.let { state.openEventSource(it) }
                state.dismissActionSheet()
            }
            SheetAction("Report…") { reportOpen = true }
        } else {
            // A message that never left this device can be thrown away.
            SheetAction("Discard", destructive = true) {
                state.discardEcho(event.uniqueId)
                state.dismissActionSheet()
            }
        }
        if (event.isOwn && event.eventId != null) {
            SheetAction("Remove", destructive = true) {
                event.eventId?.let { state.redact(it) }
                state.dismissActionSheet()
            }
        }
        Spacer(Modifier.height(24.dp))

        if (forwardOpen) {
            RoomPickerDialog(
                state,
                title = "Forward To",
                onPick = { roomId ->
                    event.eventId?.let { state.forwardEvent(it, roomId) }
                    forwardOpen = false
                    state.dismissActionSheet()
                },
                onDismiss = { forwardOpen = false },
            )
        }
        if (reportOpen) {
            ReportDialog(
                onReport = { reason ->
                    event.eventId?.let { eventId ->
                        state.reportEvent(eventId, reason) { failure ->
                            if (failure != null) {
                                // The toast path already spoke.
                            }
                        }
                    }
                    reportOpen = false
                    state.dismissActionSheet()
                },
                onDismiss = { reportOpen = false },
            )
        }
    }
}

/// Pick one joined room — the forward target.
@Composable
internal fun RoomPickerDialog(
    state: CommuneState,
    title: String,
    onPick: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    androidx.compose.material3.AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = {
            androidx.compose.foundation.lazy.LazyColumn {
                val rooms = state.rooms.filter {
                    it.category == io.github.steeb_k.commune.core.FfiRoomCategory.NORMAL ||
                        it.category ==
                        io.github.steeb_k.commune.core.FfiRoomCategory.FAVORITE ||
                        it.category ==
                        io.github.steeb_k.commune.core.FfiRoomCategory.LOW_PRIORITY
                }
                items(rooms.size) { index ->
                    val room = rooms[index]
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .clickable { onPick(room.roomId) }
                            .padding(vertical = 8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        RoomAvatar(state, room, size = 32.dp)
                        Spacer(Modifier.size(12.dp))
                        Text(roomName(room), style = MaterialTheme.typography.bodyLarge)
                    }
                }
            }
        },
        confirmButton = {
            androidx.compose.material3.TextButton(onClick = onDismiss) { Text("Cancel") }
        },
    )
}

/// Ask for an optional reason and report.
@Composable
private fun ReportDialog(onReport: (String) -> Unit, onDismiss: () -> Unit) {
    var reason by remember { mutableStateOf("") }

    androidx.compose.material3.AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Report Event?") },
        text = {
            Column {
                Text(
                    "Reporting sends this event's ID to your homeserver's " +
                        "administrator. They cannot see the content of an " +
                        "encrypted or removed event.",
                    style = MaterialTheme.typography.bodyMedium,
                )
                Spacer(Modifier.height(8.dp))
                OutlinedTextField(
                    value = reason,
                    onValueChange = { reason = it },
                    label = { Text("Reason (optional)") },
                )
            }
        },
        confirmButton = {
            androidx.compose.material3.TextButton(onClick = { onReport(reason) }) {
                Text("Report", color = MaterialTheme.colorScheme.error)
            }
        },
        dismissButton = {
            androidx.compose.material3.TextButton(onClick = onDismiss) { Text("Cancel") }
        },
    )
}

/// The raw JSON of an event — the properties dialog's source view.
@Composable
private fun EventSourceDialog(state: CommuneState) {
    val source = state.eventSource ?: return

    androidx.compose.material3.AlertDialog(
        onDismissRequest = { state.closeEventSource() },
        title = { Text("Event Source") },
        text = {
            Text(
                source,
                style = MaterialTheme.typography.bodySmall,
                fontFamily = androidx.compose.ui.text.font.FontFamily.Monospace,
                modifier = Modifier
                    .verticalScroll(androidx.compose.foundation.rememberScrollState())
                    .horizontalScroll(androidx.compose.foundation.rememberScrollState()),
            )
        },
        confirmButton = {
            androidx.compose.material3.TextButton(onClick = { state.closeEventSource() }) {
                Text("Close")
            }
        },
    )
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

/// The bar over the timeline while messages are selected: the count and
/// what can be done with them all at once.
@Composable
private fun SelectionBar(state: CommuneState) {
    var forwardOpen by remember { mutableStateOf(false) }

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 4.dp, vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        IconButton(onClick = { state.clearSelection() }) {
            Icon(Icons.Filled.Close, contentDescription = "Leave selection")
        }
        Text(
            "${state.selectedIds.size} selected",
            style = MaterialTheme.typography.titleMedium,
            modifier = Modifier.weight(1f),
        )
        androidx.compose.material3.TextButton(onClick = { state.copySelectedText() }) {
            Text("Copy")
        }
        androidx.compose.material3.TextButton(onClick = { forwardOpen = true }) {
            Text("Forward")
        }
        if (state.selectedEvents().all { it.isOwn }) {
            androidx.compose.material3.TextButton(onClick = { state.removeSelected() }) {
                Text("Remove", color = MaterialTheme.colorScheme.error)
            }
        }
    }

    if (forwardOpen) {
        RoomPickerDialog(
            state,
            title = "Forward To",
            onPick = { roomId ->
                state.forwardSelected(roomId)
                forwardOpen = false
            },
            onDismiss = { forwardOpen = false },
        )
    }
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
        // Two joined members is the test, not whether anybody marked the
        // room a direct chat — the GTK `can_call` rule verbatim. Two
        // people in a room nobody tagged can still call each other, and a
        // direct chat that grew a third member cannot. (GTK also refuses
        // where it cannot send a message, which catches the server-notices
        // room; the Kotlin core has no permissions layer to ask yet.)
        if (room.joinedMembersCount == 2UL) {
            IconButton(onClick = { state.placeCallInRoom(room) }) {
                Icon(
                    androidx.compose.material.icons.Icons.Filled.Call,
                    contentDescription = "Call",
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            IconButton(onClick = { state.placeCallInRoom(room, video = true) }) {
                Icon(
                    androidx.compose.material.icons.Icons.Filled.Videocam,
                    contentDescription = "Video call",
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
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

/// The stable identity of a timeline item, for the list to anchor on
/// when history is prepended.
private fun timelineKey(item: FfiTimelineItem): Any = when (item) {
    is FfiTimelineItem.Event -> item.uniqueId
    is FfiTimelineItem.DateDivider -> "date-${item.timestamp}"
    is FfiTimelineItem.ReadMarker -> "read-marker"
    is FfiTimelineItem.TimelineStart -> "timeline-start"
}

@Composable
internal fun Timeline(
    state: CommuneState,
    room: FfiRoom,
    items: List<FfiTimelineItem>,
    modifier: Modifier,
    onOpenThread: ((String) -> Unit)? = null,
    live: Boolean = false,
) {
    val listState = rememberLazyListState()
    val roomId = room.roomId
    val currentItems by androidx.compose.runtime.rememberUpdatedState(items)

    // A room opens at the newest message. The only exception in the whole
    // screen is a notification tap, which asked for the oldest unread.
    var positioned by remember(roomId) { mutableStateOf(false) }
    var searchPages by remember(roomId) { mutableStateOf(0) }

    // Landing at the newest message is not one scroll: pictures measure
    // after they decode and grow the content under the viewport, which
    // would leave the view stranded above the end. So the bottom is a
    // state to hold — until the reader scrolls away themselves.
    var stickToBottom by remember(roomId) { mutableStateOf(!state.jumpToUnread) }

    // A notification jump holds onto the first unread it landed on: the
    // timeline's read marker can open stale (cached account data) and
    // relocate moments later, and the view follows it — until the user
    // scrolls somewhere themselves.
    var jumpAnchor by remember(roomId) { mutableStateOf<String?>(null) }
    var userScrolled by remember(roomId) { mutableStateOf(false) }
    LaunchedEffect(roomId) {
        listState.interactionSource.interactions.collect {
            if (it is androidx.compose.foundation.interaction.DragInteraction.Start) {
                userScrolled = true
                stickToBottom = false
            }
        }
    }

    // ...and a scroll that comes to rest on the newest message is the
    // reader choosing the bottom again. Without this the flag above is a
    // one-way latch: scroll up once and the room is never "at the bottom"
    // again, so leaving it remembers a position it should have forgotten.
    LaunchedEffect(roomId) {
        androidx.compose.runtime.snapshotFlow {
            listState.isScrollInProgress to listState.canScrollForward
        }.collect { (scrolling, canScrollDown) ->
            if (positioned && !scrolling && !canScrollDown) {
                stickToBottom = true
            }
        }
    }

    LaunchedEffect(items) {
        if (items.isEmpty()) return@LaunchedEffect

        // The one and only case that does not open at the newest message.
        if (!positioned && live && state.jumpToUnread) {
            // More pages may hold the marker; the timeline start, or
            // enough fruitless pages, means it is not coming.
            val exhausted = items.firstOrNull() is FfiTimelineItem.TimelineStart ||
                searchPages >= 4
            val marker = items.indexOfFirst { it is FfiTimelineItem.ReadMarker }
            when {
                marker >= 0 -> {
                    // Anchor on the first unread event, not the virtual
                    // marker: marking read removes the marker item, and a
                    // vanished anchor lets concurrent pagination drag the
                    // viewport off. The negative offset keeps the divider
                    // peeking in above it.
                    val firstUnread = (marker + 1 until items.size)
                        .firstOrNull { items[it] is FfiTimelineItem.Event }
                        ?: marker
                    listState.scrollToItem(firstUnread, -130)
                    jumpAnchor = (items.getOrNull(firstUnread)
                        as? FfiTimelineItem.Event)?.eventId
                    positioned = true
                    state.completeUnreadJump()
                }
                exhausted -> {
                    // The marker never turned up. Fall through to the
                    // bottom, which is where everything else lands.
                    positioned = true
                    stickToBottom = true
                    state.completeUnreadJump()
                }
                else -> {
                    searchPages += 1
                    state.paginateOlder()
                }
            }
            return@LaunchedEffect
        }

        positioned = true

        if (jumpAnchor != null && !userScrolled) {
            // The read marker settled somewhere else: follow it there.
            val marker = items.indexOfFirst { it is FfiTimelineItem.ReadMarker }
            if (marker >= 0) {
                val firstUnread = (marker + 1 until items.size)
                    .firstOrNull { items[it] is FfiTimelineItem.Event } ?: marker
                val id = (items.getOrNull(firstUnread)
                    as? FfiTimelineItem.Event)?.eventId
                if (id != null && id != jumpAnchor) {
                    listState.scrollToItem(firstUnread, -130)
                    jumpAnchor = id
                }
            }
        }
    }

    // Hold the bottom against everything that grows the content after
    // the fact — a picture that just decoded, a page that just landed.
    LaunchedEffect(roomId) {
        androidx.compose.runtime.snapshotFlow {
            stickToBottom to listState.canScrollForward
        }.collect { (sticking, canScrollDown) ->
            if (sticking && canScrollDown) {
                val last = currentItems.size - 1
                if (last >= 0) listState.scrollToItem(last)
            }
        }
    }

    if (live) {
        // Reading further back pulls more history in as the top nears,
        // until the start of the timeline is loaded.
        LaunchedEffect(roomId) {
            androidx.compose.runtime.snapshotFlow { listState.firstVisibleItemIndex }
                .collect { first ->
                    if (positioned && first <= 2 &&
                        currentItems.firstOrNull() !is FfiTimelineItem.TimelineStart
                    ) {
                        state.paginateOlder()
                    }
                }
        }

    }

    // The keyboard resizing the viewport must not hide the newest
    // messages: when it opens and the view was near the bottom, stay
    // pinned there, as any messenger does.
    val imeBottom = WindowInsets.ime
        .getBottom(androidx.compose.ui.platform.LocalDensity.current)
    LaunchedEffect(imeBottom > 0) {
        if (imeBottom > 0 && items.isNotEmpty()) {
            val lastVisible =
                listState.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: 0
            if (lastVisible >= items.size - 8) {
                listState.scrollToItem(items.size - 1)
            }
        }
    }

    LazyColumn(
        state = listState,
        modifier = modifier.fillMaxWidth(),
        contentPadding = androidx.compose.foundation.layout.PaddingValues(vertical = 8.dp),
    ) {
        items(items.size, key = { timelineKey(items[it]) }) { index ->
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
                            gallery = items,
                        )
                    } else {
                        StateLine(state, item)
                    }
                }
                is FfiTimelineItem.DateDivider ->
                    CenteredDivider(DATE.format(Date(item.timestamp.toLong())))
                // A visible line, and the anchor a notification jump
                // scrolls to — a zero-height marker cannot hold the
                // viewport while images around it measure in.
                is FfiTimelineItem.ReadMarker -> NewMessagesDivider()
                is FfiTimelineItem.TimelineStart ->
                    CenteredDivider("The conversation starts here")
            }
        }
    }
}

internal fun FfiEventKind.isMessageLike(): Boolean = when (this) {
    is FfiEventKind.Call,
    is FfiEventKind.Text,
    is FfiEventKind.Media,
    is FfiEventKind.Location,
    is FfiEventKind.UnableToDecrypt,
    is FfiEventKind.Redacted -> true
    is FfiEventKind.Membership,
    is FfiEventKind.ProfileChange,
    is FfiEventKind.OtherState,
    is FfiEventKind.Unsupported -> false
}

/// The authenticity shield of a message in an encrypted room: red for a
/// warning, grey for a caveat, as the GTK history draws them. A tap says
/// why, where the desktop shows a tooltip.
@Composable
private fun ShieldIcon(shield: io.github.steeb_k.commune.core.FfiShield) {
    val context = androidx.compose.ui.platform.LocalContext.current
    val message = shieldMessage(shield.code)
    Icon(
        if (shield.isWarning) {
            androidx.compose.material.icons.Icons.Outlined.GppBad
        } else {
            androidx.compose.material.icons.Icons.Outlined.GppMaybe
        },
        contentDescription = message,
        tint = if (shield.isWarning) {
            MaterialTheme.colorScheme.error
        } else {
            MaterialTheme.colorScheme.onSurfaceVariant
        },
        modifier = Modifier
            .size(16.dp)
            .clickable {
                android.widget.Toast.makeText(context, message, android.widget.Toast.LENGTH_LONG)
                    .show()
            },
    )
}

/// The sentence for a shield code — the GTK app's words.
internal fun shieldMessage(code: io.github.steeb_k.commune.core.FfiShieldCode): String =
    when (code) {
        io.github.steeb_k.commune.core.FfiShieldCode.AUTHENTICITY_NOT_GUARANTEED ->
            "The authenticity of this message cannot be guaranteed on this device."
        io.github.steeb_k.commune.core.FfiShieldCode.UNKNOWN_DEVICE ->
            "The device that sent this message is not known."
        io.github.steeb_k.commune.core.FfiShieldCode.UNSIGNED_DEVICE ->
            "The device that sent this message has not been verified by its owner."
        io.github.steeb_k.commune.core.FfiShieldCode.UNVERIFIED_IDENTITY ->
            "The sender of this message has not been verified."
        io.github.steeb_k.commune.core.FfiShieldCode.VERIFICATION_VIOLATION ->
            "The sender of this message was verified once, and has changed identity since."
        io.github.steeb_k.commune.core.FfiShieldCode.MISMATCHED_SENDER ->
            "The sender of this message does not match the device that encrypted it."
        io.github.steeb_k.commune.core.FfiShieldCode.SENT_IN_CLEAR ->
            "This message was not encrypted, in a room that is."
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
    // The list the bubble sits in: a picture opens with the list's other
    // pictures a swipe away.
    gallery: List<FfiTimelineItem> = emptyList(),
) {
    val own = event.isOwn
    val bubbleColor = if (own) {
        MaterialTheme.colorScheme.primary.copy(alpha = 0.25f)
    } else {
        MaterialTheme.colorScheme.onSurface.copy(alpha = 0.08f)
    }

    val callKind = event.kind as? FfiEventKind.Call
    if (callKind != null) {
        CallRow(event, callKind, own)
        return
    }

    val body = when (event.kind) {
        is FfiEventKind.Text -> event.body
        is FfiEventKind.Media -> event.body
        is FfiEventKind.Location -> event.body.ifBlank { "Shared a location" }
        is FfiEventKind.UnableToDecrypt -> "Could not decrypt this message"
        is FfiEventKind.Redacted -> "Message removed"
        else -> return
    }
    val muted = event.kind !is FfiEventKind.Text && event.kind !is FfiEventKind.Media &&
        event.kind !is FfiEventKind.Location

    val selected = state.selectMode && event.uniqueId in state.selectedIds
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .background(
                if (selected) {
                    MaterialTheme.colorScheme.primary.copy(alpha = 0.12f)
                } else {
                    androidx.compose.ui.graphics.Color.Transparent
                }
            )
            .padding(horizontal = 12.dp, vertical = 2.dp),
        horizontalArrangement = if (own) Arrangement.End else Arrangement.Start,
    ) {
        // The surface hugs the text; the reactions, the readers and the
        // thread chip follow below it on the same side, as the GTK row
        // places them outside the bubble.
        Column(
            modifier = Modifier.widthIn(max = 320.dp),
            horizontalAlignment = if (own) Alignment.End else Alignment.Start,
        ) {
            Column(
                modifier = Modifier
                    .clip(RoundedCornerShape(12.dp))
                    .background(bubbleColor)
                    .combinedClickable(
                        onClick = {
                            if (state.selectMode) state.toggleSelected(event.uniqueId)
                        },
                        onLongClick = {
                            if (state.selectMode) {
                                state.toggleSelected(event.uniqueId)
                            } else {
                                state.showActionSheet(event)
                            }
                        },
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
                // A voice message or audio file plays where it sits; video
                // still takes the whole screen, which is where video wants
                // to be.
                if (mediaKind == FfiMediaKind.AUDIO) {
                    AudioBubblePlayer(state, event.uniqueId)
                }
                if (mediaKind == FfiMediaKind.VIDEO) {
                    // The still the event carries, with the play button
                    // over it, as the GTK history shows a video; a video
                    // without one keeps the plain button. The video itself
                    // is fetched only when it is played.
                    var poster by remember(event.uniqueId) {
                        mutableStateOf<String?>(null)
                    }
                    LaunchedEffect(event.uniqueId) {
                        poster = try {
                            state.app.getTimelineMediaThumbnail(room.roomId, event.uniqueId, 720u)
                        } catch (_: Exception) {
                            null
                        }
                    }
                    val posterPath = poster
                    if (posterPath != null) {
                        Box(
                            modifier = Modifier
                                .padding(bottom = 4.dp)
                                .clip(RoundedCornerShape(8.dp))
                                .clickable { state.openMediaPlayer(event.uniqueId) },
                            contentAlignment = Alignment.Center,
                        ) {
                            MediaImage(
                                posterPath,
                                contentDescription = event.body,
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .heightIn(max = 420.dp),
                                targetSizePx = 720,
                            )
                            Icon(
                                androidx.compose.ui.res.painterResource(
                                    io.github.steeb_k.commune.R.drawable.ic_play_symbolic
                                ),
                                contentDescription = "Play",
                                tint = androidx.compose.ui.graphics.Color.White,
                                modifier = Modifier
                                    .size(56.dp)
                                    .clip(CircleShape)
                                    .background(
                                        androidx.compose.ui.graphics.Color.Black.copy(alpha = 0.45f)
                                    )
                                    .padding(12.dp),
                            )
                        }
                    } else {
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
                            Text("Play video", style = MaterialTheme.typography.bodyMedium)
                        }
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
                                .clickable {
                                    state.openTimelineImage(gallery, event.uniqueId, path)
                                }
                                .padding(bottom = 4.dp),
                        )
                    }
                }

                Row(verticalAlignment = Alignment.Bottom) {
                    // A text message is the document the core built from its
                    // formatted body: markup, links, mentions and emoticons
                    // drawn as the GTK history draws them. A file is its
                    // name beside a save button, as the GTK file row.
                    // Everything else shows its plain body.
                    if (mediaKind == FfiMediaKind.FILE) {
                        Row(
                            verticalAlignment = Alignment.CenterVertically,
                            modifier = Modifier.weight(1f, fill = false),
                        ) {
                            Icon(
                                androidx.compose.material.icons.Icons.AutoMirrored.Filled.InsertDriveFile,
                                contentDescription = null,
                                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                                modifier = Modifier.size(20.dp),
                            )
                            Spacer(Modifier.size(6.dp))
                            Text(
                                body,
                                style = MaterialTheme.typography.bodyLarge,
                                color = MaterialTheme.colorScheme.onSurface,
                                maxLines = 2,
                                overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
                                modifier = Modifier.weight(1f, fill = false),
                            )
                            Spacer(Modifier.size(4.dp))
                            IconButton(
                                onClick = { state.saveEventMedia(event) },
                                modifier = Modifier.size(36.dp),
                            ) {
                                Icon(
                                    androidx.compose.ui.res.painterResource(
                                        io.github.steeb_k.commune.R.drawable.ic_save_symbolic
                                    ),
                                    contentDescription = "Save File",
                                    tint = MaterialTheme.colorScheme.primary,
                                )
                            }
                        }
                    } else if (event.rich.isNotEmpty()) {
                        RichBody(
                            state,
                            event.rich,
                            style = MaterialTheme.typography.bodyLarge,
                            color = MaterialTheme.colorScheme.onSurface,
                            modifier = Modifier.weight(1f, fill = false),
                        )
                    } else {
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
                    }
                    if (event.isEdited) {
                        Spacer(Modifier.size(4.dp))
                        Text(
                            "(edited)",
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    event.shield?.let { shield ->
                        Spacer(Modifier.size(4.dp))
                        ShieldIcon(shield)
                    }
                }
                event.previewUrl?.let { url -> UrlPreviewCard(state, url) }

                (event.kind as? FfiEventKind.Location)?.let { location ->
                    val context = androidx.compose.ui.platform.LocalContext.current
                    // The place itself, the way the GTK location viewer shows
                    // it. Tapping either the map or the row below opens it in
                    // whatever maps application is installed.
                    LocationMap(
                        location.geoUri,
                        modifier = Modifier
                            .clip(RoundedCornerShape(8.dp))
                            .clickable {
                                try {
                                    context.startActivity(
                                        android.content.Intent(
                                            android.content.Intent.ACTION_VIEW,
                                            android.net.Uri.parse(location.geoUri),
                                        )
                                    )
                                } catch (_: Exception) {
                                    // No maps app; the row below still shows.
                                }
                            },
                    )
                    Spacer(Modifier.size(4.dp))
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        modifier = Modifier
                            .clip(RoundedCornerShape(8.dp))
                            .background(MaterialTheme.colorScheme.surfaceVariant)
                            .clickable {
                                try {
                                    context.startActivity(
                                        android.content.Intent(
                                            android.content.Intent.ACTION_VIEW,
                                            android.net.Uri.parse(location.geoUri),
                                        )
                                    )
                                } catch (_: Exception) {
                                    // No maps app; the coordinates in the body
                                    // are all there is to offer.
                                }
                            }
                            .padding(horizontal = 10.dp, vertical = 8.dp),
                    ) {
                        Icon(
                            androidx.compose.material.icons.Icons.Filled.Place,
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        Spacer(Modifier.size(6.dp))
                        Text("Open in Maps", style = MaterialTheme.typography.bodyMedium)
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
            }

            ReactionChips(state, event)

            if (event.isOwn && event.receipts.isNotEmpty()) {
                // Tapping the little avatars names the readers.
                var receiptsOpen by remember { mutableStateOf(false) }
                if (receiptsOpen) {
                    androidx.compose.material3.AlertDialog(
                        onDismissRequest = { receiptsOpen = false },
                        title = { Text("Read By") },
                        text = {
                            Column {
                                for (userId in event.receipts) {
                                    val name = state.composerMembers
                                        .find { it.userId == userId }
                                        ?.displayName
                                        ?: localpart(userId)
                                    Row(
                                        modifier = Modifier.padding(vertical = 4.dp),
                                        verticalAlignment = Alignment.CenterVertically,
                                    ) {
                                        InitialsAvatar(
                                            identifier = userId,
                                            name = name,
                                            size = 24.dp,
                                        )
                                        Spacer(Modifier.size(8.dp))
                                        Column {
                                            Text(
                                                name,
                                                style = MaterialTheme.typography.bodyLarge,
                                            )
                                            Text(
                                                userId,
                                                style = MaterialTheme.typography.bodySmall,
                                                color =
                                                    MaterialTheme.colorScheme.onSurfaceVariant,
                                            )
                                        }
                                    }
                                }
                            }
                        },
                        confirmButton = {
                            androidx.compose.material3.TextButton(
                                onClick = { receiptsOpen = false },
                            ) { Text("Close") }
                        },
                    )
                }
                Row(
                    modifier = Modifier
                        .padding(top = 2.dp)
                        .clickable { receiptsOpen = true },
                ) {
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
internal fun StateLine(state: CommuneState, event: FfiTimelineItem.Event) {
    // A pending invite can be taken back from its own line.
    val invitedUser = (event.kind as? FfiEventKind.Membership)
        ?.takeIf { it.change == FfiMembershipChange.INVITED }
        ?.user
    var revokeOpen by remember { mutableStateOf(false) }

    Text(
        stateSentence(event),
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier
            .fillMaxWidth()
            .let { modifier ->
                if (invitedUser != null) {
                    modifier.combinedClickable(
                        onClick = {},
                        onLongClick = { revokeOpen = true },
                    )
                } else {
                    modifier
                }
            }
            .padding(horizontal = 16.dp, vertical = 4.dp),
        textAlign = androidx.compose.ui.text.style.TextAlign.Center,
    )

    if (revokeOpen && invitedUser != null) {
        androidx.compose.material3.AlertDialog(
            onDismissRequest = { revokeOpen = false },
            title = { Text("Revoke Invite?") },
            text = { Text("Take back the invitation of $invitedUser?") },
            confirmButton = {
                androidx.compose.material3.TextButton(onClick = {
                    state.kickUser(invitedUser) { }
                    revokeOpen = false
                }) { Text("Revoke", color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = {
                androidx.compose.material3.TextButton(onClick = { revokeOpen = false }) {
                    Text("Cancel")
                }
            },
        )
    }
}

/// An audio message played inside its bubble: play/pause and a
/// progress line, no jump to a full screen for a few seconds of sound.
@Composable
private fun AudioBubblePlayer(state: CommuneState, uniqueId: String) {
    val context = androidx.compose.ui.platform.LocalContext.current
    var path by remember(uniqueId) { mutableStateOf<String?>(null) }
    var player by remember(uniqueId) {
        mutableStateOf<androidx.media3.exoplayer.ExoPlayer?>(null)
    }
    var playing by remember(uniqueId) { mutableStateOf(false) }
    var position by remember(uniqueId) { mutableStateOf(0f) }
    var loading by remember(uniqueId) { mutableStateOf(false) }

    // The player is this bubble's; it goes when the bubble does.
    androidx.compose.runtime.DisposableEffect(uniqueId) {
        onDispose {
            player?.release()
            player = null
        }
    }

    // While it plays, follow the position for the progress line.
    LaunchedEffect(playing) {
        while (playing) {
            player?.let { current ->
                val duration = current.duration
                position = if (duration > 0) {
                    (current.currentPosition.toFloat() / duration).coerceIn(0f, 1f)
                } else {
                    0f
                }
                if (!current.isPlaying && current.playbackState ==
                    androidx.media3.common.Player.STATE_ENDED
                ) {
                    playing = false
                    position = 0f
                    current.seekTo(0)
                }
            }
            kotlinx.coroutines.delay(200)
        }
    }

    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier
            .padding(bottom = 4.dp)
            .clip(RoundedCornerShape(8.dp))
            .background(MaterialTheme.colorScheme.surfaceVariant)
            .padding(horizontal = 8.dp, vertical = 6.dp),
    ) {
        IconButton(
            onClick = {
                val existing = player
                if (existing != null) {
                    if (existing.isPlaying) {
                        existing.pause()
                        playing = false
                    } else {
                        existing.play()
                        playing = true
                    }
                    return@IconButton
                }
                // First press fetches the file, then starts it.
                loading = true
                state.fetchMediaPath(uniqueId) { fetched ->
                    loading = false
                    path = fetched
                    if (fetched == null) return@fetchMediaPath
                    val created = androidx.media3.exoplayer.ExoPlayer.Builder(context).build()
                    created.setMediaItem(
                        androidx.media3.common.MediaItem.fromUri(
                            android.net.Uri.fromFile(java.io.File(fetched))
                        )
                    )
                    created.prepare()
                    created.play()
                    player = created
                    playing = true
                }
            },
            enabled = !loading,
        ) {
            Icon(
                if (playing) {
                    androidx.compose.material.icons.Icons.Filled.Pause
                } else {
                    androidx.compose.material.icons.Icons.Filled.PlayArrow
                },
                contentDescription = if (playing) "Pause" else "Play",
                tint = MaterialTheme.colorScheme.primary,
            )
        }
        androidx.compose.material3.LinearProgressIndicator(
            progress = { position },
            modifier = Modifier.width(140.dp),
        )
    }
}

/// A call that happened here, and what became of it — the wording of
/// the application's call_row.
@Composable
private fun CallRow(
    event: FfiTimelineItem.Event,
    kind: FfiEventKind.Call,
    own: Boolean,
) {
    val name = event.senderDisplayName ?: localpart(event.sender)
    val text = when (kind.outcome) {
        io.github.steeb_k.commune.core.FfiCallOutcome.ANSWERED -> "Call ended."
        io.github.steeb_k.commune.core.FfiCallOutcome.DECLINED -> "Call declined."
        io.github.steeb_k.commune.core.FfiCallOutcome.MISSED,
        io.github.steeb_k.commune.core.FfiCallOutcome.RINGING ->
            if (own) "No answer." else "Missed call from $name."
        // A call from before this session was running: nothing here
        // knows how it ended, and inventing an answer is worse than
        // saying only what the invite says.
        null -> when {
            own && kind.hasVideo -> "Outgoing video call."
            own -> "Outgoing call."
            kind.hasVideo -> "Incoming video call from $name."
            else -> "Incoming call from $name."
        }
    }

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 6.dp),
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            if (kind.hasVideo) {
                androidx.compose.material.icons.Icons.Filled.Videocam
            } else {
                androidx.compose.material.icons.Icons.Filled.Call
            },
            contentDescription = null,
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.size(18.dp),
        )
        Spacer(Modifier.size(8.dp))
        Text(
            text,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

/// The unread boundary: the date divider's shape in the accent color.
@Composable
internal fun NewMessagesDivider() {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        HorizontalDivider(
            modifier = Modifier.weight(1f),
            color = MaterialTheme.colorScheme.primary,
        )
        Text(
            "New messages",
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(horizontal = 12.dp),
        )
        HorizontalDivider(
            modifier = Modifier.weight(1f),
            color = MaterialTheme.colorScheme.primary,
        )
    }
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
    onVoice: (() -> Unit)? = null,
    onLocation: (() -> Unit)? = null,
    members: List<io.github.steeb_k.commune.core.FfiMember> = emptyList(),
    emoticons: List<io.github.steeb_k.commune.core.FfiSticker> = emptyList(),
    // The room's saved draft, and where to keep it: the GTK composer
    // keeps one per room in the SDK's store, and so does this.
    loadDraft: (suspend () -> String?)? = null,
    saveDraft: (suspend (String) -> Unit)? = null,
    // The body of the message being edited, while one is: the GTK composer
    // puts it in the entry when the edit starts and empties the entry when
    // the edit ends, sent or abandoned.
    editBody: String? = null,
    // Text shared from another app, appended to whatever is being written
    // so it can be finished before it goes; taken once.
    prefill: String? = null,
    onPrefillTaken: () -> Unit = {},
) {
    var draft by remember {
        mutableStateOf(androidx.compose.ui.text.input.TextFieldValue(""))
    }
    var wasEditing by remember { mutableStateOf(false) }
    LaunchedEffect(editBody) {
        if (editBody != null) {
            draft = androidx.compose.ui.text.input.TextFieldValue(
                editBody,
                androidx.compose.ui.text.TextRange(editBody.length),
            )
            wasEditing = true
        } else if (wasEditing) {
            draft = androidx.compose.ui.text.input.TextFieldValue("")
            wasEditing = false
        }
    }
    var draftLoaded by remember { mutableStateOf(loadDraft == null) }
    LaunchedEffect(Unit) {
        val saved = loadDraft?.invoke()
        if (saved != null && draft.text.isEmpty()) {
            draft = androidx.compose.ui.text.input.TextFieldValue(
                saved,
                androidx.compose.ui.text.TextRange(saved.length),
            )
        }
        draftLoaded = true
    }
    // Shared text goes after the saved draft, once that is in, so a
    // half-written message is added to rather than replaced.
    LaunchedEffect(prefill, draftLoaded) {
        val extra = prefill ?: return@LaunchedEffect
        if (!draftLoaded) return@LaunchedEffect
        val current = draft.text
        val joined = if (current.isBlank()) extra else current.trimEnd() + "\n" + extra
        draft = androidx.compose.ui.text.input.TextFieldValue(
            joined,
            androidx.compose.ui.text.TextRange(joined.length),
        )
        onPrefillTaken()
    }
    LaunchedEffect(draft.text) {
        if (!draftLoaded) return@LaunchedEffect
        // A moment after typing stops, not every keystroke.
        kotlinx.coroutines.delay(800)
        try {
            saveDraft?.invoke(draft.text)
        } catch (_: Exception) {
        }
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

    // Emoticon completion: `:shortcode` completes from the image packs.
    val emoticonQuery = currentWord
        .takeIf { it.startsWith(":") && it.length > 1 && !it.endsWith(":") }
        ?.drop(1)
    val emoticonMatches = emoticonQuery?.let { query ->
        emoticons.filter { it.shortcode.startsWith(query, ignoreCase = true) }.take(4)
    }.orEmpty()

    if (emoticonMatches.isNotEmpty()) {
        Row(modifier = Modifier.padding(horizontal = 16.dp, vertical = 2.dp)) {
            for (emoticon in emoticonMatches) {
                Text(
                    ":${emoticon.shortcode}:",
                    style = MaterialTheme.typography.labelLarge,
                    color = MaterialTheme.colorScheme.primary,
                    modifier = Modifier
                        .padding(end = 8.dp)
                        .clip(RoundedCornerShape(12.dp))
                        .background(MaterialTheme.colorScheme.surfaceVariant)
                        .clickable {
                            val text = draft.text.dropLast(currentWord.length) +
                                ":" + emoticon.shortcode + ": "
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
        var attachMenuOpen by remember { mutableStateOf(false) }
        Box {
            IconButton(
                onClick = { attachMenuOpen = true },
                enabled = onAttach != null || onLocation != null,
            ) {
                Icon(
                    androidx.compose.ui.res.painterResource(
                        io.github.steeb_k.commune.R.drawable.ic_attachment_symbolic
                    ),
                    contentDescription = "Attach",
                )
            }
            androidx.compose.material3.DropdownMenu(
                expanded = attachMenuOpen,
                onDismissRequest = { attachMenuOpen = false },
            ) {
                if (onAttach != null) {
                    androidx.compose.material3.DropdownMenuItem(
                        text = { Text("Attach File") },
                        onClick = {
                            attachMenuOpen = false
                            onAttach()
                        },
                    )
                }
                if (onLocation != null) {
                    androidx.compose.material3.DropdownMenuItem(
                        text = { Text("Share Location") },
                        onClick = {
                            attachMenuOpen = false
                            onLocation()
                        },
                    )
                }
            }
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
            keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(
                capitalization = androidx.compose.ui.text.input.KeyboardCapitalization.Sentences,
                autoCorrectEnabled = true,
            ),
        )
        Spacer(Modifier.size(6.dp))
        Box(
            modifier = Modifier
                .size(48.dp)
                .clip(CircleShape)
                .background(MaterialTheme.colorScheme.primary),
            contentAlignment = Alignment.Center,
        ) {
            if (draft.text.isBlank() && onVoice != null) {
                IconButton(onClick = { onVoice() }) {
                    Icon(
                        androidx.compose.material.icons.Icons.Filled.Mic,
                        contentDescription = "Record a voice message",
                        tint = MaterialTheme.colorScheme.onPrimary,
                    )
                }
            } else {
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
}

/// While a voice message records: discard on the left, a live red dot
/// and elapsed time, send on the right.
@Composable
private fun RecordingBar(state: CommuneState) {
    var elapsed by remember { mutableStateOf(0L) }
    LaunchedEffect(Unit) {
        while (true) {
            elapsed =
                (android.os.SystemClock.elapsedRealtime() - state.recordingStarted) / 1000
            kotlinx.coroutines.delay(250)
        }
    }

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 8.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        IconButton(onClick = { state.stopVoiceRecording(send = false) }) {
            Icon(Icons.Filled.Close, contentDescription = "Discard recording")
        }
        Box(
            modifier = Modifier
                .size(10.dp)
                .clip(CircleShape)
                .background(MaterialTheme.colorScheme.error),
        )
        Spacer(Modifier.size(8.dp))
        Text(
            "Recording…  %d:%02d".format(elapsed / 60, elapsed % 60),
            style = MaterialTheme.typography.bodyLarge,
            modifier = Modifier.weight(1f),
        )
        Box(
            modifier = Modifier
                .size(48.dp)
                .clip(CircleShape)
                .background(MaterialTheme.colorScheme.primary),
            contentAlignment = Alignment.Center,
        ) {
            IconButton(onClick = { state.stopVoiceRecording(send = true) }) {
                Icon(
                    androidx.compose.ui.res.painterResource(
                        io.github.steeb_k.commune.R.drawable.ic_send_symbolic
                    ),
                    contentDescription = "Send voice message",
                    tint = MaterialTheme.colorScheme.onPrimary,
                )
            }
        }
    }
}
