// The sound of a phone ringing. The GTK app ships its own ringtone;
// Android already has one the person chose, and using it means a call
// from Commune sounds like a call.
package io.github.steeb_k.commune

import android.content.Context
import android.media.AudioAttributes
import android.media.RingtoneManager

object Ringtone {
    private var playing: android.media.Ringtone? = null

    fun start(context: Context) {
        if (playing != null) return
        val uri = RingtoneManager.getActualDefaultRingtoneUri(
            context,
            RingtoneManager.TYPE_RINGTONE,
        ) ?: RingtoneManager.getDefaultUri(RingtoneManager.TYPE_RINGTONE) ?: return

        playing = RingtoneManager.getRingtone(context, uri)?.apply {
            audioAttributes = AudioAttributes.Builder()
                .setUsage(AudioAttributes.USAGE_NOTIFICATION_RINGTONE)
                .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
                .build()
            isLooping = true
            play()
        }
    }

    fun stop() {
        try {
            playing?.stop()
        } catch (_: Exception) {
            // A ringtone that already stopped needs no stopping.
        }
        playing = null
    }
}
