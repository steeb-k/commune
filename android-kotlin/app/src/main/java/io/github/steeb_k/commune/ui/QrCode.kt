package io.github.steeb_k.commune.ui

import androidx.compose.foundation.Image
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.qrcode.QRCodeWriter

/// A QR code for the given bytes, drawn as the GTK verification page
/// draws its own: the verification's bytes encoded one to one, which is
/// what the other session's scanner expects to read back.
@Composable
fun QrCodeImage(bytes: ByteArray, modifier: Modifier = Modifier, sizePx: Int = 512) {
    val bitmap = remember(bytes) {
        // ISO-8859-1 maps every byte to one character and back, so the
        // bytes survive the writer's string interface untouched.
        val text = String(bytes, Charsets.ISO_8859_1)
        val matrix = QRCodeWriter().encode(
            text,
            BarcodeFormat.QR_CODE,
            sizePx,
            sizePx,
            mapOf(EncodeHintType.CHARACTER_SET to "ISO-8859-1", EncodeHintType.MARGIN to 1),
        )
        val pixels = IntArray(sizePx * sizePx) { index ->
            val x = index % sizePx
            val y = index / sizePx
            if (matrix[x, y]) android.graphics.Color.BLACK else android.graphics.Color.WHITE
        }
        android.graphics.Bitmap.createBitmap(pixels, sizePx, sizePx, android.graphics.Bitmap.Config.ARGB_8888)
    }
    Image(
        bitmap.asImageBitmap(),
        contentDescription = "QR code",
        modifier = modifier,
        contentScale = ContentScale.Fit,
    )
}
