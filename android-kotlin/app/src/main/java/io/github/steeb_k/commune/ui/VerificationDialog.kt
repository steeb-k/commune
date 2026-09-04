// The device-verification dialog: the request, the emoji row to compare,
// and the confirmation — the GTK identity-verification flow's SAS core,
// one dialog at a time.
package io.github.steeb_k.commune.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import io.github.steeb_k.commune.CommuneState

@Composable
fun VerificationDialog(state: CommuneState) {
    val flowId = state.verificationFlowId ?: return

    when {
        state.verificationDone -> AlertDialog(
            onDismissRequest = { state.dismissVerification() },
            title = { Text("Verified") },
            text = { Text("This session is now verified.") },
            confirmButton = {
                TextButton(onClick = { state.dismissVerification() }) { Text("Done") }
            },
        )

        state.verificationEmojis.isNotEmpty() -> AlertDialog(
            onDismissRequest = {},
            title = { Text("Compare Emoji") },
            text = {
                Column {
                    Text(
                        "Confirm that both sessions show the same emoji, " +
                            "in the same order.",
                        style = MaterialTheme.typography.bodyMedium,
                    )
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.SpaceEvenly,
                    ) {
                        for (emoji in state.verificationEmojis.take(4)) {
                            EmojiCell(emoji.symbol, emoji.description)
                        }
                    }
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.SpaceEvenly,
                    ) {
                        for (emoji in state.verificationEmojis.drop(4)) {
                            EmojiCell(emoji.symbol, emoji.description)
                        }
                    }
                }
            },
            confirmButton = {
                TextButton(onClick = { state.confirmVerification() }) { Text("They Match") }
            },
            dismissButton = {
                TextButton(onClick = { state.cancelVerification() }) { Text("No Match") }
            },
        )

        state.verificationAccepted -> {
            // The flow says when a QR code is worth showing; ask until it
            // does, or until the emoji arrive and this branch is left.
            var qr by androidx.compose.runtime.remember(flowId) {
                androidx.compose.runtime.mutableStateOf<ByteArray?>(null)
            }
            androidx.compose.runtime.LaunchedEffect(flowId) {
                while (qr == null) {
                    qr = state.app.verificationQrCode(flowId)
                    kotlinx.coroutines.delay(500)
                }
            }
            AlertDialog(
                onDismissRequest = {},
                title = { Text("Waiting for the Other Session") },
                text = {
                    Column(horizontalAlignment = Alignment.CenterHorizontally) {
                        Text(
                            "Scan this QR code from the other session, or wait for " +
                                "it to start the emoji comparison.",
                            style = MaterialTheme.typography.bodyMedium,
                        )
                        qr?.let { QrCodeImage(it, modifier = Modifier.fillMaxWidth()) }
                    }
                },
                confirmButton = {
                    TextButton(onClick = { state.scanQrCode?.invoke() }) { Text("Scan Theirs") }
                },
                dismissButton = {
                    TextButton(onClick = { state.cancelVerification() }) { Text("Cancel") }
                },
            )
        }

        state.verificationOutgoing -> AlertDialog(
            onDismissRequest = {},
            title = { Text("Verify This Session") },
            text = {
                Text(
                    "Accept the request on one of your other sessions to " +
                        "compare emoji, or scan the QR code it shows.",
                )
            },
            confirmButton = {
                TextButton(onClick = { state.scanQrCode?.invoke() }) { Text("Scan QR") }
            },
            dismissButton = {
                TextButton(onClick = { state.cancelVerification() }) { Text("Cancel") }
            },
        )

        else -> AlertDialog(
            onDismissRequest = {},
            title = { Text("Verification Request") },
            text = {
                Text(
                    "${state.verificationUser ?: "Someone"} wants to verify " +
                        "this session.",
                )
            },
            confirmButton = {
                TextButton(onClick = { state.acceptVerification() }) { Text("Accept") }
            },
            dismissButton = {
                TextButton(onClick = { state.cancelVerification() }) { Text("Decline") }
            },
        )
    }
}

@Composable
private fun EmojiCell(symbol: String, description: String) {
    Column(horizontalAlignment = Alignment.CenterHorizontally) {
        Text(symbol, style = MaterialTheme.typography.headlineMedium)
        Text(
            description,
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}
