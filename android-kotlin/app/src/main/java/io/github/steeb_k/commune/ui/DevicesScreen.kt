// The account's sessions: this one first, then the rest by recency,
// each renameable and sign-out-able (the server asks for the password
// again for that).
package io.github.steeb_k.commune.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import io.github.steeb_k.commune.CommuneState
import io.github.steeb_k.commune.core.FfiDevice
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

private val SEEN = SimpleDateFormat("MMM d, HH:mm", Locale.getDefault())

@Composable
fun DevicesScreen(state: CommuneState) {
    var acting by remember { mutableStateOf<FfiDevice?>(null) }

    Column(modifier = Modifier.fillMaxSize()) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 4.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconButton(onClick = { state.closeDevices() }) {
                Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
            }
            Text("Sessions", style = MaterialTheme.typography.titleMedium)
        }

        when {
            state.devices.isEmpty() && state.devicesBusy ->
                LoadingFace(modifier = Modifier.fillMaxSize())

            else -> LazyColumn(modifier = Modifier.fillMaxSize()) {
                items(state.devices.size, key = { state.devices[it].deviceId }) { index ->
                    DeviceRow(state.devices[index], onClick = { acting = state.devices[index] })
                }
            }
        }
    }

    acting?.let { device ->
        DeviceSheet(state, device, onDismiss = { acting = null })
    }
}

@Composable
private fun DeviceRow(device: FfiDevice, onClick: () -> Unit) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    device.displayName ?: device.deviceId,
                    style = MaterialTheme.typography.bodyLarge,
                    maxLines = 1,
                )
                if (device.isCurrent) {
                    Spacer(Modifier.size(8.dp))
                    Text(
                        "This session",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.primary,
                    )
                }
            }
            Text(
                listOfNotNull(
                    device.deviceId,
                    if (device.isVerified) "verified" else "not verified",
                    device.lastSeenTs?.let { SEEN.format(Date(it.toLong())) },
                ).joinToString(" · "),
                style = MaterialTheme.typography.bodySmall,
                color = if (device.isVerified) {
                    MaterialTheme.colorScheme.onSurfaceVariant
                } else {
                    MaterialTheme.colorScheme.error
                },
            )
        }
    }
}

/// Rename, or sign the session out with the password.
@Composable
private fun DeviceSheet(state: CommuneState, device: FfiDevice, onDismiss: () -> Unit) {
    var name by remember { mutableStateOf(device.displayName ?: "") }
    var password by remember { mutableStateOf("") }
    var error by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(device.displayName ?: device.deviceId) },
        text = {
            Column {
                OutlinedTextField(
                    value = name,
                    onValueChange = { name = it },
                    label = { Text("Session name") },
                    singleLine = true,
                )
                if (!device.isCurrent) {
                    OutlinedTextField(
                        value = password,
                        onValueChange = { password = it },
                        label = { Text("Password, to sign it out") },
                        singleLine = true,
                        visualTransformation =
                            androidx.compose.ui.text.input.PasswordVisualTransformation(),
                    )
                }
                error?.let {
                    Text(
                        it,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.error,
                    )
                }
            }
        },
        confirmButton = {
            Row {
                if (!device.isCurrent) {
                    TextButton(
                        enabled = password.isNotBlank() && !busy,
                        onClick = {
                            busy = true
                            state.signOutDevice(device.deviceId, password) { failure ->
                                busy = false
                                if (failure == null) onDismiss() else error = failure
                            }
                        },
                    ) { Text("Sign Out", color = MaterialTheme.colorScheme.error) }
                }
                TextButton(
                    enabled = name.isNotBlank() && !busy,
                    onClick = {
                        busy = true
                        state.renameDevice(device.deviceId, name) { failure ->
                            busy = false
                            if (failure == null) onDismiss() else error = failure
                        }
                    },
                ) { Text("Rename") }
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("Close") }
        },
    )
}
