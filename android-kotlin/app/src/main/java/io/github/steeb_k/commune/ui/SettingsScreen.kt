// Account settings, the first slice: the three toggles the core already
// persists — notifications, public read receipts, typing — grouped as the
// GTK Account Settings groups them.
package io.github.steeb_k.commune.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.TextButton
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.text.font.FontFamily
import io.github.steeb_k.commune.CommuneState
import io.github.steeb_k.commune.core.FfiRecoveryState

@Composable
fun SettingsScreen(state: CommuneState) {
    val settings = state.settings

    Column(modifier = Modifier.fillMaxSize()) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 4.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconButton(onClick = { state.closeSettings() }) {
                Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
            }
            Text("Account Settings", style = MaterialTheme.typography.titleMedium)
        }

        val userId = state.ownUserId
        if (userId != null) {
            Text(
                userId,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
            )
        }

        SettingsGroup("Notifications")
        SettingSwitch(
            title = "Enable Notifications for This Account",
            checked = settings?.notificationsEnabled == true,
            onChange = { state.setNotificationsEnabled(it) },
        )

        SettingsGroup("Safety")
        SettingSwitch(
            title = "Send Read Receipts",
            subtitle = "Turned off, receipts are still sent privately",
            checked = settings?.publicReadReceiptsEnabled == true,
            onChange = { state.setPublicReadReceiptsEnabled(it) },
        )
        SettingSwitch(
            title = "Send Typing Notifications",
            checked = settings?.typingEnabled == true,
            onChange = { state.setTypingEnabled(it) },
        )

        SettingsGroup("Encryption")
        RecoveryRow(state)
    }
}

/// The recovery row: where recovery stands, and the action that moves it
/// forward — set it up, or complete it with the key.
@Composable
private fun RecoveryRow(state: CommuneState) {
    var keyDialog by remember { mutableStateOf(false) }

    val (label, action) = when (state.recoveryState) {
        FfiRecoveryState.ENABLED -> "Recovery is set up" to null
        FfiRecoveryState.DISABLED -> "Recovery is not set up" to "Set Up"
        FfiRecoveryState.INCOMPLETE -> "Recovery key needed" to "Enter Key"
        FfiRecoveryState.UNKNOWN -> "Checking recovery…" to null
    }

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text("Account Recovery", style = MaterialTheme.typography.bodyLarge)
            Text(
                label,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            state.recoveryError?.let { error ->
                Text(
                    error,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.error,
                )
            }
        }
        if (action != null) {
            TextButton(
                enabled = !state.recoveryBusy,
                onClick = {
                    if (state.recoveryState == FfiRecoveryState.DISABLED) {
                        state.enableRecovery()
                    } else {
                        keyDialog = true
                    }
                },
            ) {
                Text(if (state.recoveryBusy) "…" else action)
            }
        }
    }

    state.recoveryKey?.let { key ->
        AlertDialog(
            onDismissRequest = { state.dismissRecoveryKey() },
            title = { Text("Your Recovery Key") },
            text = {
                Column {
                    Text(
                        "Write this key down somewhere safe. It is the only way " +
                            "to regain your message history on a new device.",
                        style = MaterialTheme.typography.bodyMedium,
                    )
                    Spacer(Modifier.size(12.dp))
                    Text(
                        key,
                        style = MaterialTheme.typography.bodyLarge,
                        fontFamily = FontFamily.Monospace,
                    )
                }
            },
            confirmButton = {
                TextButton(onClick = { state.dismissRecoveryKey() }) { Text("Done") }
            },
        )
    }

    if (keyDialog) {
        var input by remember { mutableStateOf("") }
        AlertDialog(
            onDismissRequest = { keyDialog = false },
            title = { Text("Enter Recovery Key") },
            text = {
                OutlinedTextField(
                    value = input,
                    onValueChange = { input = it },
                    placeholder = { Text("EsT3 ...") },
                    singleLine = true,
                )
            },
            confirmButton = {
                TextButton(
                    enabled = input.isNotBlank() && !state.recoveryBusy,
                    onClick = {
                        state.recover(input)
                        keyDialog = false
                    },
                ) { Text("Recover") }
            },
            dismissButton = {
                TextButton(onClick = { keyDialog = false }) { Text("Cancel") }
            },
        )
    }
}

@Composable
private fun SettingsGroup(title: String) {
    Text(
        title,
        style = MaterialTheme.typography.labelLarge,
        color = MaterialTheme.colorScheme.primary,
        modifier = Modifier.padding(start = 16.dp, top = 20.dp, bottom = 4.dp),
    )
}

@Composable
private fun SettingSwitch(
    title: String,
    checked: Boolean,
    onChange: (Boolean) -> Unit,
    subtitle: String? = null,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.bodyLarge)
            if (subtitle != null) {
                Text(
                    subtitle,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
        Spacer(Modifier.size(12.dp))
        Switch(checked = checked, onCheckedChange = onChange)
    }
}
