package io.github.steeb_k.commune.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import io.github.steeb_k.commune.CommuneState
import io.github.steeb_k.commune.core.FfiUserSearchResult
import kotlinx.coroutines.delay

/// The user directory's answers to what is being typed, as the GTK app's
/// invite page and direct chat dialog list them under the field. A tap
/// puts the user's ID in the field; `exclude` hides the ones a room
/// already has.
@Composable
fun UserSuggestions(
    state: CommuneState,
    query: String,
    exclude: Set<String>,
    onPick: (String) -> Unit,
) {
    var results by remember { mutableStateOf<List<FfiUserSearchResult>>(emptyList()) }
    var latest by remember { mutableStateOf("") }

    LaunchedEffect(query) {
        val term = query.trim()
        latest = term
        if (term.length < 2) {
            results = emptyList()
            return@LaunchedEffect
        }
        // A pause, so a word in progress is not a request per keystroke.
        delay(300)
        state.searchUsers(term) { found ->
            // A late answer to an earlier term is not an answer to this one.
            if (latest == term) results = found
        }
    }

    val shown = results
        .filter { it.userId !in exclude && it.userId != query.trim() }
        .take(5)
    if (shown.isEmpty()) return

    Column(modifier = Modifier.padding(top = 4.dp)) {
        for (user in shown) {
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .clickable { onPick(user.userId) }
                    .padding(vertical = 6.dp),
            ) {
                Text(
                    user.displayName ?: localpart(user.userId),
                    style = MaterialTheme.typography.bodyMedium,
                )
                Text(
                    user.userId,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}
