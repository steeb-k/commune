// What the GTK message toolbar measures about a file before it sends it,
// done with Android's media stack: a picture's dimensions and Blurhash and,
// past the toolbar's thresholds, a downscaled thumbnail; a video's
// dimensions, duration, and its first frame as the thumbnail; an audio
// file's duration. The core sends the file size alone and owes nothing
// else, so without this a video lands in the room as a bare file card with
// no still to show.
package io.github.steeb_k.commune

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Matrix
import android.media.ExifInterface
import android.media.MediaMetadataRetriever
import android.os.Build
import io.github.steeb_k.commune.core.FfiMediaInfo
import io.github.steeb_k.commune.core.FfiThumbnail
import java.io.File
import kotlin.math.PI
import kotlin.math.abs
import kotlin.math.cos
import kotlin.math.floor
import kotlin.math.max
import kotlin.math.min
import kotlin.math.pow
import kotlin.math.roundToInt

object MediaMeasure {
    /// The toolbar's `THUMBNAIL_MAX_DIMENSIONS`, in logical pixels; it
    /// scales them by the display's scale factor, and so does this.
    private const val MAX_WIDTH = 600
    private const val MAX_HEIGHT = 400

    /// The toolbar's `THUMBNAIL_DIMENSIONS_THRESHOLD`: a picture that
    /// exceeds the maximum by less than this is sent as it is.
    private const val DIMENSIONS_THRESHOLD = 200

    /// The toolbar's `THUMBNAIL_MAX_FILESIZE_THRESHOLD`: a picture heavier
    /// than this gets a thumbnail whatever its dimensions.
    private const val MAX_FILESIZE = 1024L * 1024L

    /// The toolbar's `WEBP_DEFAULT_QUALITY`.
    private const val WEBP_QUALITY = 60
    private const val WEBP_MIME = "image/webp"

    /// The scale factor the maximum dimensions grow by: the display's,
    /// as on the desktop, but capped — a phone at 3x would upload a
    /// 1800-pixel "thumbnail" for every picture, and the room shows it far
    /// smaller than that.
    private fun scaleFactor(context: Context): Int =
        context.resources.displayMetrics.density.roundToInt().coerceIn(1, 2)

    /// Measure the file at `path` of the given MIME type. Null for a kind
    /// the toolbar does not measure; never throws, since the attachment
    /// goes with whatever could be measured.
    fun measure(context: Context, path: String, mime: String, size: Long): FfiMediaInfo? =
        try {
            when {
                mime.startsWith("image/") -> measureImage(context, path, size)
                mime.startsWith("video/") -> measureVideo(context, path)
                mime.startsWith("audio/") -> measureAudio(path)
                else -> null
            }
        } catch (_: Exception) {
            null
        }

    private fun info(
        width: Int? = null,
        height: Int? = null,
        durationMs: Long? = null,
        blurhash: String? = null,
        thumbnail: FfiThumbnail? = null,
    ) = FfiMediaInfo(
        width = width?.toUInt(),
        height = height?.toUInt(),
        durationMs = durationMs?.toULong(),
        blurhash = blurhash,
        thumbnail = thumbnail,
    )

    // ---- Pictures ----

    private fun measureImage(context: Context, path: String, size: Long): FfiMediaInfo? {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeFile(path, bounds)
        if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return null

        // The dimensions as the picture is seen: a photo straight from a
        // camera is often stored on its side with the turn in its EXIF,
        // which the desktop's decoder applies before measuring.
        val orientation = exifOrientation(path)
        val turned = orientation == ExifInterface.ORIENTATION_ROTATE_90 ||
            orientation == ExifInterface.ORIENTATION_ROTATE_270
        val width = if (turned) bounds.outHeight else bounds.outWidth
        val height = if (turned) bounds.outWidth else bounds.outHeight

        val factor = scaleFactor(context)
        val maxWidth = MAX_WIDTH * factor
        val maxHeight = MAX_HEIGHT * factor
        val needsThumbnail = size > MAX_FILESIZE ||
            width >= maxWidth + DIMENSIONS_THRESHOLD ||
            height >= maxHeight + DIMENSIONS_THRESHOLD

        if (!needsThumbnail) {
            // Not worth a thumbnail; the Blurhash still comes from the
            // picture, read small since the hash sees no detail anyway.
            val small = decodeScaled(path, orientation, BLURHASH_SOURCE, BLURHASH_SOURCE)
            return info(width, height, blurhash = small?.let(::blurhash))
        }

        val thumbnail = decodeScaled(path, orientation, maxWidth, maxHeight) ?: return info(width, height)
        val encoded = encodeThumbnail(context, thumbnail)
        return info(width, height, blurhash = blurhash(thumbnail), thumbnail = encoded)
    }

    private fun exifOrientation(path: String): Int =
        try {
            ExifInterface(path).getAttributeInt(
                ExifInterface.TAG_ORIENTATION,
                ExifInterface.ORIENTATION_NORMAL,
            )
        } catch (_: Exception) {
            ExifInterface.ORIENTATION_NORMAL
        }

    /// Decode the picture at `path` to fit within the given dimensions,
    /// the EXIF turn applied: subsampled by the decoder to no smaller than
    /// the target, then scaled down to fit it exactly.
    private fun decodeScaled(path: String, orientation: Int, maxWidth: Int, maxHeight: Int): Bitmap? {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeFile(path, bounds)
        if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return null
        // The target in the stored frame's own orientation.
        val turned = orientation == ExifInterface.ORIENTATION_ROTATE_90 ||
            orientation == ExifInterface.ORIENTATION_ROTATE_270
        val targetWidth = if (turned) maxHeight else maxWidth
        val targetHeight = if (turned) maxWidth else maxHeight

        var sample = 1
        while (bounds.outWidth / (sample * 2) >= targetWidth &&
            bounds.outHeight / (sample * 2) >= targetHeight
        ) {
            sample *= 2
        }
        val decoded = BitmapFactory.decodeFile(
            path,
            BitmapFactory.Options().apply { inSampleSize = sample },
        ) ?: return null
        val fitted = scaleToFit(decoded, targetWidth, targetHeight)
        return applyOrientation(fitted, orientation)
    }

    private fun applyOrientation(bitmap: Bitmap, orientation: Int): Bitmap {
        val matrix = Matrix()
        when (orientation) {
            ExifInterface.ORIENTATION_ROTATE_90 -> matrix.postRotate(90f)
            ExifInterface.ORIENTATION_ROTATE_180 -> matrix.postRotate(180f)
            ExifInterface.ORIENTATION_ROTATE_270 -> matrix.postRotate(270f)
            ExifInterface.ORIENTATION_FLIP_HORIZONTAL -> matrix.postScale(-1f, 1f)
            ExifInterface.ORIENTATION_FLIP_VERTICAL -> matrix.postScale(1f, -1f)
            ExifInterface.ORIENTATION_TRANSPOSE -> {
                matrix.postRotate(90f)
                matrix.postScale(-1f, 1f)
            }
            ExifInterface.ORIENTATION_TRANSVERSE -> {
                matrix.postRotate(270f)
                matrix.postScale(-1f, 1f)
            }
            else -> return bitmap
        }
        return Bitmap.createBitmap(bitmap, 0, 0, bitmap.width, bitmap.height, matrix, true)
    }

    // ---- Videos ----

    private fun measureVideo(context: Context, path: String): FfiMediaInfo? {
        val retriever = MediaMetadataRetriever()
        try {
            retriever.setDataSource(path)
            val rotation = retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_ROTATION)
                ?.toIntOrNull() ?: 0
            val storedWidth = retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_WIDTH)
                ?.toIntOrNull()
            val storedHeight = retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_HEIGHT)
                ?.toIntOrNull()
            val turned = rotation == 90 || rotation == 270
            val width = if (turned) storedHeight else storedWidth
            val height = if (turned) storedWidth else storedHeight
            val durationMs = retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_DURATION)
                ?.toLongOrNull()

            // The first frame, as the toolbar's thumbnailer pipeline takes
            // it; the retriever hands it back already turned upright.
            val frame = retriever.getFrameAtTime(0, MediaMetadataRetriever.OPTION_CLOSEST_SYNC)
                ?: return info(width, height, durationMs)
            val factor = scaleFactor(context)
            val fitted = scaleToFit(frame, MAX_WIDTH * factor, MAX_HEIGHT * factor)
            val encoded = encodeThumbnail(context, fitted)
            return info(width, height, durationMs, blurhash(fitted), encoded)
        } finally {
            try {
                retriever.release()
            } catch (_: Exception) {
            }
        }
    }

    // ---- Audio ----

    private fun measureAudio(path: String): FfiMediaInfo? {
        val retriever = MediaMetadataRetriever()
        try {
            retriever.setDataSource(path)
            val durationMs = retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_DURATION)
                ?.toLongOrNull() ?: return null
            return info(durationMs = durationMs)
        } finally {
            try {
                retriever.release()
            } catch (_: Exception) {
            }
        }
    }

    // ---- Thumbnails ----

    /// The toolbar's `downscale_for`: scale down to fit the maximum
    /// dimensions, keeping the aspect, only when either side reaches them.
    private fun scaleToFit(bitmap: Bitmap, maxWidth: Int, maxHeight: Int): Bitmap {
        if (bitmap.width < maxWidth && bitmap.height < maxHeight) return bitmap
        val ratio = min(maxWidth.toFloat() / bitmap.width, maxHeight.toFloat() / bitmap.height)
        val width = max(1, (bitmap.width * ratio).roundToInt())
        val height = max(1, (bitmap.height * ratio).roundToInt())
        if (width == bitmap.width && height == bitmap.height) return bitmap
        return Bitmap.createScaledBitmap(bitmap, width, height, true)
    }

    /// Encode the thumbnail as the toolbar does — WebP at its quality —
    /// into a file under the cache, which the core reads and removes when
    /// it sends.
    private fun encodeThumbnail(context: Context, bitmap: Bitmap): FfiThumbnail? {
        val dir = File(context.cacheDir, "thumbnails").apply { mkdirs() }
        val file = File(dir, "thumbnail-${System.nanoTime()}.webp")
        val format = if (Build.VERSION.SDK_INT >= 30) {
            Bitmap.CompressFormat.WEBP_LOSSY
        } else {
            @Suppress("DEPRECATION")
            Bitmap.CompressFormat.WEBP
        }
        val written = file.outputStream().use { bitmap.compress(format, WEBP_QUALITY, it) }
        if (!written) {
            file.delete()
            return null
        }
        return FfiThumbnail(
            path = file.absolutePath,
            mimeType = WEBP_MIME,
            width = bitmap.width.toUInt(),
            height = bitmap.height.toUInt(),
        )
    }

    // ---- Blurhash ----

    /// The longest side a picture is read down to before hashing: the
    /// hash is a handful of cosine terms and sees nothing finer.
    private const val BLURHASH_SOURCE = 64

    private const val BASE83 =
        "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz#\$%*+,-.:;=?@[]^_{|}~"

    /// The Blurhash of the picture, with the toolbar's component counts:
    /// 3×4 for a portrait, 4×3 for a landscape, 3×3 for a square.
    private fun blurhash(source: Bitmap): String? {
        val bitmap = scaleToFit(source, BLURHASH_SOURCE, BLURHASH_SOURCE)
        val width = bitmap.width
        val height = bitmap.height
        if (width == 0 || height == 0) return null
        val (componentsX, componentsY) = when {
            width < height -> 3 to 4
            width > height -> 4 to 3
            else -> 3 to 3
        }

        val pixels = IntArray(width * height)
        bitmap.getPixels(pixels, 0, width, 0, 0, width, height)
        val linear = FloatArray(width * height * 3)
        for (index in pixels.indices) {
            val pixel = pixels[index]
            linear[index * 3] = srgbToLinear((pixel shr 16) and 0xFF)
            linear[index * 3 + 1] = srgbToLinear((pixel shr 8) and 0xFF)
            linear[index * 3 + 2] = srgbToLinear(pixel and 0xFF)
        }
        val cosX = Array(componentsX) { i ->
            FloatArray(width) { x -> cos(PI * i * x / width).toFloat() }
        }
        val cosY = Array(componentsY) { j ->
            FloatArray(height) { y -> cos(PI * j * y / height).toFloat() }
        }

        val factors = ArrayList<FloatArray>(componentsX * componentsY)
        for (j in 0 until componentsY) {
            for (i in 0 until componentsX) {
                val normalisation = if (i == 0 && j == 0) 1f else 2f
                var r = 0f
                var g = 0f
                var b = 0f
                for (y in 0 until height) {
                    val basisY = cosY[j][y]
                    for (x in 0 until width) {
                        val basis = normalisation * cosX[i][x] * basisY
                        val offset = (y * width + x) * 3
                        r += basis * linear[offset]
                        g += basis * linear[offset + 1]
                        b += basis * linear[offset + 2]
                    }
                }
                val scale = 1f / (width * height)
                factors.add(floatArrayOf(r * scale, g * scale, b * scale))
            }
        }

        val dc = factors[0]
        val ac = factors.drop(1)
        val hash = StringBuilder()
        hash.append(encode83((componentsX - 1) + (componentsY - 1) * 9, 1))

        val maximumValue: Float
        if (ac.isNotEmpty()) {
            val actualMaximum = ac.maxOf { component -> component.maxOf { abs(it) } }
            val quantised = floor(max(0f, min(82f, floor(actualMaximum * 166f - 0.5f)))).toInt()
            maximumValue = (quantised + 1) / 166f
            hash.append(encode83(quantised, 1))
        } else {
            maximumValue = 1f
            hash.append(encode83(0, 1))
        }

        hash.append(encode83(encodeDc(dc), 4))
        for (component in ac) hash.append(encode83(encodeAc(component, maximumValue), 2))
        return hash.toString()
    }

    private fun encodeDc(value: FloatArray): Int =
        (linearToSrgb(value[0]) shl 16) + (linearToSrgb(value[1]) shl 8) + linearToSrgb(value[2])

    private fun encodeAc(value: FloatArray, maximumValue: Float): Int {
        fun quantise(component: Float): Int =
            floor(max(0f, min(18f, floor(signPow(component / maximumValue, 0.5f) * 9f + 9.5f)))).toInt()
        return quantise(value[0]) * 19 * 19 + quantise(value[1]) * 19 + quantise(value[2])
    }

    private fun signPow(value: Float, exponent: Float): Float =
        if (value < 0f) -abs(value).pow(exponent) else abs(value).pow(exponent)

    private fun srgbToLinear(value: Int): Float {
        val v = value / 255f
        return if (v <= 0.04045f) v / 12.92f else ((v + 0.055f) / 1.055f).pow(2.4f)
    }

    private fun linearToSrgb(value: Float): Int {
        val v = max(0f, min(1f, value))
        val srgb = if (v <= 0.0031308f) v * 12.92f else 1.055f * v.pow(1f / 2.4f) - 0.055f
        return (srgb * 255f + 0.5f).toInt().coerceIn(0, 255)
    }

    private fun encode83(value: Int, length: Int): String {
        val out = CharArray(length)
        var remaining = value
        for (index in length - 1 downTo 0) {
            out[index] = BASE83[remaining % 83]
            remaining /= 83
        }
        return String(out)
    }
}
