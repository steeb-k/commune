package io.github.steeb_k.commune.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import io.github.steeb_k.commune.CommuneState
import io.github.steeb_k.commune.core.FfiMatrixLink

/// What a Matrix link asks before anything happens: a room we are not in
/// is joined, a user is chatted with — the GTK app's room preview and
/// user profile dialog, reduced to the one action each offers here.
@Composable
fun MatrixLinkDialog(state: CommuneState) {
    val link = state.pendingLink ?: return

    val (title, subject, confirm) = when (link) {
        is FfiMatrixLink.Room -> Triple("Join Room", link.roomIdOrAlias, "Join")
        is FfiMatrixLink.User -> Triple("Direct Chat", link.userId, "Chat")
    }

    AlertDialog(
        onDismissRequest = { state.dismissLink() },
        title = { Text(title) },
        text = {
            Column {
                Text(subject, style = MaterialTheme.typography.bodyLarge)
                state.conversationError?.let { error ->
                    Text(
                        error,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.error,
                    )
                }
            }
        },
        confirmButton = {
            Button(
                enabled = !state.conversationBusy,
                onClick = {
                    when (link) {
                        // The permalink form keeps the `via` servers the
                        // link carried, which the core's join reads.
                        is FfiMatrixLink.Room -> state.joinRoom(permalink(link)) {
                            state.dismissLink()
                        }
                        is FfiMatrixLink.User -> state.startDirectChat(link.userId) {
                            state.dismissLink()
                        }
                    }
                },
            ) {
                Text(if (state.conversationBusy) "…" else confirm)
            }
        },
        dismissButton = {
            TextButton(onClick = { state.dismissLink() }) { Text("Cancel") }
        },
    )
}

/// The matrix.to permalink of a room link, `via` servers included.
private fun permalink(link: FfiMatrixLink.Room): String {
    val via = link.via.joinToString("&") { "via=${android.net.Uri.encode(it)}" }
    val query = if (via.isEmpty()) "" else "?$via"
    return "https://matrix.to/#/${android.net.Uri.encode(link.roomIdOrAlias)}$query"
}
