package io.github.steeb_k.commune.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.AlertDialog
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
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import io.github.steeb_k.commune.CommuneState
import io.github.steeb_k.commune.core.FfiPendingEmail
import io.github.steeb_k.commune.core.FfiThirdPartyIds

// The account's own settings — its password, its addresses, and its
// deactivation — as the GTK app's account settings subpages offer them.
// Each asks for the current password where the homeserver asks for it.

/// One settings row with a title, a caption, and an action.
@Composable
private fun ActionRow(
    title: String,
    caption: String,
    action: String,
    danger: Boolean = false,
    onClick: () -> Unit,
) {
    val color = if (danger) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.bodyLarge, color = color)
            Text(
                caption,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        TextButton(onClick = onClick) { Text(action, color = color) }
    }
}

@Composable
private fun ErrorText(error: String?) {
    error?.let {
        Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
    }
}

/// Change the password: the new one twice, and the current one for the
/// homeserver's password stage.
@Composable
fun ChangePasswordRow(state: CommuneState) {
    var open by remember { mutableStateOf(false) }
    ActionRow("Change Password", "Choose a new password for the account", "Change") { open = true }
    if (!open) return

    var password by remember { mutableStateOf("") }
    var confirmation by remember { mutableStateOf("") }
    var current by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val close = { open = false }

    AlertDialog(
        onDismissRequest = close,
        title = { Text("Change Password") },
        text = {
            Column {
                OutlinedTextField(
                    value = password,
                    onValueChange = { password = it },
                    label = { Text("New password") },
                    singleLine = true,
                    visualTransformation = PasswordVisualTransformation(),
                )
                OutlinedTextField(
                    value = confirmation,
                    onValueChange = { confirmation = it },
                    label = { Text("Confirm new password") },
                    singleLine = true,
                    visualTransformation = PasswordVisualTransformation(),
                    isError = confirmation.isNotEmpty() && confirmation != password,
                )
                Spacer(Modifier.height(8.dp))
                OutlinedTextField(
                    value = current,
                    onValueChange = { current = it },
                    label = { Text("Current password") },
                    singleLine = true,
                    visualTransformation = PasswordVisualTransformation(),
                )
                ErrorText(error)
            }
        },
        confirmButton = {
            TextButton(
                enabled = !busy && password.length >= 8 && password == confirmation && current.isNotEmpty(),
                onClick = {
                    busy = true
                    error = null
                    state.changePassword(password, current) { failure ->
                        busy = false
                        if (failure == null) {
                            state.toast("Password changed successfully")
                            close()
                        } else {
                            error = failure
                        }
                    }
                },
            ) { Text(if (busy) "…" else "Change") }
        },
        dismissButton = { TextButton(onClick = close) { Text("Cancel") } },
    )
}

/// The email addresses and phone numbers on the account: listed, removed,
/// and an email added through its validation link.
@Composable
fun ThirdPartyIdsRow(state: CommuneState) {
    var open by remember { mutableStateOf(false) }
    ActionRow(
        "Email Addresses and Phone Numbers",
        "The addresses linked to the account",
        "Open",
    ) { open = true }
    if (!open) return

    var ids by remember { mutableStateOf<FfiThirdPartyIds?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    var newEmail by remember { mutableStateOf("") }
    var pending by remember { mutableStateOf<FfiPendingEmail?>(null) }
    var current by remember { mutableStateOf("") }
    var reloads by remember { mutableStateOf(0) }

    LaunchedEffect(reloads) {
        state.thirdPartyIds { found, failure ->
            ids = found
            error = failure
        }
    }

    AlertDialog(
        onDismissRequest = { open = false },
        title = { Text("Email Addresses and Phone Numbers") },
        text = {
            Column {
                val loaded = ids
                if (loaded == null && error == null) {
                    Text("Loading…", color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                loaded?.let { list ->
                    if (list.ids.isEmpty()) {
                        Text("No addresses", color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    for (id in list.ids) {
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Column(modifier = Modifier.weight(1f)) {
                                Text(id.address, style = MaterialTheme.typography.bodyMedium)
                                Text(
                                    if (id.isEmail) "Email" else "Phone",
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                            if (list.canChange) {
                                TextButton(
                                    enabled = !busy,
                                    onClick = {
                                        busy = true
                                        state.deleteThirdPartyId(id.address, id.isEmail) { failure ->
                                            busy = false
                                            if (failure == null) {
                                                state.toast("Address removed")
                                                reloads += 1
                                            } else {
                                                error = failure
                                            }
                                        }
                                    },
                                ) { Text("Remove") }
                            }
                        }
                    }
                    if (list.canChange) {
                        Spacer(Modifier.height(8.dp))
                        val waiting = pending
                        if (waiting == null) {
                            OutlinedTextField(
                                value = newEmail,
                                onValueChange = { newEmail = it },
                                label = { Text("Add an email address") },
                                singleLine = true,
                            )
                        } else {
                            // The GTK dialog's words: the link first, then
                            // the password the homeserver asks for.
                            Text(
                                "A validation link was sent to ${waiting.address}. " +
                                    "Open it, then continue here.",
                                style = MaterialTheme.typography.bodyMedium,
                            )
                            OutlinedTextField(
                                value = current,
                                onValueChange = { current = it },
                                label = { Text("Current password") },
                                singleLine = true,
                                visualTransformation = PasswordVisualTransformation(),
                            )
                            TextButton(
                                enabled = !busy,
                                onClick = {
                                    busy = true
                                    state.requestEmailValidation(waiting.address, waiting) { again, failure ->
                                        busy = false
                                        if (again != null) pending = again else error = failure
                                    }
                                },
                            ) { Text("Resend the email") }
                        }
                    }
                }
                ErrorText(error)
            }
        },
        confirmButton = {
            val waiting = pending
            when {
                ids?.canChange != true -> {}
                waiting == null -> TextButton(
                    enabled = !busy && newEmail.contains('@'),
                    onClick = {
                        busy = true
                        error = null
                        state.requestEmailValidation(newEmail.trim(), null) { sent, failure ->
                            busy = false
                            if (sent != null) pending = sent else error = failure
                        }
                    },
                ) { Text(if (busy) "…" else "Add") }
                else -> TextButton(
                    enabled = !busy && current.isNotEmpty(),
                    onClick = {
                        busy = true
                        error = null
                        state.addPendingEmail(waiting, current) { failure ->
                            busy = false
                            if (failure == null) {
                                state.toast("Address added")
                                pending = null
                                newEmail = ""
                                current = ""
                                reloads += 1
                            } else {
                                error = failure
                            }
                        }
                    },
                ) { Text(if (busy) "…" else "Continue") }
            }
        },
        dismissButton = { TextButton(onClick = { open = false }) { Text("Close") } },
    )
}

/// Deactivate the account, behind typing the user ID and the password: it
/// cannot be undone, and the GTK subpage asks the same way.
@Composable
fun DeactivateAccountRow(state: CommuneState) {
    var open by remember { mutableStateOf(false) }
    ActionRow(
        "Deactivate Account",
        "Permanently disable the account; messages stay readable to others",
        "Deactivate",
        danger = true,
    ) { open = true }
    if (!open) return

    val userId = state.ownUserId.orEmpty()
    var confirmation by remember { mutableStateOf("") }
    var current by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }

    AlertDialog(
        onDismissRequest = { open = false },
        title = { Text("Deactivate Account?") },
        text = {
            Column {
                Text(
                    "Deactivating the account cannot be undone. Type $userId to confirm.",
                    style = MaterialTheme.typography.bodyMedium,
                )
                OutlinedTextField(
                    value = confirmation,
                    onValueChange = { confirmation = it },
                    label = { Text(userId) },
                    singleLine = true,
                )
                OutlinedTextField(
                    value = current,
                    onValueChange = { current = it },
                    label = { Text("Current password") },
                    singleLine = true,
                    visualTransformation = PasswordVisualTransformation(),
                )
                ErrorText(error)
            }
        },
        confirmButton = {
            TextButton(
                enabled = !busy && confirmation == userId && userId.isNotEmpty() && current.isNotEmpty(),
                onClick = {
                    busy = true
                    error = null
                    state.deactivateAccount(current) { failure ->
                        busy = false
                        if (failure != null) error = failure
                    }
                },
            ) { Text(if (busy) "…" else "Deactivate", color = MaterialTheme.colorScheme.error) }
        },
        dismissButton = { TextButton(onClick = { open = false }) { Text("Cancel") } },
    )
}
