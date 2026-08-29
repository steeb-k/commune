// The image packs this account owns: the GTK account settings'
// image_packs_page, which edits the packs living in Commune's own
// packs room — create one, name it, put pictures in it with the
// shortcodes that summon them, take them out again.
package io.github.steeb_k.commune.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Close
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import io.github.steeb_k.commune.CommuneState

@Composable
fun ImagePacksScreen(state: CommuneState) {
    var createOpen by remember { mutableStateOf(false) }
    var renaming by remember { mutableStateOf<String?>(null) }
    var deleting by remember { mutableStateOf<String?>(null) }

    LaunchedEffect(Unit) { state.loadOwnedPacks() }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState()),
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 4.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconButton(onClick = { state.closeImagePacks() }) {
                Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
            }
            Text(
                "Image Packs",
                style = MaterialTheme.typography.titleMedium,
                modifier = Modifier.weight(1f),
            )
            IconButton(onClick = { createOpen = true }) {
                Icon(Icons.Filled.Add, contentDescription = "New pack")
            }
        }

        if (state.ownedPacks.isEmpty()) {
            Box(
                modifier = Modifier.fillMaxWidth().padding(vertical = 48.dp),
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    "No packs of your own yet",
                    style = MaterialTheme.typography.bodyLarge,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }

        for (pack in state.ownedPacks) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    pack.name,
                    style = MaterialTheme.typography.titleSmall,
                    color = MaterialTheme.colorScheme.primary,
                    modifier = Modifier.weight(1f),
                )
                TextButton(onClick = { renaming = pack.stateKey }) { Text("Rename") }
                TextButton(onClick = { state.pickPackImage(pack.stateKey) }) { Text("Add") }
                IconButton(onClick = { deleting = pack.stateKey }) {
                    Icon(
                        Icons.Filled.Close,
                        contentDescription = "Delete ${pack.name}",
                        tint = MaterialTheme.colorScheme.error,
                    )
                }
            }
            for (image in pack.images) {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(start = 32.dp, end = 16.dp, bottom = 4.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    PackImageThumb(state, image.url)
                    Spacer(Modifier.size(12.dp))
                    Column(modifier = Modifier.weight(1f)) {
                        Text(
                            ":${image.shortcode}:",
                            style = MaterialTheme.typography.bodyLarge,
                        )
                        Text(
                            image.body,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    IconButton(
                        onClick = { state.removePackImage(pack.stateKey, image.shortcode) },
                    ) {
                        Icon(
                            Icons.Filled.Close,
                            contentDescription = "Remove ${image.shortcode}",
                            tint = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
        }

        state.packsError?.let { error ->
            Text(
                error,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.error,
                modifier = Modifier.padding(16.dp),
            )
        }
    }

    if (createOpen) {
        TextPromptDialog(
            title = "New Pack",
            label = "Pack name",
            confirm = "Create",
            onConfirm = {
                state.createImagePack(it)
                createOpen = false
            },
            onDismiss = { createOpen = false },
        )
    }
    renaming?.let { stateKey ->
        TextPromptDialog(
            title = "Rename Pack",
            label = "Pack name",
            confirm = "Rename",
            onConfirm = {
                state.renameImagePack(stateKey, it)
                renaming = null
            },
            onDismiss = { renaming = null },
        )
    }
    deleting?.let { stateKey ->
        AlertDialog(
            onDismissRequest = { deleting = null },
            title = { Text("Delete Pack?") },
            text = {
                Text("The pack and its images go, and it stops being available everywhere.")
            },
            confirmButton = {
                TextButton(onClick = {
                    state.deleteImagePack(stateKey)
                    deleting = null
                }) { Text("Delete", color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = {
                TextButton(onClick = { deleting = null }) { Text("Cancel") }
            },
        )
    }
    state.pendingPackImage?.let { pending ->
        TextPromptDialog(
            title = "Shortcode",
            label = "Shortcode (without colons)",
            confirm = "Add",
            onConfirm = { state.confirmPackImage(it) },
            onDismiss = { state.cancelPackImage() },
        )
        // The pending image's own name is a decent default hint.
        Unit
    }
}

/// One pack image's picture, fetched by its mxc URI.
@Composable
private fun PackImageThumb(state: CommuneState, mxcUri: String) {
    var path by remember(mxcUri) { mutableStateOf<String?>(null) }
    LaunchedEffect(mxcUri) { path = state.fetchMxcPath(mxcUri) }

    val current = path
    if (current == null) {
        Box(modifier = Modifier.size(40.dp))
    } else {
        MediaImage(
            current,
            contentDescription = null,
            modifier = Modifier.size(40.dp),
            targetSizePx = 120,
            fill = true,
        )
    }
}

/// A one-field dialog: the shape every pack edit asks for.
@Composable
private fun TextPromptDialog(
    title: String,
    label: String,
    confirm: String,
    onConfirm: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    var text by remember { mutableStateOf("") }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = {
            OutlinedTextField(
                value = text,
                onValueChange = { text = it },
                label = { Text(label) },
                singleLine = true,
            )
        },
        confirmButton = {
            TextButton(
                enabled = text.isNotBlank(),
                onClick = { onConfirm(text.trim()) },
            ) { Text(confirm) }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("Cancel") }
        },
    )
}
