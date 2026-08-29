// What the GTK login's session_setup_view does after a session comes
// up: read the three encryption states, and if this session is not
// verified or recovery is not set up, say so and offer the way out —
// verify against another session, enter the recovery key, or create the
// identity. Nobody should reach their rooms without being told their
// old messages are unreadable until they do one of these.
package io.github.steeb_k.commune.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import io.github.steeb_k.commune.CommuneState
import io.github.steeb_k.commune.core.FfiCryptoIdentityState
import io.github.steeb_k.commune.core.FfiRecoveryState
import io.github.steeb_k.commune.core.FfiVerificationState

@Composable
fun SessionSetupScreen(state: CommuneState) {
    val security = state.securityState ?: return
    var recoveryKey by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var showKeyEntry by remember { mutableStateOf(false) }
    var showPassword by remember { mutableStateOf(false) }

    val unverified = security.verification != FfiVerificationState.VERIFIED

    LoginColumn {
        androidx.compose.material3.Icon(
            androidx.compose.ui.res.painterResource(
                io.github.steeb_k.commune.R.drawable.ic_app_symbolic
            ),
            contentDescription = null,
            tint = MaterialTheme.colorScheme.primary,
            modifier = Modifier.size(72.dp),
        )
        Spacer(Modifier.height(20.dp))

        Text(
            if (unverified) "Verify This Session" else "Set Up Recovery",
            style = MaterialTheme.typography.headlineMedium,
            textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(12.dp))
        Text(
            when {
                unverified && security.identity == FfiCryptoIdentityState.MISSING ->
                    "This account has no encryption identity yet. Setting one up lets " +
                        "your sessions trust each other and keeps your messages readable."
                unverified && security.identity == FfiCryptoIdentityState.LAST_MAN_STANDING ->
                    "This is your only session, so there is nothing to compare against. " +
                        "Enter your recovery key to unlock your encrypted messages."
                unverified ->
                    "Until this session is verified, your encrypted messages stay " +
                        "unreadable here. Verify against another session, or enter your " +
                        "recovery key."
                else ->
                    "Recovery keeps your encrypted messages reachable if you lose this " +
                        "device. Without it, a lost session means lost history."
            },
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(28.dp))

        if (showKeyEntry) {
            OutlinedTextField(
                value = recoveryKey,
                onValueChange = { recoveryKey = it },
                label = { Text("Recovery Key") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
            Spacer(Modifier.height(12.dp))
            Button(
                onClick = { state.submitRecoveryKey(recoveryKey) },
                enabled = recoveryKey.isNotBlank() && !state.setupBusy && !state.recoveryBusy,
                modifier = Modifier.widthIn(min = 260.dp),
            ) { Text("Unlock") }
            TextButton(onClick = { showKeyEntry = false }) { Text("Back") }
        } else if (showPassword) {
            OutlinedTextField(
                value = password,
                onValueChange = { password = it },
                label = { Text("Account Password") },
                singleLine = true,
                visualTransformation = PasswordVisualTransformation(),
                modifier = Modifier.fillMaxWidth(),
            )
            Spacer(Modifier.height(12.dp))
            Button(
                onClick = { state.bootstrapCrossSigning(password) },
                enabled = password.isNotEmpty() && !state.setupBusy && !state.recoveryBusy,
                modifier = Modifier.widthIn(min = 260.dp),
            ) { Text("Set Up Encryption") }
            TextButton(onClick = { showPassword = false }) { Text("Back") }
        } else {
            when {
                unverified && security.identity == FfiCryptoIdentityState.MISSING -> {
                    Button(
                        onClick = { showPassword = true },
                        modifier = Modifier.widthIn(min = 260.dp),
                    ) { Text("Set Up Encryption") }
                }
                unverified && security.identity == FfiCryptoIdentityState.OTHER_SESSIONS -> {
                    Button(
                        onClick = { state.requestVerification() },
                        modifier = Modifier.widthIn(min = 260.dp),
                    ) { Text("Verify with Another Session") }
                    Spacer(Modifier.height(12.dp))
                    if (security.recovery != FfiRecoveryState.DISABLED) {
                        OutlinedButton(
                            onClick = { showKeyEntry = true },
                            modifier = Modifier.widthIn(min = 260.dp),
                        ) { Text("Use Recovery Key") }
                    }
                }
                unverified -> {
                    Button(
                        onClick = { showKeyEntry = true },
                        modifier = Modifier.widthIn(min = 260.dp),
                    ) { Text("Enter Recovery Key") }
                }
                else -> {
                    Button(
                        onClick = { state.enableRecovery() },
                        enabled = !state.setupBusy && !state.recoveryBusy,
                        modifier = Modifier.widthIn(min = 260.dp),
                    ) { Text("Set Up Recovery") }
                }
            }
        }

        (state.setupError ?: state.recoveryError)?.let { error ->
            Spacer(Modifier.height(12.dp))
            Text(error, color = MaterialTheme.colorScheme.error, textAlign = TextAlign.Center)
        }
        if (state.setupBusy || state.recoveryBusy) {
            Spacer(Modifier.height(16.dp))
            LoadingRing(modifier = Modifier.size(56.dp))
        }

        Spacer(Modifier.height(24.dp))
        TextButton(onClick = { state.skipSessionSetup() }) {
            Text("Not Now")
        }
    }
}

/// The recovery key a fresh setup just produced: shown once, because
/// the server never gives it again.
@Composable
fun RecoveryKeyScreen(state: CommuneState, key: String) {
    LoginColumn {
        Text(
            "Save Your Recovery Key",
            style = MaterialTheme.typography.headlineMedium,
            textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(12.dp))
        Text(
            "This key is the only way back into your encrypted messages if you " +
                "lose this device. It is shown once — write it down or put it in " +
                "your password manager now.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(24.dp))
        androidx.compose.material3.Card(modifier = Modifier.fillMaxWidth()) {
            Text(
                key,
                style = MaterialTheme.typography.bodyLarge,
                fontFamily = androidx.compose.ui.text.font.FontFamily.Monospace,
                textAlign = TextAlign.Center,
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(16.dp),
            )
        }
        Spacer(Modifier.height(16.dp))
        val clipboard = androidx.compose.ui.platform.LocalClipboardManager.current
        OutlinedButton(
            onClick = {
                clipboard.setText(androidx.compose.ui.text.AnnotatedString(key))
            },
            modifier = Modifier.widthIn(min = 260.dp),
        ) { Text("Copy Key") }
        Spacer(Modifier.height(12.dp))
        Button(
            onClick = { state.dismissRecoveryKey() },
            modifier = Modifier.widthIn(min = 260.dp),
        ) { Text("I Saved It") }
    }
}
