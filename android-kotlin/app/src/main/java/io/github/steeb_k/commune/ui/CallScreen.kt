// The call: who it is with, where it has got to, and the two or three
// things a person does during one — the GTK call_view, on a phone.
package io.github.steeb_k.commune.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Call
import androidx.compose.material.icons.filled.CallEnd
import androidx.compose.material.icons.filled.Cameraswitch
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.filled.Mic
import androidx.compose.material.icons.filled.MicOff
import androidx.compose.material.icons.filled.Videocam
import androidx.compose.material.icons.filled.VideocamOff
import androidx.compose.material.icons.filled.VolumeUp
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import io.github.steeb_k.commune.CommuneState

@Composable
fun CallScreen(state: CommuneState) {
    val call = state.call ?: return
    var speaker by remember { mutableStateOf(false) }

    val name = call.peer.removePrefix("@").substringBefore(':')

    val showingVideo = call.remoteVideo || call.cameraOn

    // Over a picture the controls and labels need their own contrast; a
    // voice call keeps the theme's.
    val controlScrim = if (showingVideo) {
        Color.Black.copy(alpha = 0.55f)
    } else {
        MaterialTheme.colorScheme.surfaceVariant
    }
    val controlTint = if (showingVideo) Color.White else MaterialTheme.colorScheme.onSurface
    val activeTint = if (showingVideo) {
        Color(0xFF7FD1FF)
    } else {
        MaterialTheme.colorScheme.primary
    }
    val labelColor = if (showingVideo) Color.White else MaterialTheme.colorScheme.onSurface
    val subLabelColor = if (showingVideo) {
        Color.White.copy(alpha = 0.85f)
    } else {
        MaterialTheme.colorScheme.onSurfaceVariant
    }

    // A face on screen is a screen that must not go dark. Held only while
    // there are pictures: a voice call wants the display to fall asleep
    // against an ear, not stay lit under it.
    val view = androidx.compose.ui.platform.LocalView.current
    androidx.compose.runtime.DisposableEffect(showingVideo) {
        view.keepScreenOn = showingVideo
        onDispose { view.keepScreenOn = false }
    }

    androidx.compose.foundation.layout.Box(modifier = Modifier.fillMaxSize()) {
        if (showingVideo) {
            VideoSurfaces(state, call)
        }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .then(
                if (showingVideo) {
                    Modifier
                } else {
                    Modifier.background(MaterialTheme.colorScheme.surface)
                }
            )
            .padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.SpaceBetween,
    ) {
        // Stepping out of the call to read the room behind it. The call
        // carries on; the bar at the top of every page brings it back.
        //
        // Not while it is still ringing: an unanswered call is shown over
        // the lock screen, and stepping out of it there would be a way
        // into the whole application without unlocking. There is nothing
        // to step back to before answering anyway.
        Row(modifier = Modifier.fillMaxWidth()) {
            if (call.state != CommuneState.CallPhase.Ringing) {
                IconButton(onClick = { state.minimizeCall() }) {
                    Icon(
                        Icons.Filled.KeyboardArrowDown,
                        contentDescription = "Leave the call on screen",
                        tint = labelColor,
                    )
                }
            }
        }

        Column(
            modifier = Modifier.padding(top = 16.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            if (!showingVideo) {
                InitialsAvatar(identifier = call.peer, name = name, size = 112.dp)
            }
            Spacer(Modifier.height(24.dp))
            Text(
                name,
                style = MaterialTheme.typography.headlineMedium,
                textAlign = TextAlign.Center,
                color = labelColor,
            )
            Spacer(Modifier.height(8.dp))
            Text(
                when (call.state) {
                    CommuneState.CallPhase.Ringing -> "Incoming call"
                    CommuneState.CallPhase.Dialing -> "Calling…"
                    CommuneState.CallPhase.Connecting -> "Connecting…"
                    CommuneState.CallPhase.Connected -> "Connected"
                    CommuneState.CallPhase.Ended -> "Call ended"
                },
                style = MaterialTheme.typography.bodyLarge,
                color = subLabelColor,
            )
        }

        // Everything done during a call, gathered at the bottom where a
        // thumb reaches rather than adrift in the middle of the picture,
        // and carried on a scrim: over a brightly lit video the theme's
        // own on-surface colour is very nearly invisible.
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            modifier = Modifier.padding(bottom = 32.dp),
        ) {
            if (call.state != CommuneState.CallPhase.Ringing) {
                Row(
                    modifier = Modifier
                        .clip(androidx.compose.foundation.shape.RoundedCornerShape(36.dp))
                        .background(controlScrim)
                        .padding(horizontal = 8.dp, vertical = 4.dp),
                    horizontalArrangement = Arrangement.spacedBy(4.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    IconButton(onClick = { state.toggleMute() }) {
                        Icon(
                            if (call.muted) Icons.Filled.MicOff else Icons.Filled.Mic,
                            contentDescription = if (call.muted) "Unmute" else "Mute",
                            tint = controlTint,
                        )
                    }
                    IconButton(onClick = { state.toggleCamera() }) {
                        Icon(
                            if (call.cameraOn) Icons.Filled.Videocam else Icons.Filled.VideocamOff,
                            contentDescription = if (call.cameraOn) {
                                "Turn the camera off"
                            } else {
                                "Turn the camera on"
                            },
                            tint = controlTint,
                        )
                    }
                    if (call.cameraOn) {
                        IconButton(onClick = { state.switchCamera() }) {
                            Icon(
                                Icons.Filled.Cameraswitch,
                                contentDescription = "Switch camera",
                                tint = controlTint,
                            )
                        }
                    }
                    IconButton(onClick = {
                        speaker = !speaker
                        state.setSpeakerphone(speaker)
                    }) {
                        Icon(
                            Icons.Filled.VolumeUp,
                            contentDescription = "Speakerphone",
                            tint = if (speaker) activeTint else controlTint,
                        )
                    }
                }
                Spacer(Modifier.height(20.dp))
            }

            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = if (call.state == CommuneState.CallPhase.Ringing) {
                    Arrangement.SpaceEvenly
                } else {
                    Arrangement.Center
                },
                verticalAlignment = Alignment.CenterVertically,
            ) {
                if (call.state == CommuneState.CallPhase.Ringing) {
                    CallButton(
                        icon = Icons.Filled.CallEnd,
                        description = "Decline",
                        color = MaterialTheme.colorScheme.error,
                        onClick = { state.declineCall() },
                    )
                    CallButton(
                        icon = Icons.Filled.Call,
                        description = "Answer",
                        color = Color(0xFF2E7D32),
                        onClick = { state.answerCall() },
                    )
                } else {
                    CallButton(
                        icon = Icons.Filled.CallEnd,
                        description = "Hang up",
                        color = MaterialTheme.colorScheme.error,
                        onClick = { state.hangUp() },
                    )
                }
            }
        }
    }
    }
}

/// The two pictures a video call has: the far end behind everything,
/// this end in the corner.
@Composable
private fun androidx.compose.foundation.layout.BoxScope.VideoSurfaces(
    state: CommuneState,
    call: CommuneState.ActiveCall,
) {
    val context = androidx.compose.ui.platform.LocalContext.current

    if (call.remoteVideo) {
        androidx.compose.ui.viewinterop.AndroidView(
            factory = {
                org.webrtc.SurfaceViewRenderer(context).apply {
                    state.initRemoteRenderer(this)
                }
            },
            modifier = Modifier.fillMaxSize(),
        )
    }
    if (call.cameraOn) {
        androidx.compose.ui.viewinterop.AndroidView(
            factory = {
                org.webrtc.SurfaceViewRenderer(context).apply {
                    state.initLocalRenderer(this)
                }
            },
            modifier = Modifier
                .align(Alignment.TopEnd)
                .padding(16.dp)
                .size(width = 120.dp, height = 160.dp),
        )
    }
}

@Composable
private fun CallButton(
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    description: String,
    color: Color,
    onClick: () -> Unit,
) {
    IconButton(
        onClick = onClick,
        modifier = Modifier
            .size(72.dp)
            .clip(CircleShape)
            .background(color),
    ) {
        Icon(icon, contentDescription = description, tint = Color.White)
    }
}
