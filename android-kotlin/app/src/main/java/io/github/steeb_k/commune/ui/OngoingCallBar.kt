// A call that has been put aside still has to be reachable, and still has
// to be endable, from wherever the reader went. This is that bar.
package io.github.steeb_k.commune.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Call
import androidx.compose.material.icons.filled.CallEnd
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import io.github.steeb_k.commune.CommuneState

@Composable
fun OngoingCallBar(state: CommuneState) {
    val call = state.call ?: return
    val name = call.peer.removePrefix("@").substringBefore(':')

    Row(
        modifier = Modifier
            .fillMaxWidth()
            .background(MaterialTheme.colorScheme.primaryContainer)
            // The whole bar is the way back, not just the icon.
            .clickable { state.restoreCall() }
            .padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(
                Icons.Filled.Call,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onPrimaryContainer,
                modifier = Modifier.size(20.dp),
            )
            Spacer(Modifier.width(12.dp))
            Text(
                when (call.state) {
                    CommuneState.CallPhase.Ringing -> "Incoming call from $name"
                    CommuneState.CallPhase.Dialing -> "Calling $name"
                    CommuneState.CallPhase.Connecting -> "Connecting to $name"
                    CommuneState.CallPhase.Connected -> "Ongoing call with $name"
                    CommuneState.CallPhase.Ended -> "Call ended"
                },
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onPrimaryContainer,
                maxLines = 1,
            )
        }
        IconButton(onClick = { state.hangUp() }) {
            Icon(
                Icons.Filled.CallEnd,
                contentDescription = "Hang up",
                tint = MaterialTheme.colorScheme.error,
            )
        }
    }
}
