// The rooms other apps can share straight into: Android's sharing
// shortcuts, one per recent room, listed on the share sheet as
// "Commune → <room>" with the room's avatar (the sheet badges it with the
// app icon itself). The same shortcuts show on the launcher icon's
// long-press menu and open their room. Element does the same on Android;
// the GTK app has no share sheet to mirror.
package io.github.steeb_k.commune

import android.app.Person
import android.content.Context
import android.content.Intent
import android.content.pm.ShortcutInfo
import android.content.pm.ShortcutManager
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Matrix
import android.graphics.Paint
import android.graphics.RectF
import android.graphics.Typeface
import android.graphics.drawable.Icon
import android.os.Build
import io.github.steeb_k.commune.core.CoreApp
import io.github.steeb_k.commune.core.FfiRoom
import io.github.steeb_k.commune.core.FfiRoomCategory
import io.github.steeb_k.commune.ui.avatarColorsArgb
import io.github.steeb_k.commune.ui.avatarInitial
import io.github.steeb_k.commune.ui.roomName
import kotlin.concurrent.thread
import kotlinx.coroutines.runBlocking

class ShareShortcuts(private val context: Context, private val app: CoreApp) {
    private val manager = context.getSystemService(ShortcutManager::class.java)

    /// What identifies the shortcuts last published: the rooms in order,
    /// with what their entries show. The room list arrives with every
    /// sync, and the system rate-limits republishing, so the same set is
    /// not pushed twice.
    private data class Key(
        val roomId: String,
        val name: String,
        val avatarUrl: String?,
    )

    private var published: List<Key> = emptyList()

    /// A counter for the avatar work in flight: a newer set makes an
    /// older one's result moot.
    @Volatile
    private var generation = 0

    /// Publish the most recently active rooms that can be shared into.
    fun update(rooms: List<FfiRoom>) {
        val limit = minOf(manager.maxShortcutCountPerActivity, MAX_TARGETS)
        val targets = rooms
            .filter { it.category in SHAREABLE }
            .sortedByDescending { it.latestActivity }
            .take(limit)
        val keys = targets.map { Key(it.roomId, roomName(it), it.avatarUrl) }
        if (keys == published) return
        published = keys
        val mine = ++generation

        // Avatars come from the media cache or the network: off the main
        // thread, and dropped if the list moved on meanwhile.
        thread {
            val shortcuts = targets.mapIndexed { rank, room -> shortcut(room, rank) }
            if (mine != generation) return@thread
            try {
                manager.dynamicShortcuts = shortcuts
            } catch (_: Exception) {
                // Rate-limited or otherwise refused: the next change of
                // the list tries again.
                published = emptyList()
            }
        }
    }

    /// Withdraw every shortcut: the account is gone.
    fun clear() {
        published = emptyList()
        generation++
        try {
            manager.removeAllDynamicShortcuts()
        } catch (_: Exception) {
        }
    }

    private fun shortcut(room: FfiRoom, rank: Int): ShortcutInfo {
        val name = roomName(room)
        val icon = Icon.createWithAdaptiveBitmap(avatarBitmap(room, name))
        val person = Person.Builder()
            .setName(name)
            .setKey(room.roomId)
            .setIcon(icon)
            .build()
        // The launcher's tap opens the room, the way a notification's does.
        val open = Intent(context, MainActivity::class.java)
            .setAction(MainActivity.ACTION_OPEN_ROOM)
            .putExtra("room_id", room.roomId)
        val builder = ShortcutInfo.Builder(context, room.roomId)
            .setShortLabel(name)
            .setLongLabel(name)
            .setIcon(icon)
            .setIntent(open)
            .setCategories(setOf(SHARE_CATEGORY))
            .setPerson(person)
            .setRank(rank)
        if (Build.VERSION.SDK_INT >= 30) builder.setLongLived(true)
        return builder.build()
    }

    /// The room's avatar as an adaptive icon: the picture filling the
    /// canvas, or the initials avatar the room list shows. The system
    /// masks the icon to its shape and shows the centre of it.
    private fun avatarBitmap(room: FfiRoom, name: String): Bitmap {
        val size = ICON_SIZE
        val out = Bitmap.createBitmap(size, size, Bitmap.Config.ARGB_8888)
        val canvas = Canvas(out)

        val picture = try {
            runBlocking { app.getRoomAvatar(room.roomId, size.toUInt()) }
                ?.let { BitmapFactory.decodeFile(it) }
        } catch (_: Exception) {
            null
        }
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

        val (background, foreground) = avatarColorsArgb(room.roomId)
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

    companion object {
        /// The category `res/xml/shortcuts.xml` matches share targets by.
        const val SHARE_CATEGORY = "io.github.steeb_k.commune.SHARE_TARGET"

        /// How many rooms are offered: the share sheet shows a handful and
        /// the launcher menu fewer still.
        private const val MAX_TARGETS = 8

        /// The adaptive icon canvas, in pixels: 108dp at 2x.
        private const val ICON_SIZE = 216

        /// The rooms a message can be sent to from outside: joined ones,
        /// not invitations, spaces or the server's notices.
        private val SHAREABLE = setOf(
            FfiRoomCategory.FAVORITE,
            FfiRoomCategory.NORMAL,
            FfiRoomCategory.LOW_PRIORITY,
        )
    }
}
