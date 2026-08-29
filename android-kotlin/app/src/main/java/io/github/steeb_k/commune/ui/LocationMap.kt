// A shared location is a place, and a place wants a picture of itself.
// The GTK app draws one with libshumate; this draws the same OpenStreetMap
// tile by hand, because the project takes no map SDK and no image-loading
// library for one bubble.
package io.github.steeb_k.commune.ui

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.util.LruCache
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Place
import androidx.compose.material3.Icon
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
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.net.URL
import kotlin.math.PI
import kotlin.math.asinh
import kotlin.math.floor
import kotlin.math.tan

/// Close enough to read a street name, which is what a shared location is
/// for.
private const val ZOOM = 16

/// A tile is 256 pixels square, by the slippy-map convention every tile
/// server follows.
private const val TILE = 256

/// The tiles already fetched. A room full of locations must not re-fetch a
/// tile every time its bubble scrolls back into view — the OpenStreetMap
/// tile policy asks that of anybody using it, and it is the right thing
/// anyway.
private val tiles = LruCache<String, Bitmap>(32)

/// Where a point sits on the world map at [ZOOM], in tiles. The whole
/// number is which tile; the fraction is where in it.
private fun tileOf(latitude: Double, longitude: Double): Pair<Double, Double> {
    val scale = 1 shl ZOOM
    val x = (longitude + 180.0) / 360.0 * scale
    val radians = latitude * PI / 180.0
    val y = (1.0 - asinh(tan(radians)) / PI) / 2.0 * scale
    return x to y
}

/// The latitude and longitude out of a `geo:` URI, which may carry an
/// accuracy or other parameters after them.
internal fun parseGeoUri(uri: String): Pair<Double, Double>? {
    val body = uri.removePrefix("geo:").substringBefore(';').substringBefore('?')
    val parts = body.split(',')
    if (parts.size < 2) return null
    val latitude = parts[0].trim().toDoubleOrNull() ?: return null
    val longitude = parts[1].trim().toDoubleOrNull() ?: return null
    if (latitude !in -90.0..90.0 || longitude !in -180.0..180.0) return null
    return latitude to longitude
}

@Composable
fun LocationMap(geoUri: String, modifier: Modifier = Modifier) {
    val point = remember(geoUri) { parseGeoUri(geoUri) }
    if (point == null) {
        // Not a location this can draw; the caller still offers the link.
        return
    }
    val (latitude, longitude) = point
    val (tileX, tileY) = remember(geoUri) { tileOf(latitude, longitude) }
    val column = floor(tileX).toInt()
    val row = floor(tileY).toInt()

    var bitmap by remember(geoUri) { mutableStateOf<Bitmap?>(null) }
    LaunchedEffect(geoUri) {
        val key = "$ZOOM/$column/$row"
        tiles.get(key)?.let {
            bitmap = it
            return@LaunchedEffect
        }
        bitmap = withContext(Dispatchers.IO) {
            try {
                val connection = URL("https://tile.openstreetmap.org/$key.png")
                    .openConnection() as java.net.HttpURLConnection
                // The tile policy asks that every client identify itself.
                connection.setRequestProperty(
                    "User-Agent",
                    "Commune/1.0 (Matrix client; +https://github.com/steeb-k/commune)",
                )
                connection.connectTimeout = 10_000
                connection.readTimeout = 10_000
                connection.inputStream.use { BitmapFactory.decodeStream(it) }
            } catch (_: Exception) {
                // No tile is a bubble without a picture, not a broken one.
                null
            }
        }?.also { tiles.put(key, it) }
    }

    BoxWithConstraints(
        modifier = modifier
            .width(240.dp)
            .background(MaterialTheme.colorScheme.surfaceVariant),
    ) {
        val side = maxWidth
        Box(modifier = Modifier.size(side)) {
            val picture = bitmap
            if (picture != null) {
                Image(
                    bitmap = picture.asImageBitmap(),
                    contentDescription = "Map of the shared location",
                    contentScale = ContentScale.FillBounds,
                    modifier = Modifier.fillMaxSize(),
                )
                // The pin goes where in the tile the point actually falls.
                val fractionX = (tileX - column).toFloat()
                val fractionY = (tileY - row).toFloat()
                Icon(
                    Icons.Filled.Place,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.error,
                    modifier = Modifier
                        .size(32.dp)
                        // Place the pin's point, not its corner, on the spot.
                        .offset(
                            x = side * fractionX - 16.dp,
                            y = side * fractionY - 32.dp,
                        ),
                )
            } else {
                Text(
                    "Loading the map…",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.align(Alignment.Center),
                )
            }
        }
    }
}

/// The tile a point falls in, at [ZOOM]. Exposed for the tests.
internal fun tileIndexFor(latitude: Double, longitude: Double): Pair<Int, Int> {
    val (x, y) = tileOf(latitude, longitude)
    return floor(x).toInt() to floor(y).toInt()
}
