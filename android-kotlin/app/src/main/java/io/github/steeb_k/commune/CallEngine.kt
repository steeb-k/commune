// The media half of a call. The core speaks m.call.* and hands over
// session descriptions and candidates; everything below turns those into
// sound — WebRTC's peer connection, an audio track, and the ICE the core
// then puts on the wire. The split is the parity plan's: signalling in
// the shared core, media where the platform is.
package io.github.steeb_k.commune

import android.content.Context
import io.github.steeb_k.commune.core.FfiIceCandidate
import org.webrtc.AudioTrack
import org.webrtc.Camera2Enumerator
import org.webrtc.CameraVideoCapturer
import org.webrtc.DefaultVideoDecoderFactory
import org.webrtc.DefaultVideoEncoderFactory
import org.webrtc.EglBase
import org.webrtc.IceCandidate
import org.webrtc.MediaConstraints
import org.webrtc.MediaStream
import org.webrtc.PeerConnection
import org.webrtc.PeerConnectionFactory
import org.webrtc.RtpReceiver
import org.webrtc.SdpObserver
import org.webrtc.SessionDescription
import org.webrtc.SurfaceTextureHelper
import org.webrtc.SurfaceViewRenderer
import org.webrtc.VideoSource
import org.webrtc.VideoTrack

/// One call's media. Built when a call starts, released when it ends.
class CallEngine(
    private val context: Context,
    private val iceServers: List<PeerConnection.IceServer>,
    /// Candidates this end gathered, for the core to send.
    private val onCandidate: (FfiIceCandidate) -> Unit,
    /// Gathering finished: the core sends the end-of-candidates marker.
    private val onGatheringDone: () -> Unit,
    /// The connection came up, or fell over for good.
    private val onConnected: () -> Unit,
    private val onFailed: () -> Unit,
    /// The far end started sending pictures.
    private val onRemoteVideo: (VideoTrack) -> Unit = {},
    /// WebRTC wants a new description — the far end changed what it
    /// sends, or this end turned a camera on.
    private val onNeedsRenegotiation: () -> Unit = {},
) {
    private val eglBase: EglBase = EglBase.create()
    private var factory: PeerConnectionFactory? = null
    private var connection: PeerConnection? = null
    private var localAudio: AudioTrack? = null
    private var localVideo: VideoTrack? = null
    private var videoCapturer: CameraVideoCapturer? = null
    private var videoSource: VideoSource? = null
    private var surfaceHelper: SurfaceTextureHelper? = null
    private var remoteVideo: VideoTrack? = null

    /// The rendering context the renderers need to share.
    val eglContext: EglBase.Context get() = eglBase.eglBaseContext

    /// Candidates that arrived before the remote description did; WebRTC
    /// refuses them until it has one.
    private val pendingRemoteCandidates = mutableListOf<IceCandidate>()
    private var remoteDescriptionSet = false

    init {
        PeerConnectionFactory.initialize(
            PeerConnectionFactory.InitializationOptions.builder(context)
                .createInitializationOptions()
        )
        factory = PeerConnectionFactory.builder()
            .setVideoEncoderFactory(DefaultVideoEncoderFactory(eglBase.eglBaseContext, true, true))
            .setVideoDecoderFactory(DefaultVideoDecoderFactory(eglBase.eglBaseContext))
            .createPeerConnectionFactory()

        val config = PeerConnection.RTCConfiguration(iceServers).apply {
            sdpSemantics = PeerConnection.SdpSemantics.UNIFIED_PLAN
            continualGatheringPolicy =
                PeerConnection.ContinualGatheringPolicy.GATHER_CONTINUALLY
            bundlePolicy = PeerConnection.BundlePolicy.MAXBUNDLE
            rtcpMuxPolicy = PeerConnection.RtcpMuxPolicy.REQUIRE
        }

        connection = factory?.createPeerConnection(
            config,
            object : PeerConnection.Observer {
                override fun onIceCandidate(candidate: IceCandidate) {
                    // Both spellings: the specification asks for one and
                    // clients in the wild want the other.
                    onCandidate(
                        FfiIceCandidate(
                            candidate = candidate.sdp,
                            sdpMid = candidate.sdpMid,
                            sdpMLineIndex = candidate.sdpMLineIndex.toUInt(),
                        )
                    )
                }

                override fun onIceGatheringChange(
                    state: PeerConnection.IceGatheringState?,
                ) {
                    if (state == PeerConnection.IceGatheringState.COMPLETE) {
                        onGatheringDone()
                    }
                }

                override fun onConnectionChange(state: PeerConnection.PeerConnectionState?) {
                    when (state) {
                        PeerConnection.PeerConnectionState.CONNECTED -> onConnected()
                        PeerConnection.PeerConnectionState.FAILED -> onFailed()
                        else -> {}
                    }
                }

                override fun onIceConnectionChange(
                    state: PeerConnection.IceConnectionState?,
                ) {
                    when (state) {
                        PeerConnection.IceConnectionState.CONNECTED,
                        PeerConnection.IceConnectionState.COMPLETED -> onConnected()
                        PeerConnection.IceConnectionState.FAILED -> onFailed()
                        else -> {}
                    }
                }

                override fun onSignalingChange(state: PeerConnection.SignalingState?) {}
                override fun onIceConnectionReceivingChange(receiving: Boolean) {}
                override fun onIceCandidatesRemoved(candidates: Array<out IceCandidate>?) {}
                override fun onAddStream(stream: MediaStream?) {}
                override fun onRemoveStream(stream: MediaStream?) {}
                override fun onDataChannel(channel: org.webrtc.DataChannel?) {}
                override fun onRenegotiationNeeded() {
                    if (renegotiationArmed) onNeedsRenegotiation()
                }
                override fun onAddTrack(
                    receiver: RtpReceiver?,
                    streams: Array<out MediaStream>?,
                ) {
                    val track = receiver?.track()
                    if (track is VideoTrack) {
                        remoteVideo = track
                        onRemoteVideo(track)
                    }
                }
            },
        )

        // The voice this end contributes.
        val source = factory?.createAudioSource(MediaConstraints())
        localAudio = factory?.createAudioTrack("commune-audio", source)
        localAudio?.let { connection?.addTrack(it, listOf("commune-stream")) }
    }

    /// Until the first description is exchanged, WebRTC's request for
    /// a negotiation is the call itself, not a renegotiation.
    private var renegotiationArmed = false

    fun armRenegotiation() {
        renegotiationArmed = true
    }

    /// Turn the camera on or off mid-call. Adding the track makes
    /// WebRTC ask for a renegotiation, which becomes an
    /// `m.call.negotiate`.
    fun setCameraEnabled(enabled: Boolean, front: Boolean = true): Boolean {
        if (!enabled) {
            localVideo?.setEnabled(false)
            try {
                videoCapturer?.stopCapture()
            } catch (_: InterruptedException) {
            }
            return true
        }

        if (localVideo != null) {
            localVideo?.setEnabled(true)
            startCapture()
            return true
        }

        val factory = factory ?: return false
        val enumerator = Camera2Enumerator(context)
        val name = enumerator.deviceNames.firstOrNull { device ->
            if (front) enumerator.isFrontFacing(device) else enumerator.isBackFacing(device)
        } ?: enumerator.deviceNames.firstOrNull() ?: return false

        val capturer = enumerator.createCapturer(name, null) ?: return false
        val helper = SurfaceTextureHelper.create("commune-capture", eglBase.eglBaseContext)
        val source = factory.createVideoSource(false)
        capturer.initialize(helper, context, source.capturerObserver)

        val track = factory.createVideoTrack("commune-video", source)
        connection?.addTrack(track, listOf("commune-stream"))

        videoCapturer = capturer
        surfaceHelper = helper
        videoSource = source
        localVideo = track
        startCapture()
        return true
    }

    private fun startCapture() {
        try {
            videoCapturer?.startCapture(1280, 720, 30)
        } catch (_: Exception) {
            // A camera that will not start is a call without pictures.
        }
    }

    fun switchCamera() {
        videoCapturer?.switchCamera(null)
    }

    /// Show this end's own picture in the given renderer.
    fun attachLocalVideo(renderer: SurfaceViewRenderer) {
        localVideo?.addSink(renderer)
    }

    /// Show the far end's picture.
    fun attachRemoteVideo(renderer: SurfaceViewRenderer) {
        remoteVideo?.addSink(renderer)
    }

    val hasRemoteVideo: Boolean get() = remoteVideo != null
    val hasLocalVideo: Boolean get() = localVideo?.enabled() == true

    /// Build an offer for a renegotiation mid-call.
    fun createRenegotiationOffer(onSdp: (String) -> Unit) {
        val connection = connection ?: return
        connection.createOffer(
            object : SimpleSdpObserver() {
                override fun onCreateSuccess(description: SessionDescription) {
                    connection.setLocalDescription(SimpleSdpObserver(), description)
                    onSdp(description.description)
                }
            },
            MediaConstraints(),
        )
    }

    /// Apply a renegotiation the far end offered, and answer it.
    fun acceptRenegotiation(
        sdp: String,
        sessionType: String,
        onAnswer: (String) -> Unit,
    ) {
        val connection = connection ?: return
        val type = if (sessionType.equals("answer", ignoreCase = true)) {
            SessionDescription.Type.ANSWER
        } else {
            SessionDescription.Type.OFFER
        }
        connection.setRemoteDescription(
            object : SimpleSdpObserver() {
                override fun onSetSuccess() {
                    flushRemoteCandidates()
                    // Only an offer wants an answer back.
                    if (type != SessionDescription.Type.OFFER) return
                    connection.createAnswer(
                        object : SimpleSdpObserver() {
                            override fun onCreateSuccess(description: SessionDescription) {
                                connection.setLocalDescription(
                                    SimpleSdpObserver(),
                                    description,
                                )
                                onAnswer(description.description)
                            }
                        },
                        MediaConstraints(),
                    )
                }
            },
            SessionDescription(type, sdp),
        )
    }

    /// The stream this end describes in its metadata.
    val localStreamId: String get() = "commune-stream"

    /// Build the offer that starts an outgoing call.
    fun createOffer(onSdp: (String) -> Unit, onError: (String) -> Unit) {
        val connection = connection ?: return onError("No connection")
        connection.createOffer(
            object : SimpleSdpObserver() {
                override fun onCreateSuccess(description: SessionDescription) {
                    connection.setLocalDescription(SimpleSdpObserver(), description)
                    onSdp(description.description)
                }

                override fun onCreateFailure(error: String?) {
                    onError(error ?: "Could not start the call")
                }
            },
            MediaConstraints(),
        )
    }

    /// Take the other end's offer and answer it.
    fun acceptOffer(sdp: String, onSdp: (String) -> Unit, onError: (String) -> Unit) {
        val connection = connection ?: return onError("No connection")
        connection.setRemoteDescription(
            object : SimpleSdpObserver() {
                override fun onSetSuccess() {
                    flushRemoteCandidates()
                    connection.createAnswer(
                        object : SimpleSdpObserver() {
                            override fun onCreateSuccess(description: SessionDescription) {
                                connection.setLocalDescription(SimpleSdpObserver(), description)
                                onSdp(description.description)
                            }

                            override fun onCreateFailure(error: String?) {
                                onError(error ?: "Could not answer the call")
                            }
                        },
                        MediaConstraints(),
                    )
                }

                override fun onSetFailure(error: String?) {
                    onError(error ?: "Could not read the call offer")
                }
            },
            SessionDescription(SessionDescription.Type.OFFER, sdp),
        )
    }

    /// Apply the answer to a call this end placed.
    fun acceptAnswer(sdp: String) {
        connection?.setRemoteDescription(
            object : SimpleSdpObserver() {
                override fun onSetSuccess() = flushRemoteCandidates()
            },
            SessionDescription(SessionDescription.Type.ANSWER, sdp),
        )
    }

    /// Candidates from the other end.
    fun addRemoteCandidates(candidates: List<FfiIceCandidate>) {
        for (candidate in candidates) {
            val ice = IceCandidate(
                candidate.sdpMid,
                candidate.sdpMLineIndex.toInt(),
                candidate.candidate,
            )
            // Before a remote description exists these are refused, so
            // they wait for it.
            if (remoteDescriptionSet) {
                connection?.addIceCandidate(ice)
            } else {
                synchronized(pendingRemoteCandidates) { pendingRemoteCandidates.add(ice) }
            }
        }
    }

    private fun flushRemoteCandidates() {
        remoteDescriptionSet = true
        synchronized(pendingRemoteCandidates) {
            for (candidate in pendingRemoteCandidates) {
                connection?.addIceCandidate(candidate)
            }
            pendingRemoteCandidates.clear()
        }
    }

    fun setMicrophoneEnabled(enabled: Boolean) {
        localAudio?.setEnabled(enabled)
    }

    fun release() {
        try {
            videoCapturer?.stopCapture()
            videoCapturer?.dispose()
            surfaceHelper?.dispose()
            videoSource?.dispose()
            connection?.dispose()
            factory?.dispose()
            eglBase.release()
        } catch (_: Exception) {
            // A call that is already gone needs no tidying.
        }
        connection = null
        factory = null
    }
}

/// The observer methods a given call does not care about.
open class SimpleSdpObserver : SdpObserver {
    override fun onCreateSuccess(description: SessionDescription) {}
    override fun onSetSuccess() {}
    override fun onCreateFailure(error: String?) {}
    override fun onSetFailure(error: String?) {}
}
