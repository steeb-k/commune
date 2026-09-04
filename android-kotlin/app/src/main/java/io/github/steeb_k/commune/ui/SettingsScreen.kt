// Account settings, the first slice: the three toggles the core already
// persists — notifications, public read receipts, typing — grouped as the
// GTK Account Settings groups them.
package io.github.steeb_k.commune.ui

import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Close
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

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(androidx.compose.foundation.rememberScrollState()),
    ) {
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

        SettingsGroup("Profile")
        ProfileRows(state)

        SettingsGroup("Notifications")
        SettingSwitch(
            title = "Enable Notifications for This Account",
            checked = settings?.notificationsEnabled == true,
            onChange = { state.setNotificationsEnabled(it) },
        )
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text("Delivery", style = MaterialTheme.typography.bodyLarge)
                Text(
                    when (state.pushMode) {
                        io.github.steeb_k.commune.PushManager.MODE_UNIFIEDPUSH ->
                            "Instant push (UnifiedPush)"
                        io.github.steeb_k.commune.PushManager.MODE_SYNC ->
                            "Background sync"
                        else -> "Not chosen yet"
                    },
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            TextButton(onClick = {
                state.reopenPushOnboarding()
                state.closeSettings()
            }) { Text("Change") }
        }
        KeywordRows(state)

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
        SettingSwitch(
            title = "Share Presence",
            subtitle = "Let others see when you are online",
            checked = settings?.sharePresence == true,
            onChange = { state.setSharePresence(it) },
        )
        SettingSwitch(
            title = "Show Link Previews",
            subtitle = "In rooms that are not encrypted; the homeserver reads the page",
            checked = settings?.urlPreviewsEnabled == true,
            onChange = { state.setUrlPreviewsEnabled(it) },
        )
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text("Ignored Users", style = MaterialTheme.typography.bodyLarge)
                Text(
                    "Users whose messages are hidden everywhere",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            TextButton(onClick = { state.openIgnoredUsers() }) { Text("Open") }
        }

        SettingsGroup("Sessions")
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text("Manage Sessions", style = MaterialTheme.typography.bodyLarge)
                Text(
                    "Rename or sign out your other sessions",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            TextButton(onClick = { state.openDevices() }) { Text("Open") }
        }

        SettingsGroup("Stickers")
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text("Image Packs", style = MaterialTheme.typography.bodyLarge)
                Text(
                    "Your own sticker and emoticon packs",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            TextButton(onClick = { state.openImagePacks() }) { Text("Open") }
        }

        SettingsGroup("Encryption")
        KeyBackupRows(state)
        RecoveryRow(state)
        ResetIdentityRow(state)
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text("Verify This Session", style = MaterialTheme.typography.bodyLarge)
                Text(
                    "Compare emoji with one of your other sessions",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            TextButton(onClick = {
                state.requestVerification()
                state.closeSettings()
            }) { Text("Verify") }
        }

        FullScreenCallRow()

        SettingsGroup("Account")
        SettingsGroup("Account")
        ChangePasswordRow(state)
        ThirdPartyIdsRow(state)
        DeactivateAccountRow(state)
        LogoutRow(state)
    }
}

/// Ringing on a locked screen needs a permission the system does not hand
/// out on its own.
///
/// From Android 14 an application only gets `USE_FULL_SCREEN_INTENT`
/// granted at install if the system already believes it is a calling or
/// alarm application. Declaring the permission is not enough: without the
/// grant the incoming call is refused the screen and becomes one more line
/// in the shade, which is no use to a phone lying face down on a table.
/// Nothing can grant it from here, so this row says so and opens the page
/// that can. It stays out of the way once there is nothing to ask for.
///
/// (Android's own affair — the GTK application has no counterpart.)
@Composable
private fun FullScreenCallRow() {
    if (android.os.Build.VERSION.SDK_INT < android.os.Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
        return
    }
    val context = androidx.compose.ui.platform.LocalContext.current
    val manager = context.getSystemService(android.app.NotificationManager::class.java)
    // Re-read whenever this screen comes back, so granting it makes the
    // row disappear.
    var permitted by androidx.compose.runtime.remember {
        androidx.compose.runtime.mutableStateOf(manager.canUseFullScreenIntent())
    }
    androidx.compose.runtime.LaunchedEffect(Unit) {
        permitted = manager.canUseFullScreenIntent()
    }
    if (permitted) return

    SettingsGroup("Calls")
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text("Full-Screen Call Alerts", style = MaterialTheme.typography.bodyLarge)
            Text(
                "Android is not letting calls take over a locked screen. " +
                    "Without this a call arrives as a quiet notification.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        TextButton(onClick = {
            context.startActivity(
                android.content.Intent(
                    android.provider.Settings
                        .ACTION_MANAGE_APP_USE_FULL_SCREEN_INTENT,
                    android.net.Uri.parse("package:" + context.packageName),
                )
            )
        }) { Text("Allow") }
    }
}

/// The keywords that trigger notifications: the GTK notifications
/// page's list, with an add row at the end.
@Composable
private fun KeywordRows(state: CommuneState) {
    androidx.compose.runtime.LaunchedEffect(Unit) { state.loadNotificationKeywords() }
    var newKeyword by remember { mutableStateOf("") }

    Text(
        "Keywords that trigger notifications",
        style = MaterialTheme.typography.bodyMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
    )
    for (keyword in state.notificationKeywords) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                keyword,
                style = MaterialTheme.typography.bodyLarge,
                modifier = Modifier.weight(1f),
            )
            IconButton(onClick = { state.removeNotificationKeyword(keyword) }) {
                Icon(
                    Icons.Filled.Close,
                    contentDescription = "Remove “$keyword”",
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        OutlinedTextField(
            value = newKeyword,
            onValueChange = { newKeyword = it },
            placeholder = { Text("Add a keyword") },
            singleLine = true,
            modifier = Modifier.weight(1f),
        )
        TextButton(
            enabled = newKeyword.isNotBlank(),
            onClick = {
                state.addNotificationKeyword(newKeyword.trim())
                newKeyword = ""
            },
        ) { Text("Add") }
    }
}

/// Display name (tap to edit) and avatar.
@Composable
private fun ProfileRows(state: CommuneState) {
    var editOpen by remember { mutableStateOf(false) }
    androidx.compose.runtime.LaunchedEffect(Unit) { state.loadProfile() }

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text("Display Name", style = MaterialTheme.typography.bodyLarge)
            Text(
                state.profileName ?: state.ownUserId.orEmpty(),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        TextButton(onClick = { editOpen = true }) { Text("Edit") }
    }
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text("Avatar", style = MaterialTheme.typography.bodyLarge)
            Text(
                "Pick a new profile picture",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        TextButton(onClick = { state.pickAvatar?.invoke() }) { Text("Change") }
    }

    if (editOpen) {
        var name by remember { mutableStateOf(state.profileName.orEmpty()) }
        var error by remember { mutableStateOf<String?>(null) }
        androidx.compose.material3.AlertDialog(
            onDismissRequest = { editOpen = false },
            title = { Text("Display Name") },
            text = {
                Column {
                    OutlinedTextField(
                        value = name,
                        onValueChange = { name = it },
                        singleLine = true,
                    )
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
                TextButton(
                    enabled = name.isNotBlank(),
                    onClick = {
                        state.setDisplayName(name) { failure ->
                            if (failure == null) editOpen = false else error = failure
                        }
                    },
                ) { Text("Save") }
            },
            dismissButton = {
                TextButton(onClick = { editOpen = false }) { Text("Cancel") }
            },
        )
    }
}

/// Reset the crypto identity: new cross-signing keys, which every other
/// session will have to verify again. The GTK recovery setup offers it
/// when the old identity is lost.
@Composable
private fun ResetIdentityRow(state: CommuneState) {
    var open by remember { mutableStateOf(false) }
    var password by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text("Reset Crypto Identity", style = MaterialTheme.typography.bodyLarge)
            Text(
                "New cross-signing keys; other sessions must verify this one again",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        TextButton(onClick = {
            // A fresh dialog each time: nothing typed before survives it.
            password = ""
            error = null
            open = true
        }) { Text("Reset") }
    }

    if (open) {
        androidx.compose.material3.AlertDialog(
            onDismissRequest = { open = false },
            title = { Text("Reset Crypto Identity?") },
            text = {
                Column {
                    Text(
                        "Your other sessions will no longer trust this one until you " +
                            "verify it again. Enter your password to confirm.",
                    )
                    androidx.compose.material3.OutlinedTextField(
                        value = password,
                        onValueChange = { password = it },
                        label = { Text("Password") },
                        singleLine = true,
                        visualTransformation =
                            androidx.compose.ui.text.input.PasswordVisualTransformation(),
                    )
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
                TextButton(
                    enabled = !busy && password.isNotEmpty(),
                    onClick = {
                        busy = true
                        error = null
                        state.resetCrossSigning(password) { failure ->
                            busy = false
                            if (failure == null) {
                                state.toast("Crypto identity reset")
                                open = false
                            } else {
                                error = failure
                            }
                        }
                    },
                ) { Text(if (busy) "…" else "Reset", color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = {
                TextButton(onClick = { open = false }) { Text("Cancel") }
            },
        )
    }
}

/// Log out, behind a confirmation.
@Composable
private fun LogoutRow(state: CommuneState) {
    var confirmOpen by remember { mutableStateOf(false) }

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text(
                "Log Out",
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.error,
            )
            Text(
                "End this session on this device",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        TextButton(onClick = { confirmOpen = true }) {
            Text("Log Out", color = MaterialTheme.colorScheme.error)
        }
    }

    if (confirmOpen) {
        androidx.compose.material3.AlertDialog(
            onDismissRequest = { confirmOpen = false },
            title = { Text("Log Out?") },
            text = {
                Text(
                    "Make sure your recovery key is saved: without another " +
                        "verified session, it is the only way back into your " +
                        "encrypted messages."
                )
            },
            confirmButton = {
                TextButton(onClick = {
                    confirmOpen = false
                    state.logout()
                }) { Text("Log Out", color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = {
                TextButton(onClick = { confirmOpen = false }) { Text("Cancel") }
            },
        )
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


/// Export the room keys to Downloads, or import an export — each behind
/// a passphrase dialog.
@Composable
private fun KeyBackupRows(state: CommuneState) {
    var exportOpen by remember { mutableStateOf(false) }
    var importOpen by remember { mutableStateOf(false) }
    var exportResult by remember { mutableStateOf<String?>(null) }

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text("Message Keys", style = MaterialTheme.typography.bodyLarge)
            Text(
                exportResult ?: state.importResult
                    ?: "Export or import the keys to your encrypted messages",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        TextButton(onClick = { exportOpen = true }) { Text("Export") }
        TextButton(onClick = { importOpen = true }) { Text("Import") }
    }

    if (exportOpen) {
        PassphraseDialog(
            title = "Export Keys",
            explainer = "The export is encrypted with this passphrase; " +
                "it lands in Downloads as commune-keys.txt.",
            confirm = "Export",
            onConfirm = { passphrase, done ->
                state.exportKeys(passphrase) { failure ->
                    done(failure)
                    if (failure == null) {
                        exportResult = "Keys exported to Downloads"
                        exportOpen = false
                    }
                }
            },
            onDismiss = { exportOpen = false },
        )
    }
    if (importOpen) {
        PassphraseDialog(
            title = "Import Keys",
            explainer = "Enter the passphrase of the export, then pick the file.",
            confirm = "Pick File",
            onConfirm = { passphrase, done ->
                state.clearImportResult()
                state.pendingImportPassphrase = passphrase
                state.pickKeyFile?.invoke()
                done(null)
                importOpen = false
            },
            onDismiss = { importOpen = false },
        )
    }
}

@Composable
private fun PassphraseDialog(
    title: String,
    explainer: String,
    confirm: String,
    onConfirm: (String, (String?) -> Unit) -> Unit,
    onDismiss: () -> Unit,
) {
    var passphrase by remember { mutableStateOf("") }
    var error by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }

    androidx.compose.material3.AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = {
            Column {
                Text(
                    explainer,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                OutlinedTextField(
                    value = passphrase,
                    onValueChange = { passphrase = it },
                    label = { Text("Passphrase") },
                    singleLine = true,
                    visualTransformation =
                        androidx.compose.ui.text.input.PasswordVisualTransformation(),
                )
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
            TextButton(
                enabled = passphrase.isNotBlank() && !busy,
                onClick = {
                    busy = true
                    onConfirm(passphrase) { failure ->
                        busy = false
                        if (failure != null) error = failure
                    }
                },
            ) { Text(confirm) }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("Cancel") }
        },
    )
}