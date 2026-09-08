// Avatars drawn for the system rather than for Compose: the picture
// filling a square canvas, or the initials avatar the room list shows,
// as an adaptive icon the system masks to its shape. Sharing shortcuts
// use it for rooms, message notifications for senders.
package io.github.steeb_k.commune

import android.app.Person
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Matrix
import android.graphics.Paint
import android.graphics.RectF
import android.graphics.Typeface
import android.graphics.drawable.Icon
import io.github.steeb_k.commune.core.CoreApp
import io.github.steeb_k.commune.ui.avatarColorsArgb
import io.github.steeb_k.commune.ui.avatarInitial
import io.github.steeb_k.commune.ui.localpart
import kotlinx.coroutines.runBlocking

/// The adaptive icon canvas, in pixels: 108dp at 2x.
internal const val AVATAR_ICON_SIZE = 216

/// The avatar as a square bitmap: `picture` scaled to fill it when there
/// is one, the initials avatar in the identifier's colour otherwise. The
/// system masks the icon to its shape and shows the centre of it.
internal fun avatarBitmap(identifier: String, name: String, picture: Bitmap?): Bitmap {
    val size = AVATAR_ICON_SIZE
    val out = Bitmap.createBitmap(size, size, Bitmap.Config.ARGB_8888)
    val canvas = Canvas(out)

    if (picture != null) {
        val scale = maxOf(size.toFloat() / picture.width, size.toFloat() / picture.height)
        val matrix = Matrix().apply {
            setScale(scale, scale)
            postTranslate(
                (size - picture.width * scale) / 2f,
                (size - picture.height * scale) / 2f,
            )
        }
        canvas.drawBitmap(picture, matrix, Paint(Paint.FILTER_BITMAP_FLAG))
        return out
    }

    val (background, foreground) = avatarColorsArgb(identifier)
    canvas.drawRect(RectF(0f, 0f, size.toFloat(), size.toFloat()), Paint().apply { color = background })
    // The letter sized to the part the mask reveals — the middle two
    // thirds — as the room list's 0.45 of its circle.
    val text = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = foreground
        typeface = Typeface.DEFAULT_BOLD
        textSize = size * 0.30f
        textAlign = Paint.Align.CENTER
    }
    val baseline = size / 2f - (text.descent() + text.ascent()) / 2f
    canvas.drawText(avatarInitial(name), size / 2f, baseline, text)
    return out
}

/// A message's sender as the shade's Person.
///
/// Keyed by their Matrix ID, so the system files one person's messages
/// under one person however their name is spelled — the sidebar's count
/// and the push for the same event used to name the sender differently,
/// one by localpart and one by display name, and the shade showed every
/// message twice. Named as the room knows them, the way the GTK
/// notifications name a sender: the display name, with the ID appended
/// when another member shares it, the localpart when the store has no
/// member. Pictured with their avatar, or the initials avatar the member
/// list shows.
///
/// `fallbackName` is what to call them when the store has no member —
/// the name a push payload carries, say. Reads the store and may fetch
/// the picture: call off the main thread.
internal fun senderPerson(
    app: CoreApp,
    roomId: String,
    senderId: String,
    fallbackName: String? = null,
): Person {
    val member = try {
        runBlocking { app.roomMember(roomId, senderId) }
    } catch (_: Exception) {
        null
    }
    val name = when {
        member == null -> fallbackName?.takeIf { it.isNotBlank() } ?: localpart(senderId)
        member.isNameAmbiguous -> "${member.displayName} ($senderId)"
        else -> member.displayName
    }
    val picture = member?.avatarUrl?.let { url ->
        try {
            runBlocking { app.getAvatar(url, AVATAR_ICON_SIZE.toUInt()) }
                ?.let { BitmapFactory.decodeFile(it) }
        } catch (_: Exception) {
            null
        }
    }
    return Person.Builder()
        .setKey(senderId)
        .setName(name)
        .setIcon(Icon.createWithAdaptiveBitmap(avatarBitmap(senderId, name, picture)))
        .build()
}
