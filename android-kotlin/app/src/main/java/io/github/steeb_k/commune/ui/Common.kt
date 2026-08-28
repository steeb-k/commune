// Small shared pieces: initials avatars (the GTK app's Adw.Avatar fallback),
// and the loading screen.
package io.github.steeb_k.commune.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.foundation.Image
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import io.github.steeb_k.commune.CommuneState
import io.github.steeb_k.commune.core.FfiRoom
import io.github.steeb_k.commune.core.FfiRoomDisplayName

/// The name of a room, as text. The Empty variants are our sentences to
/// make — the core hands over semantics.
fun roomName(room: FfiRoom): String = when (val displayName = room.displayName) {
    is FfiRoomDisplayName.Named -> displayName.name
    is FfiRoomDisplayName.EmptyWas -> "Empty Room (was ${displayName.user})"
    is FfiRoomDisplayName.Empty -> "Empty Room"
    is FfiRoomDisplayName.Unknown -> "Unknown"
}

/// The avatar palette, in the spirit of Adw.Avatar's color set.
private val AVATAR_COLORS = listOf(
    Color(0xFF83B6EC) to Color(0xFF152E4C),
    Color(0xFF7AD9F1) to Color(0xFF0E3A46),
    Color(0xFF8DE6B1) to Color(0xFF11402A),
    Color(0xFFF8E359) to Color(0xFF4C4212),
    Color(0xFFFFCB62) to Color(0xFF503A12),
    Color(0xFFFFA95A) to Color(0xFF553311),
    Color(0xFFF78773) to Color(0xFF521A12),
    Color(0xFFE973AB) to Color(0xFF4D1832),
    Color(0xFFCB78D4) to Color(0xFF421C47),
    Color(0xFF9E91E8) to Color(0xFF272052),
)

/// An initials avatar, colored stably by its identifier.
@Composable
fun InitialsAvatar(identifier: String, name: String, size: Dp) {
    val (background, foreground) =
        AVATAR_COLORS[(identifier.hashCode().mod(AVATAR_COLORS.size))]
    val initial = name.firstOrNull { it.isLetterOrDigit() }?.uppercase() ?: "?"

    Box(
        modifier = Modifier
            .size(size)
            .clip(CircleShape)
            .background(background),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = initial,
            color = foreground,
            fontWeight = FontWeight.Bold,
            fontSize = (size.value * 0.45f).sp,
        )
    }
}

/// The loading page: what the GTK window's `loading` stack page shows.
@Composable
fun LoadingScreen() {
    Column(
        modifier = Modifier.fillMaxSize(),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        CircularProgressIndicator()
        Text(
            "Fetching Account Data…",
            style = MaterialTheme.typography.titleLarge,
            modifier = Modifier.padding(top = 24.dp),
        )
    }
}


/// A room avatar: the picture when there is one, initials otherwise.
@Composable
fun RoomAvatar(state: CommuneState, room: FfiRoom, size: Dp) {
    val avatarUrl = room.avatarUrl

    if (avatarUrl == null) {
        InitialsAvatar(identifier = room.roomId, name = roomName(room), size = size)
        return
    }

    var bitmap by remember(avatarUrl) { mutableStateOf<android.graphics.Bitmap?>(null) }
    LaunchedEffect(avatarUrl) {
        val path = state.app.getRoomAvatar(room.roomId, 96u)
        if (path != null) {
            bitmap = android.graphics.BitmapFactory.decodeFile(path)
        }
    }

    val loaded = bitmap
    if (loaded == null) {
        InitialsAvatar(identifier = room.roomId, name = roomName(room), size = size)
    } else {
        Image(
            loaded.asImageBitmap(),
            contentDescription = null,
            contentScale = ContentScale.Crop,
            modifier = Modifier.size(size).clip(CircleShape),
        )
    }
}
