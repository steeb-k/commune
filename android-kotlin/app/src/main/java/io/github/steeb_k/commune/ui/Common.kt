// Small shared pieces: initials avatars (the GTK app's Adw.Avatar fallback),
// and the loading screen.
package io.github.steeb_k.commune.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
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
    LoadingFace(modifier = Modifier.fillMaxSize())
}

/// The loading treatment everywhere something is not ready yet: the wavy
/// Material progress indicator across the top (the View library's stable
/// one) over the app's symbolic mark — never a blank page.
@Composable
fun LoadingFace(modifier: Modifier = Modifier) {
    Column(modifier = modifier) {
        val density = androidx.compose.ui.platform.LocalDensity.current
        androidx.compose.ui.viewinterop.AndroidView(
            factory = { context ->
                val themed = android.view.ContextThemeWrapper(
                    context,
                    com.google.android.material.R.style.Theme_Material3_DayNight_NoActionBar,
                )
                com.google.android.material.progressindicator.LinearProgressIndicator(themed)
                    .apply {
                        isIndeterminate = true
                        with(density) {
                            waveAmplitude = 3.dp.roundToPx()
                            setWavelength(24.dp.roundToPx())
                        }
                    }
            },
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp),
        )
        Box(
            modifier = Modifier.fillMaxSize(),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                androidx.compose.ui.res.painterResource(
                    io.github.steeb_k.commune.R.drawable.ic_app_symbolic
                ),
                contentDescription = null,
                tint = MaterialTheme.colorScheme.surfaceVariant,
                modifier = Modifier.size(96.dp),
            )
        }
    }
}


/// The loading treatment for pop-ins (sheets, dialogs): the wavy circular
/// indicator, centered. Full pages get [LoadingFace]'s horizontal bar;
/// anything that pops over the content gets the circle.
@Composable
fun LoadingRing(modifier: Modifier = Modifier) {
    Box(modifier = modifier, contentAlignment = Alignment.Center) {
        val density = androidx.compose.ui.platform.LocalDensity.current
        androidx.compose.ui.viewinterop.AndroidView(
            factory = { context ->
                val themed = android.view.ContextThemeWrapper(
                    context,
                    com.google.android.material.R.style.Theme_Material3_DayNight_NoActionBar,
                )
                com.google.android.material.progressindicator.CircularProgressIndicator(themed)
                    .apply {
                        isIndeterminate = true
                        with(density) {
                            indicatorSize = 48.dp.roundToPx()
                            waveAmplitude = 2.dp.roundToPx()
                            setWavelength(16.dp.roundToPx())
                        }
                    }
            },
        )
    }
}


/// Still images already decoded, keyed by path and size class. The
/// files are content-addressed cache entries, so nothing goes stale;
/// the cache only bounds how many decoded bitmaps stay warm. Animated
/// drawables are not shared — each view needs its own animation state.
private val decodedMedia =
    android.util.LruCache<String, android.graphics.drawable.Drawable>(64)

/// An image from a media file, animating when the platform decoder says
/// it animates (GIF, animated WebP); stills come out as plain drawables
/// from the same call. Decoding happens off the UI thread, downsampled
/// to the size class actually presented — a grid of photos must never
/// hold full-resolution bitmaps.
@Composable
fun MediaImage(
    path: String,
    contentDescription: String?,
    modifier: Modifier = Modifier,
    targetSizePx: Int = 1080,
    // Fixed frames — avatars, grid tiles — crop to fill rather than
    // letterbox; free-height bubbles keep the whole picture.
    fill: Boolean = false,
) {
    val drawable by androidx.compose.runtime.produceState<
        android.graphics.drawable.Drawable?,
    >(null, path, targetSizePx) {
        val key = "$path@$targetSizePx"
        value = decodedMedia.get(key) ?: kotlinx.coroutines.withContext(
            kotlinx.coroutines.Dispatchers.IO
        ) {
            try {
                val source =
                    android.graphics.ImageDecoder.createSource(java.io.File(path))
                android.graphics.ImageDecoder.decodeDrawable(source) { decoder, info, _ ->
                    val longest = maxOf(info.size.width, info.size.height)
                    if (longest > targetSizePx) {
                        val scale = targetSizePx.toFloat() / longest
                        decoder.setTargetSize(
                            (info.size.width * scale).toInt().coerceAtLeast(1),
                            (info.size.height * scale).toInt().coerceAtLeast(1),
                        )
                    }
                }.also {
                    if (it !is android.graphics.drawable.AnimatedImageDrawable) {
                        decodedMedia.put(key, it)
                    }
                }
            } catch (_: Exception) {
                null
            }
        }
    }
    val current = drawable ?: return

    // A still picture is drawn by Compose itself, so the frame's clip and
    // size hold: an ImageView inside a lazy grid painted past its cell and
    // the media grid became a collage. Animated pictures keep the view,
    // which is what plays them.
    val bitmap = (current as? android.graphics.drawable.BitmapDrawable)?.bitmap
    if (bitmap != null) {
        val image = androidx.compose.runtime.remember(bitmap) {
            bitmap.asImageBitmap()
        }
        androidx.compose.foundation.Image(
            bitmap = image,
            contentDescription = contentDescription,
            contentScale = if (fill) {
                androidx.compose.ui.layout.ContentScale.Crop
            } else {
                androidx.compose.ui.layout.ContentScale.Fit
            },
            modifier = modifier,
        )
        return
    }

    androidx.compose.ui.viewinterop.AndroidView(
        factory = { context ->
            android.widget.ImageView(context).apply {
                adjustViewBounds = !fill
                scaleType = if (fill) {
                    android.widget.ImageView.ScaleType.CENTER_CROP
                } else {
                    android.widget.ImageView.ScaleType.FIT_CENTER
                }
                this.contentDescription = contentDescription
            }
        },
        update = { view ->
            view.setImageDrawable(current)
            (current as? android.graphics.drawable.AnimatedImageDrawable)?.start()
        },
        modifier = modifier,
    )
}


/// The blurhash of a media event, decoded small and scaled up — what
/// the bubble shows until the real bytes arrive.
@Composable
fun BlurhashImage(blurhash: String, contentDescription: String?, modifier: Modifier = Modifier) {
    val bitmap = remember(blurhash) {
        io.github.steeb_k.commune.core.decodeBlurhash(blurhash, 32u, 32u)?.let { rgba ->
            val pixels = IntArray(32 * 32)
            for (i in pixels.indices) {
                val r = rgba[i * 4].toInt() and 0xFF
                val g = rgba[i * 4 + 1].toInt() and 0xFF
                val b = rgba[i * 4 + 2].toInt() and 0xFF
                pixels[i] = (0xFF shl 24) or (r shl 16) or (g shl 8) or b
            }
            android.graphics.Bitmap.createBitmap(pixels, 32, 32, android.graphics.Bitmap.Config.ARGB_8888)
        }
    } ?: return

    Image(
        bitmap.asImageBitmap(),
        contentDescription = contentDescription,
        contentScale = ContentScale.FillBounds,
        modifier = modifier,
    )
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
