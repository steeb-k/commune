// A thread: the room's bubble timeline focused on one root event, with its
// own composer — the GTK thread view's layout, opened as a full screen the
// way Android navigates rather than a side pane.
package io.github.steeb_k.commune.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import io.github.steeb_k.commune.CommuneState
import io.github.steeb_k.commune.core.FfiRoom

@Composable
fun ThreadScreen(state: CommuneState, room: FfiRoom) {
    Column(modifier = Modifier.fillMaxSize().imePadding()) {
        ThreadHeader(room, onBack = { state.closeThread() })
        Timeline(state, room, items = state.threadItems, modifier = Modifier.weight(1f))
        Composer(
            onSend = { state.sendInThread(it) },
            onTyping = { state.setTyping(it) },
        )
    }
}

@Composable
private fun ThreadHeader(room: FfiRoom, onBack: () -> Unit) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 4.dp, vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        IconButton(onClick = onBack) {
            Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
        }
        Column(modifier = Modifier.weight(1f)) {
            Text("Thread", style = MaterialTheme.typography.titleMedium, maxLines = 1)
            Text(
                roomName(room),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
            )
        }
        Spacer(Modifier.size(8.dp))
    }
}
