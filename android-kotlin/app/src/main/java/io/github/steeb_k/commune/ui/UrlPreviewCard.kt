package io.github.steeb_k.commune.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.steeb_k.commune.CommuneState
import io.github.steeb_k.commune.core.FfiUrlPreview

/// The card under a message for the first link it carries, as the GTK
/// history draws one: the site, the title, the description, the image.
/// The core decided whether this message gets one — never in an encrypted
/// room — and the homeserver describes the page; nothing is fetched from
/// the site itself.
@Composable
fun UrlPreviewCard(state: CommuneState, url: String) {
    var preview by remember(url) { mutableStateOf<FfiUrlPreview?>(null) }
    var imagePath by remember(url) { mutableStateOf<String?>(null) }
    LaunchedEffect(url) {
        preview = state.app.urlPreview(url)
        preview?.imageUrl?.let { imagePath = state.fetchMxcPath(it) }
    }

    val card = preview ?: return
    val context = LocalContext.current

    Row(
        modifier = Modifier
            .padding(top = 6.dp)
            .fillMaxWidth()
            .clip(RoundedCornerShape(8.dp))
            .background(MaterialTheme.colorScheme.surface.copy(alpha = 0.55f))
            .clickable {
                try {
                    context.startActivity(
                        android.content.Intent(
                            android.content.Intent.ACTION_VIEW,
                            android.net.Uri.parse(card.url),
                        ),
                    )
                } catch (_: Exception) {
                }
            }
            .padding(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        imagePath?.let { path ->
            MediaImage(
                path,
                contentDescription = null,
                modifier = Modifier
                    .size(56.dp)
                    .clip(RoundedCornerShape(6.dp)),
                targetSizePx = 168,
                fill = true,
            )
            Spacer(Modifier.width(8.dp))
        }
        Column {
            Text(
                card.siteName,
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            card.title?.let {
                Text(
                    it,
                    style = MaterialTheme.typography.bodyMedium,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            card.description?.let {
                Text(
                    it,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 3,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
    }
}
