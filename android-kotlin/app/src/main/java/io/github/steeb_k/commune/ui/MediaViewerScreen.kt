// The media viewer: pictures over black, a gallery of them a swipe away
// on either side, pinch to zoom and drag the zoomed picture around inside
// its own edges, double-tap to zoom in and out, tap, back or a swipe up or
// down to leave — the GTK media viewer's job, with the gestures Android
// hands feel around for.
package io.github.steeb_k.commune.ui

import androidx.compose.animation.core.animate
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.calculateCentroid
import androidx.compose.foundation.gestures.calculatePan
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChanged
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.unit.IntSize
import io.github.steeb_k.commune.ViewerPage
import kotlinx.coroutines.launch

/// How far a picture can be zoomed in, as a multiple of fitting the screen.
private const val MAX_SCALE = 8f
/// How far a double tap zooms in.
private const val DOUBLE_TAP_SCALE = 3f
/// How long the double-tap zoom takes, in ms.
private const val ZOOM_ANIMATION_MS = 250

@Composable
fun MediaViewerScreen(
    path: String,
    isVideo: Boolean = false,
    pages: List<ViewerPage> = emptyList(),
    initialPage: Int = 0,
    onClose: () -> Unit,
) {
    if (isVideo) {
        PlayerScreen(path, onClose)
        return
    }

    // A picture opened on its own is a gallery of one.
    val gallery = pages.ifEmpty { listOf(ViewerPage(path) { path }) }
    val pagerState = rememberPagerState(
        initialPage = initialPage.coerceIn(0, gallery.lastIndex),
    ) { gallery.size }

    // The pager takes the horizontal swipes a page leaves alone: those on
    // a picture at its natural size. A zoomed picture keeps its drags.
    HorizontalPager(
        state = pagerState,
        modifier = Modifier
            .fillMaxSize()
            .background(Color.Black),
        beyondViewportPageCount = 1,
        key = { gallery[it].key },
    ) { index ->
        ImagePage(gallery[index], onClose)
    }
}

/// One picture: fetched when its page comes on screen, fitted to it, and
/// zoomable from there. Pinching zooms about the fingers and a double tap
/// about itself; a zoomed picture drags within its own edges and no
/// further; at its natural size a sideways drag is left to the pager.
@Composable
private fun ImagePage(page: ViewerPage, onClose: () -> Unit) {
    val path by produceState<String?>(null, page.key) {
        value = page.load()
    }
    var scale by remember(page.key) { mutableFloatStateOf(1f) }
    var offset by remember(page.key) { mutableStateOf(Offset.Zero) }
    // How far a vertical swipe has pulled the picture at its natural size:
    // the GTK viewer's swipe to close, which follows the finger and lets
    // go past a quarter of the screen.
    var pull by remember(page.key) { mutableFloatStateOf(0f) }
    var imageSize by remember(page.key) { mutableStateOf<IntSize?>(null) }
    var pageSize by remember { mutableStateOf(IntSize.Zero) }
    val scope = rememberCoroutineScope()

    /// Keep the picture's edges at or beyond the page's: the size the
    /// picture fits the page at, scaled, less the page, is how far it can
    /// go either way.
    fun clamp(candidate: Offset, atScale: Float): Offset {
        val picture = imageSize
        if (picture == null || picture.width == 0 || picture.height == 0 ||
            pageSize.width == 0 || pageSize.height == 0
        ) {
            return Offset.Zero
        }
        val aspect = picture.width.toFloat() / picture.height
        val fittedWidth = minOf(pageSize.width.toFloat(), pageSize.height * aspect)
        val fittedHeight = fittedWidth / aspect
        val maxX = ((fittedWidth * atScale - pageSize.width) / 2f).coerceAtLeast(0f)
        val maxY = ((fittedHeight * atScale - pageSize.height) / 2f).coerceAtLeast(0f)
        return Offset(
            candidate.x.coerceIn(-maxX, maxX),
            candidate.y.coerceIn(-maxY, maxY),
        )
    }

    /// A double tap zooms in on the spot under it, or back out to fit,
    /// as the messaging apps around this one do.
    fun toggleZoom(tap: Offset) {
        val fromScale = scale
        val fromOffset = offset
        val toScale = if (scale > 1f) 1f else DOUBLE_TAP_SCALE
        val toOffset = if (toScale == 1f) {
            Offset.Zero
        } else {
            val centre = Offset(pageSize.width / 2f, pageSize.height / 2f)
            val focus = tap - centre
            clamp((fromOffset - focus) * (toScale / fromScale) + focus, toScale)
        }
        scope.launch {
            animate(0f, 1f, animationSpec = tween(ZOOM_ANIMATION_MS)) { t, _ ->
                scale = fromScale + (toScale - fromScale) * t
                offset = fromOffset + (toOffset - fromOffset) * t
            }
        }
    }

    Box(
        modifier = Modifier
            .fillMaxSize()
            .onSizeChanged { pageSize = it }
            .pointerInput(page.key) {
                detectTapGestures(
                    onTap = { onClose() },
                    onDoubleTap = { toggleZoom(it) },
                )
            }
            .pointerInput(page.key) {
                awaitEachGesture {
                    awaitFirstDown(requireUnconsumed = false)
                    // Where a single finger has gone since it landed, and
                    // whether that became a vertical pull.
                    var travelled = Offset.Zero
                    var pulling = false
                    while (true) {
                        val event = awaitPointerEvent()
                        val pressed = event.changes.filter { it.pressed }
                        if (pressed.isEmpty()) break

                        val zoomChange = event.calculateZoom()
                        val pan = event.calculatePan()
                        if (zoomChange == 1f && pan == Offset.Zero) continue

                        val pinching = pressed.size > 1
                        if (!pinching && scale <= 1f) {
                            // A single finger on a picture at its natural
                            // size: sideways it is the pager's swipe, not
                            // ours; up or down it pulls the picture along.
                            travelled += pan
                            if (!pulling) {
                                val vertical = kotlin.math.abs(travelled.y)
                                if (vertical < viewConfiguration.touchSlop ||
                                    vertical < kotlin.math.abs(travelled.x)
                                ) {
                                    continue
                                }
                                pulling = true
                            }
                            pull += pan.y
                            event.changes.forEach {
                                if (it.positionChanged()) it.consume()
                            }
                            continue
                        }

                        val newScale = (scale * zoomChange).coerceIn(1f, MAX_SCALE)
                        // Zoom about the fingers: the spot under them stays
                        // under them, then the pan moves everything.
                        val centre = Offset(size.width / 2f, size.height / 2f)
                        val focus = event.calculateCentroid() - centre
                        val grown = newScale / scale
                        offset = clamp((offset - focus) * grown + focus + pan, newScale)
                        scale = newScale

                        event.changes.forEach {
                            if (it.positionChanged()) it.consume()
                        }
                    }

                    if (pulling) {
                        // Let go far enough and the picture is dismissed;
                        // otherwise it settles back where it was.
                        if (kotlin.math.abs(pull) > size.height / 4f) {
                            onClose()
                        } else {
                            val from = pull
                            scope.launch {
                                animate(from, 0f) { value, _ -> pull = value }
                            }
                        }
                    }
                }
            },
        contentAlignment = Alignment.Center,
    ) {
        val current = path ?: return@Box
        MediaImage(
            current,
            contentDescription = null,
            targetSizePx = 2160,
            onSize = { imageSize = it },
            modifier = Modifier
                .fillMaxSize()
                .graphicsLayer(
                    scaleX = scale,
                    scaleY = scale,
                    translationX = offset.x,
                    translationY = offset.y + pull,
                ),
        )
    }
}


/// Video or audio playback over black, with the player's own controls.
@Composable
private fun PlayerScreen(path: String, onClose: () -> Unit) {
    val context = androidx.compose.ui.platform.LocalContext.current
    val player = remember(path) {
        androidx.media3.exoplayer.ExoPlayer.Builder(context).build().apply {
            setMediaItem(androidx.media3.common.MediaItem.fromUri(android.net.Uri.fromFile(java.io.File(path))))
            prepare()
            playWhenReady = true
        }
    }
    androidx.compose.runtime.DisposableEffect(path) {
        onDispose { player.release() }
    }

    Box(
        modifier = Modifier
            .fillMaxSize()
            .background(Color.Black),
        contentAlignment = Alignment.Center,
    ) {
        androidx.compose.ui.viewinterop.AndroidView(
            factory = { viewContext ->
                androidx.media3.ui.PlayerView(viewContext).apply {
                    this.player = player
                    setShowNextButton(false)
                    setShowPreviousButton(false)
                }
            },
            modifier = Modifier.fillMaxSize(),
        )
    }
}
