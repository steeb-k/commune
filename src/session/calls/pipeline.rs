//! The `GStreamer` side of a call.
//!
//! This is where WebRTC actually happens. Everything above it deals in Matrix
//! events; everything here deals in SDP, ICE candidates and media.

use std::{
    collections::HashSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
};

use futures_channel::mpsc;
use gst::prelude::*;
use gtk::{gdk, glib};
use tracing::{debug, error, warn};

use super::turn::IceServers;

/// Remote ICE candidate lines, waiting for somewhere to go.
type PendingCandidates = Arc<Mutex<Vec<String>>>;

/// The transport addresses of the remote candidates already handed over.
type RemoteAddresses = Arc<Mutex<HashSet<String>>>;

/// An error that stops a call pipeline from being built or driven.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PipelineError {
    /// An element could not be made, added or linked.
    ///
    /// Almost always a `GStreamer` plugin that is not installed.
    #[error("{0}")]
    Element(#[from] glib::BoolError),
    /// The pipeline would not change state.
    #[error("{0}")]
    StateChange(#[from] gst::StateChangeError),
    /// Two pads would not link.
    #[error("could not link {0}")]
    Link(&'static str),
    /// Something else that ends the call.
    #[error("{0}")]
    Other(&'static str),
}

/// The audio payload type we offer, from the static range.
const OPUS_PAYLOAD_TYPE: i32 = 111;
/// The video payload type we offer, from the dynamic range.
const VP8_PAYLOAD_TYPE: i32 = 96;

/// What the pipeline has to say to the call that owns it.
#[derive(Debug)]
pub(crate) enum PipelineEvent {
    /// A local session description is ready to be sent.
    LocalDescription {
        /// The SDP text.
        sdp: String,
        /// Whether this is an answer rather than an offer.
        is_answer: bool,
    },
    /// An ICE candidate was gathered.
    IceCandidate {
        /// The candidate line, without the `a=` prefix.
        candidate: String,
        /// The index of the media section it belongs to.
        sdp_m_line_index: u32,
        /// The `a=mid:` of that section, when the local description has one.
        ///
        /// The spec asks for one of the two and every other client sends
        /// both. A candidate with only an index is one libwebrtc is handed as
        /// `IceCandidate(null, index, line)`, and a null mid is how a
        /// candidate gets dropped by the far end without a word.
        sdp_mid: Option<String>,
    },
    /// No more ICE candidates will be gathered.
    IceGatheringDone,
    /// The session has to be negotiated again.
    ///
    /// `webrtcbin` says this whenever what the pipeline sends stops matching
    /// what the last offer described — adding the camera to a call that was
    /// placed without it, most of all. It is only acted on once the call is
    /// up: the same signal fires for the first negotiation of every call,
    /// which is already driven by hand.
    NegotiationNeeded,
    /// The other party's video is being drawn.
    ///
    /// The remote paintable exists from the moment the pipeline does, so it is
    /// not a sign that any video is coming. This is: a decoded video stream
    /// has been attached to the sink, and frames are on their way.
    RemoteVideo,
    /// Media is flowing.
    Connected,
    /// The connection failed, either while establishing it or after.
    ConnectionFailed,
    /// Something went wrong that ends the call.
    Error(String),
}

/// Which kinds of media a call carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MediaKind {
    Audio,
    Video,
}

impl MediaKind {
    /// Parse an SDP media section's type.
    fn from_sdp_media(media: &str) -> Option<Self> {
        match media {
            "audio" => Some(Self::Audio),
            "video" => Some(Self::Video),
            _ => None,
        }
    }
}

/// The WebRTC pipeline of a single call.
#[derive(Debug)]
pub(crate) struct CallPipeline {
    pipeline: gst::Pipeline,
    webrtcbin: gst::Element,
    /// The switch on our own microphone.
    mic_volume: Option<gst::Element>,
    /// The switch on our own camera.
    camera_valve: Option<gst::Element>,
    /// Where our own camera is drawn, for the self-view.
    local_paintable: Option<gdk::Paintable>,
    /// Where the other party is drawn.
    remote_paintable: gdk::Paintable,
    /// The media sections we have set up, in the order of their m-lines.
    media: Vec<MediaKind>,
    /// Whether media has ever flowed on this connection.
    had_media: bool,
    /// The bus watch, dropped with the pipeline.
    bus_guard: Option<gst::bus::BusWatchGuard>,
    /// The `ice-ufrag` of the remote description, once there is one.
    ///
    /// A candidate names the ufrag it belongs to, and one that does not match
    /// is from a different ICE session — a second device that rang and did not
    /// answer, most often.
    remote_ice_ufrag: Arc<Mutex<Option<String>>>,
    /// Whether the other party's description has been applied.
    ///
    /// Remote ICE candidates are meaningless before it: there is no remote ICE
    /// agent to give them to, and `webrtcbin` discards them.
    remote_description_applied: Arc<AtomicBool>,
    /// Remote candidates that arrived before that happened.
    pending_remote_candidates: PendingCandidates,
    /// The transport addresses of the remote candidates already added.
    ///
    /// Two candidates for one address are two pairs to one place, and libnice
    /// cannot hold them apart: it resolves a check response to a remote
    /// candidate *by address*, so the response to the second pair's check
    /// lands on the first pair and takes it out of `SUCCEEDED`. The next
    /// nomination tick asserts on that and the process dies.
    remote_addresses: RemoteAddresses,
    /// The `a=mid:` of each section of our own description, in m-line order.
    local_mids: Arc<Mutex<Vec<String>>>,
    /// The m-line index the other party's transport is on.
    ///
    /// Zero until their description says otherwise, which is the answer for
    /// every peer that accepts the first section we offer.
    remote_transport_index: Arc<AtomicU32>,
    /// Whether `on-negotiation-needed` is to be listened to.
    ///
    /// `webrtcbin` emits it for the first negotiation as well, which this
    /// client drives itself; acting on that one would put a second offer on
    /// the wire for every call placed.
    renegotiation_armed: Arc<AtomicBool>,
}

impl Drop for CallPipeline {
    fn drop(&mut self) {
        // A pipeline left in `Playing` keeps the microphone and the camera
        // open, and neither the compositor nor the user is told why.
        if let Err(error) = self.pipeline.set_state(gst::State::Null) {
            warn!("Could not stop the call pipeline: {error}");
        }
    }
}

impl CallPipeline {
    /// Build a pipeline for a call we are placing.
    ///
    /// The media sections are ours to choose, so they are audio first and then
    /// video, which is the order every other client offers and the order this
    /// one expects to see in an answer.
    pub(crate) fn new_for_offer(
        with_video: bool,
        servers: &IceServers,
    ) -> Result<(Self, mpsc::UnboundedReceiver<PipelineEvent>), PipelineError> {
        let media = if with_video {
            vec![MediaKind::Audio, MediaKind::Video]
        } else {
            vec![MediaKind::Audio]
        };

        Self::build(media, servers)
    }

    /// Build a pipeline for a call we are answering.
    ///
    /// The media sections have to line up with the offer, section for section,
    /// or `webrtcbin` refuses the answer. So the offer decides, not us: what we
    /// can source we send, and what we cannot we still make room for.
    pub(crate) fn new_for_answer(
        offer_sdp: &str,
        servers: &IceServers,
    ) -> Result<(Self, mpsc::UnboundedReceiver<PipelineEvent>), PipelineError> {
        let media = media_sections(offer_sdp);
        Self::build(media, servers)
    }

    fn build(
        media: Vec<MediaKind>,
        servers: &IceServers,
    ) -> Result<(Self, mpsc::UnboundedReceiver<PipelineEvent>), PipelineError> {
        if media.is_empty() {
            return Err(PipelineError::Other(
                "the offer has no audio or video we can carry",
            ));
        }

        let pipeline = gst::Pipeline::new();

        let webrtcbin = gst::ElementFactory::make("webrtcbin")
            .name("webrtc")
            // Everything on one transport, which is what every browser and
            // every other Matrix client negotiates.
            .property_from_str("bundle-policy", "max-bundle")
            .build()?;
        pipeline.add(&webrtcbin)?;

        // No STUN server of our own choosing. The usual one is Google's, and
        // pointing every call at a third party to learn our own address tells
        // that third party that a call is happening at all. One the
        // homeserver's operator named is their decision, and using it is the
        // only way a client whose TURN server sits inside the same NAT ever
        // learns an address the other end can reach it on.
        if let Some(stun) = &servers.stun {
            debug!("Using the STUN server the homeserver named: {stun}");
            webrtcbin.set_property("stun-server", stun);
        }

        if servers.is_empty() {
            // Host candidates only. Two people on one network still find each
            // other; anybody behind a NAT does not.
            warn!("No STUN or TURN server for this call; only host candidates will be gathered");
        }

        for server in &servers.turn {
            let added = webrtcbin.emit_by_name::<bool>("add-turn-server", &[&server.uri]);

            // The credentials are in the URI, so only the transport is
            // logged — and it is the part that matters, since the first server
            // accepted is the one a relay candidate comes from.
            if added {
                debug!("webrtcbin accepted a TURN server over {}", server.transport);
            } else {
                warn!("webrtcbin refused a TURN server over {}", server.transport);
            }
        }

        let remote_video_sink = gst::ElementFactory::make("gtk4paintablesink").build()?;
        let remote_paintable = remote_video_sink.property::<gdk::Paintable>("paintable");

        let mut this = Self {
            pipeline,
            webrtcbin,
            mic_volume: None,
            camera_valve: None,
            local_paintable: None,
            remote_paintable,
            media: Vec::new(),
            had_media: false,
            bus_guard: None,
            remote_ice_ufrag: Arc::new(Mutex::new(None)),
            remote_description_applied: Arc::new(AtomicBool::new(false)),
            pending_remote_candidates: Arc::new(Mutex::new(Vec::new())),
            remote_transport_index: Arc::new(AtomicU32::new(0)),
            remote_addresses: Arc::new(Mutex::new(HashSet::new())),
            local_mids: Arc::new(Mutex::new(Vec::new())),
            renegotiation_armed: Arc::new(AtomicBool::new(false)),
        };

        for kind in media {
            match kind {
                MediaKind::Audio => this.add_audio_source()?,
                MediaKind::Video => this.add_video_source()?,
            }
            this.media.push(kind);
        }

        let (sender, receiver) = mpsc::unbounded();
        this.watch(sender, remote_video_sink)?;

        Ok((this, receiver))
    }

    /// Whether this pipeline sends and expects video.
    pub(crate) fn has_video(&self) -> bool {
        self.media.contains(&MediaKind::Video)
    }

    /// Where the other party is drawn.
    pub(crate) fn remote_paintable(&self) -> &gdk::Paintable {
        &self.remote_paintable
    }

    /// Where our own camera is drawn, if there is one.
    pub(crate) fn local_paintable(&self) -> Option<&gdk::Paintable> {
        self.local_paintable.as_ref()
    }

    /// Listen to `webrtcbin` asking for a negotiation.
    ///
    /// Called once the first one is done. Before that the signal fires for the
    /// offer this client is already making by hand, and honouring it would put
    /// a second offer on the wire for every call.
    pub(crate) fn arm_renegotiation(&self) {
        self.renegotiation_armed.store(true, Ordering::Relaxed);
    }

    /// Add the camera to a call that was placed without one.
    ///
    /// The new source is linked to a new sink pad, which is a new transceiver,
    /// which is what makes `webrtcbin` ask for a negotiation — and that ask is
    /// how the offer gets made. Nothing here writes SDP.
    pub(crate) fn enable_video(&mut self) -> Result<(), PipelineError> {
        if self.has_video() {
            return Ok(());
        }

        // [`Self::add_video_source`] answers a missing camera with a
        // receive-only section, which is right while the sections are still
        // being chosen and wrong here: the other end was never asked for
        // video, so a section to receive it in carries nothing.
        gst::ElementFactory::make("autovideosrc")
            .build()
            .map_err(|_| PipelineError::Other("there is no camera to add"))?;

        self.add_video_source()?;
        self.media.push(MediaKind::Video);

        // Everything added to a pipeline arrives in `Null`, whatever the
        // pipeline itself is doing, and a source in `Null` produces nothing.
        let mut elements = self.pipeline.iterate_elements();
        while let Ok(Some(element)) = elements.next() {
            element.sync_state_with_parent()?;
        }

        debug!("The camera is in the pipeline; waiting for webrtcbin to ask for an offer");

        Ok(())
    }

    /// Undo a local offer that has not been answered.
    ///
    /// Perfect negotiation's move for the polite party: an offer of ours and
    /// one of theirs crossed, and ours is the one that gives way. `webrtcbin`
    /// is asked to roll back to `stable` so that theirs can be applied.
    ///
    /// Untested against a peer, because it takes two clients renegotiating in
    /// the same second to reach it.
    pub(crate) fn rollback_local_description(&self) {
        let Ok(message) = gst_sdp::SDPMessage::parse_buffer(b"") else {
            return;
        };
        let rollback =
            gst_webrtc::WebRTCSessionDescription::new(gst_webrtc::WebRTCSDPType::Rollback, message);

        debug!("Rolling back our own offer in favour of theirs");
        let promise = gst::Promise::with_change_func(|reply| {
            if let Err(error) = reply {
                warn!("webrtcbin would not roll back our offer: {error:?}");
            }
        });

        self.webrtcbin
            .emit_by_name::<()>("set-local-description", &[&rollback, &promise]);
    }

    /// Add the microphone to the pipeline, encoded and packetised for the wire.
    fn add_audio_source(&mut self) -> Result<(), PipelineError> {
        let source = gst::ElementFactory::make("autoaudiosrc").build()?;
        let queue = leaky_queue()?;
        let convert = gst::ElementFactory::make("audioconvert").build()?;
        let resample = gst::ElementFactory::make("audioresample").build()?;
        let volume = gst::ElementFactory::make("volume").build()?;
        // Two channels, and not because the microphone has two.
        //
        // "The RTP clock rate ... MUST be 48000, and the number of channels
        // MUST be 2" — RFC 7587 §7, and libwebrtc holds callers to it. A
        // `webrtcbin` fed mono audio writes `a=rtpmap:111 OPUS/48000`, which
        // names a codec libwebrtc does not have, and it answers by rejecting
        // the whole section.
        //
        // That is what every call to Element for Android did. Measured on 23
        // August 2026: a voice call came back `m=audio 0` with `a=group:BUNDLE`
        // empty — nothing accepted, no transport, and no candidates ever sent,
        // so the call sat in `Connecting` until somebody gave up. A video call
        // came back with the audio rejected and the video kept, and then failed
        // for the reasons below.
        let stereo = gst::ElementFactory::make("capsfilter")
            .property(
                "caps",
                gst::Caps::builder("audio/x-raw")
                    .field("rate", 48_000i32)
                    .field("channels", 2i32)
                    .build(),
            )
            .build()?;
        let encoder = gst::ElementFactory::make("opusenc").build()?;
        let payloader = gst::ElementFactory::make("rtpopuspay")
            .property("pt", u32::try_from(OPUS_PAYLOAD_TYPE).unwrap_or_default())
            .build()?;
        let caps = gst::ElementFactory::make("capsfilter")
            .property(
                "caps",
                gst::Caps::builder("application/x-rtp")
                    .field("media", "audio")
                    .field("encoding-name", "OPUS")
                    .field("payload", OPUS_PAYLOAD_TYPE)
                    .field("clock-rate", 48_000i32)
                    // What puts the `/2` in the rtpmap line.
                    .field("encoding-params", "2")
                    .build(),
            )
            .build()?;

        let elements = [
            &source, &queue, &convert, &resample, &volume, &stereo, &encoder, &payloader, &caps,
        ];
        self.pipeline.add_many(elements)?;
        gst::Element::link_many(elements)?;

        self.link_to_webrtcbin(&caps)?;
        self.mic_volume = Some(volume);

        Ok(())
    }

    /// Add the camera to the pipeline, with a branch for our own preview.
    ///
    /// A missing camera is not fatal. The section still has to exist, or the
    /// answer would not line up with the offer, so it becomes a receive-only
    /// transceiver and the call carries the other party's video and not ours.
    fn add_video_source(&mut self) -> Result<(), PipelineError> {
        let Ok(source) = gst::ElementFactory::make("autovideosrc").build() else {
            warn!("No camera available; the video section will be receive-only");
            self.add_recvonly_video();
            return Ok(());
        };

        let valve = gst::ElementFactory::make("valve").build()?;
        let tee = gst::ElementFactory::make("tee").build()?;

        // Leaky for the same reason the send queue is, and one more: this
        // queue sits before `videoconvert`, so what it holds are the camera's
        // own buffers. A camera hands out a small fixed pool of them — four or
        // eight — and a queue that holds even a few and does not give them back
        // starves the source. `videotestsrc` allocates fresh buffers every
        // time and cannot reproduce that, which is why the measurement that
        // cleared the rest of this topology could not clear this.
        let preview_queue = leaky_queue()?;
        let preview_convert = gst::ElementFactory::make("videoconvert").build()?;
        // Ask the self-view for a plain system-memory format, by name.
        //
        // `gtk4paintablesink` offers `video/x-raw(memory:GLMemory)` ahead of
        // everything else, and `videoconvert` passes a memory feature it does
        // not recognise straight through rather than refusing it — so left
        // alone, the whole branch negotiates GL textures and the camera is
        // asked to produce them. On macOS `avfvideosrc` accepts that and then
        // dies on the first buffer, and what reaches the bus is
        // `Internal data stream error … reason error (-5)` attributed to the
        // source. It is not a negotiation error, so nothing in it points at
        // the sink that asked for GL.
        //
        // Naming a format is what settles it: `videoconvert` then has to
        // convert rather than pass through, and the camera is left on the
        // plain `video/x-raw` it is happy with (UYVY here, converted to RGBA
        // for the sink). A bare `video/x-raw` caps does **not** do it — that
        // was tried, and the GL feature still won.
        //
        // The remote video sink needs none of this: it is fed by a decoder
        // that only ever produces system memory, so there is no GL path for
        // the negotiation to prefer.
        let preview_caps = gst::ElementFactory::make("capsfilter")
            .property(
                "caps",
                gst::Caps::builder("video/x-raw")
                    .field("format", "RGBA")
                    .build(),
            )
            .build()?;
        let preview_sink = gst::ElementFactory::make("gtk4paintablesink").build()?;
        self.local_paintable = Some(preview_sink.property::<gdk::Paintable>("paintable"));

        let send_queue = leaky_queue()?;
        let convert = gst::ElementFactory::make("videoconvert").build()?;
        let scale = gst::ElementFactory::make("videoscale").build()?;
        let scaled = gst::ElementFactory::make("capsfilter")
            .property(
                "caps",
                gst::Caps::builder("video/x-raw")
                    .field("width", 640i32)
                    .field("height", 480i32)
                    .build(),
            )
            .build()?;
        let encoder = gst::ElementFactory::make("vp8enc")
            // Real time, not "as good as it can get whenever it gets there".
            .property("deadline", 1i64)
            // A receiver joining late, or one that lost a frame, needs a
            // keyframe to draw anything at all.
            .property("keyframe-max-dist", 30i32)
            .build()?;
        let payloader = gst::ElementFactory::make("rtpvp8pay")
            .property("pt", u32::try_from(VP8_PAYLOAD_TYPE).unwrap_or_default())
            .build()?;
        let caps = gst::ElementFactory::make("capsfilter")
            .property(
                "caps",
                gst::Caps::builder("application/x-rtp")
                    .field("media", "video")
                    .field("encoding-name", "VP8")
                    .field("payload", VP8_PAYLOAD_TYPE)
                    .field("clock-rate", 90_000i32)
                    .build(),
            )
            .build()?;

        self.pipeline.add_many([
            &source,
            &valve,
            &tee,
            &preview_queue,
            &preview_convert,
            &preview_caps,
            &preview_sink,
            &send_queue,
            &convert,
            &scale,
            &scaled,
            &encoder,
            &payloader,
            &caps,
        ])?;

        gst::Element::link_many([&source, &valve, &tee])?;
        gst::Element::link_many([
            &tee,
            &preview_queue,
            &preview_convert,
            &preview_caps,
            &preview_sink,
        ])?;
        gst::Element::link_many([
            &tee,
            &send_queue,
            &convert,
            &scale,
            &scaled,
            &encoder,
            &payloader,
            &caps,
        ])?;

        self.link_to_webrtcbin(&caps)?;
        self.camera_valve = Some(valve);

        Ok(())
    }

    /// Make room for a video section we cannot fill.
    fn add_recvonly_video(&mut self) {
        let caps = gst::Caps::builder("application/x-rtp")
            .field("media", "video")
            .field("encoding-name", "VP8")
            .field("payload", VP8_PAYLOAD_TYPE)
            .field("clock-rate", 90_000i32)
            .build();

        self.webrtcbin
            .emit_by_name::<gst_webrtc::WebRTCRTPTransceiver>(
                "add-transceiver",
                &[&gst_webrtc::WebRTCRTPTransceiverDirection::Recvonly, &caps],
            );
    }

    /// Link a packetised source to the next media section of `webrtcbin`.
    ///
    /// The sink pads are numbered in the order they are requested, and that
    /// number is the m-line index. This is the only thing keeping our sections
    /// in step with the other party's.
    fn link_to_webrtcbin(&self, source: &gst::Element) -> Result<(), PipelineError> {
        let Some(sink_pad) = self.webrtcbin.request_pad_simple("sink_%u") else {
            return Err(PipelineError::Other(
                "webrtcbin would not give us a sink pad",
            ));
        };
        let Some(src_pad) = source.static_pad("src") else {
            return Err(PipelineError::Other("the source has no src pad"));
        };

        src_pad
            .link(&sink_pad)
            .map_err(|_| PipelineError::Link("a source into webrtcbin"))?;

        Ok(())
    }

    /// Wire up everything the pipeline has to tell us.
    fn watch(
        &mut self,
        sender: mpsc::UnboundedSender<PipelineEvent>,
        remote_video_sink: gst::Element,
    ) -> Result<(), PipelineError> {
        let ice_sender = sender.clone();
        let ice_mids = self.local_mids.clone();
        self.webrtcbin
            .connect("on-ice-candidate", false, move |values| {
                let sdp_m_line_index = values.get(1)?.get::<u32>().ok()?;
                let candidate = values.get(2)?.get::<String>().ok()?;

                let sdp_mid = ice_mids
                    .lock()
                    .ok()
                    .and_then(|mids| mids.get(sdp_m_line_index as usize).cloned());

                // The type is what says whether the TURN server actually gave
                // us anything: `host` is our own address, `srflx` is what the
                // server says our address looks like from outside, and `relay`
                // is an allocation on the server itself. No `relay` among them
                // means the relay was never obtained, whatever the URI said.
                // The whole line, so that local and remote candidates can be
                // compared side by side: which pairs exist at all is decided by
                // their addresses and families, and by the `ufrag` they carry.
                debug!(
                    "webrtcbin gathered a {} candidate on m-line {sdp_m_line_index}: {candidate}",
                    candidate_type(&candidate)
                );

                let _ = ice_sender.unbounded_send(PipelineEvent::IceCandidate {
                    candidate,
                    sdp_m_line_index,
                    sdp_mid,
                });

                None
            });

        // Renegotiation, and only renegotiation: the first offer of a call is
        // made by hand, and `webrtcbin` asks for that one too.
        let negotiation_sender = sender.clone();
        let armed = self.renegotiation_armed.clone();
        self.webrtcbin
            .connect("on-negotiation-needed", false, move |_| {
                if !armed.load(Ordering::Relaxed) {
                    debug!("webrtcbin wants a negotiation; this one is ours to make");
                    return None;
                }

                debug!("webrtcbin says the session has to be negotiated again");
                let _ = negotiation_sender.unbounded_send(PipelineEvent::NegotiationNeeded);

                None
            });

        self.watch_states(&sender);

        // Incoming media arrives as a new pad per stream, still packetised.
        let pipeline = self.pipeline.downgrade();
        let error_sender = sender.clone();
        let media_sender = sender.clone();
        self.webrtcbin.connect_pad_added(move |_, pad| {
            if pad.direction() != gst::PadDirection::Src {
                return;
            }
            let Some(pipeline) = pipeline.upgrade() else {
                return;
            };

            if let Err(error) = attach_receiver(&pipeline, pad, &remote_video_sink, &media_sender) {
                error!("Could not play an incoming stream: {error}");
                let _ = error_sender.unbounded_send(PipelineEvent::Error(error.to_string()));
            }
        });

        self.watch_bus(sender)
    }

    /// Follow the four state machines `webrtcbin` keeps.
    fn watch_states(&self, sender: &mpsc::UnboundedSender<PipelineEvent>) {
        let gathering_sender = sender.clone();
        self.webrtcbin
            .connect_notify(Some("ice-gathering-state"), move |webrtcbin, _| {
                let state = webrtcbin
                    .property::<gst_webrtc::WebRTCICEGatheringState>("ice-gathering-state");
                debug!("ICE gathering state is now {state:?}");

                if state == gst_webrtc::WebRTCICEGatheringState::Complete {
                    let _ = gathering_sender.unbounded_send(PipelineEvent::IceGatheringDone);
                }
            });

        let connection_sender = sender.clone();
        self.webrtcbin
            .connect_notify(Some("connection-state"), move |webrtcbin, _| {
                let state =
                    webrtcbin.property::<gst_webrtc::WebRTCPeerConnectionState>("connection-state");
                debug!("Peer connection state is now {state:?}");

                match state {
                    gst_webrtc::WebRTCPeerConnectionState::Connected => {
                        let _ = connection_sender.unbounded_send(PipelineEvent::Connected);
                    }
                    gst_webrtc::WebRTCPeerConnectionState::Failed => {
                        let _ = connection_sender.unbounded_send(PipelineEvent::ConnectionFailed);
                    }
                    _ => {}
                }
            });

        // The ICE connection state, watched alongside the aggregate one above.
        //
        // `connection-state` is the aggregate `RTCPeerConnectionState`, and it
        // moves only once every transport underneath it has reported in — ICE
        // and DTLS both. `ice-connection-state` moves as soon as a candidate
        // pair starts carrying traffic. Watching only the aggregate is how a
        // call with a working path sits on "Connecting…" indefinitely.
        let ice_connection_sender = sender.clone();
        self.webrtcbin
            .connect_notify(Some("ice-connection-state"), move |webrtcbin, _| {
                let state = webrtcbin
                    .property::<gst_webrtc::WebRTCICEConnectionState>("ice-connection-state");
                debug!("ICE connection state is now {state:?}");

                match state {
                    // `Completed` is `Connected` plus "and nothing better is
                    // coming"; both mean a pair is carrying traffic.
                    gst_webrtc::WebRTCICEConnectionState::Connected
                    | gst_webrtc::WebRTCICEConnectionState::Completed => {
                        let _ = ice_connection_sender.unbounded_send(PipelineEvent::Connected);
                    }
                    gst_webrtc::WebRTCICEConnectionState::Failed => {
                        let _ =
                            ice_connection_sender.unbounded_send(PipelineEvent::ConnectionFailed);
                    }
                    // `Disconnected` is usually transient — a few lost packets
                    // — and ICE recovers from it on its own.
                    _ => {}
                }
            });

        self.webrtcbin
            .connect_notify(Some("signaling-state"), |webrtcbin, _| {
                let state =
                    webrtcbin.property::<gst_webrtc::WebRTCSignalingState>("signaling-state");
                debug!("Signalling state is now {state:?}");
            });
    }

    /// Report what the pipeline's bus says went wrong.
    fn watch_bus(
        &mut self,
        sender: mpsc::UnboundedSender<PipelineEvent>,
    ) -> Result<(), PipelineError> {
        let bus = self.pipeline.bus().expect("a pipeline always has a bus");
        let bus_sender = sender;
        let guard = bus
            .add_watch_local(move |_, message| {
                if let gst::MessageView::Error(error) = message.view() {
                    // The debug string is where the reason is. `GStreamer`
                    // reports a failed negotiation as "Internal data stream
                    // error" from whichever source starved, and only the debug
                    // field says `not-negotiated`; without it the element that
                    // is named is never the element that is wrong.
                    error!(
                        "Error from {}: {} ({})",
                        error.src().map_or_else(
                            || "the call pipeline".to_owned(),
                            |src| src.path_string().to_string()
                        ),
                        error.error(),
                        error.debug().as_deref().unwrap_or("no debug information")
                    );
                    let _ =
                        bus_sender.unbounded_send(PipelineEvent::Error(error.error().to_string()));
                }

                glib::ControlFlow::Continue
            })
            .map_err(|_| PipelineError::Other("could not watch the pipeline bus"))?;
        self.bus_guard = Some(guard);

        Ok(())
    }

    /// Start the pipeline.
    pub(crate) fn start(&self) -> Result<(), PipelineError> {
        self.pipeline.set_state(gst::State::Playing)?;
        Ok(())
    }

    /// Note that media has been seen, so that a later failure is a timeout
    /// rather than a failure to connect at all.
    pub(crate) fn note_media(&mut self) {
        self.had_media = true;
    }

    /// Whether media has ever flowed.
    pub(crate) fn had_media(&self) -> bool {
        self.had_media
    }

    /// Create the offer, and set it as our local description.
    pub(crate) fn create_offer(&self, sender: mpsc::UnboundedSender<PipelineEvent>) {
        self.create_description("create-offer", false, sender);
    }

    /// Create the answer, and set it as our local description.
    pub(crate) fn create_answer(&self, sender: mpsc::UnboundedSender<PipelineEvent>) {
        self.create_description("create-answer", true, sender);
    }

    fn create_description(
        &self,
        signal: &str,
        is_answer: bool,
        sender: mpsc::UnboundedSender<PipelineEvent>,
    ) {
        create_description(&self.webrtcbin, signal, is_answer, sender, &self.local_mids);
    }

    /// Set the session description the other party sent.
    pub(crate) fn set_remote_description(
        &self,
        sdp: &str,
        is_answer: bool,
    ) -> Result<(), PipelineError> {
        let description = Self::parse_description(sdp, is_answer)?;
        debug!("Setting the remote description:\n{}", sdp.trim_end());

        let transport = self.note_remote_transport(sdp);

        // A peer that rejected everything cannot be called, and saying so is
        // better than the alternative: no transport is negotiated, so no
        // candidate is ever sent, and a call with nothing wrong to report sits
        // in `Connecting` until the person watching it gives up.
        if is_answer && !transport.accepted_anything {
            return Err(PipelineError::Other(
                "the other party accepted none of the media we offered",
            ));
        }

        // With a promise, so that candidates held back for want of a remote
        // description can go in the moment there is one.
        let webrtcbin = self.webrtcbin.clone();
        let applied = self.remote_description_applied.clone();
        let pending = self.pending_remote_candidates.clone();
        let ufrag = self.remote_ice_ufrag.clone();
        let index = self.remote_transport_index.clone();
        let seen = self.remote_addresses.clone();
        let promise = gst::Promise::with_change_func(move |_| {
            flush_remote_candidates(&webrtcbin, &applied, &pending, &ufrag, &index, &seen);
        });

        self.webrtcbin
            .emit_by_name::<()>("set-remote-description", &[&description, &promise]);

        Ok(())
    }

    /// Take the offer, and answer it once it has actually been taken.
    ///
    /// `set-remote-description` is asynchronous. Emitting it and then calling
    /// `create-answer` on the next line asks `webrtcbin` to answer an offer it
    /// has not applied yet, and what comes back is an empty answer — which the
    /// caller then waits on for ever, because an empty answer is never sent.
    ///
    /// So the answer is created from inside the promise of the description,
    /// which is the only ordering `webrtcbin` guarantees.
    pub(crate) fn answer_remote_offer(
        &self,
        sdp: &str,
        sender: mpsc::UnboundedSender<PipelineEvent>,
    ) -> Result<(), PipelineError> {
        let description = Self::parse_description(sdp, false)?;
        debug!(
            "Setting the remote offer, and answering it when it is applied:\n{}",
            sdp.trim_end()
        );
        self.note_remote_transport(sdp);

        let webrtcbin = self.webrtcbin.clone();
        let applied = self.remote_description_applied.clone();
        let pending = self.pending_remote_candidates.clone();
        let ufrag = self.remote_ice_ufrag.clone();
        let index = self.remote_transport_index.clone();
        let seen = self.remote_addresses.clone();
        let mids = self.local_mids.clone();
        let promise = gst::Promise::with_change_func(move |reply| {
            if let Err(error) = reply {
                let _ = sender.unbounded_send(PipelineEvent::Error(format!(
                    "the other party's offer was refused: {error:?}"
                )));
                return;
            }

            debug!("The remote offer is applied; creating the answer");
            flush_remote_candidates(&webrtcbin, &applied, &pending, &ufrag, &index, &seen);
            create_description(&webrtcbin, "create-answer", true, sender, &mids);
        });

        self.webrtcbin
            .emit_by_name::<()>("set-remote-description", &[&description, &promise]);

        Ok(())
    }

    /// Note where the other party put the transport, and what it is called.
    ///
    /// Both come out of their description and neither can be assumed. A peer
    /// that rejects a section we offered leaves it in the SDP with `port 0`
    /// and no transport, and the live one — the one its candidates and its
    /// `ice-ufrag` belong to — is whichever the `a=group:BUNDLE` line names
    /// first.
    fn note_remote_transport(&self, sdp: &str) -> RemoteTransport {
        let transport = remote_transport(sdp);

        debug!(
            "The other party's transport is on m-line {} with ice-ufrag {}",
            transport.index,
            transport.ufrag.as_deref().unwrap_or("(none)")
        );

        if let Ok(mut stored) = self.remote_ice_ufrag.lock() {
            stored.clone_from(&transport.ufrag);
        }
        self.remote_transport_index
            .store(transport.index, Ordering::Relaxed);

        transport
    }

    /// Parse an SDP into something `webrtcbin` will take.
    fn parse_description(
        sdp: &str,
        is_answer: bool,
    ) -> Result<gst_webrtc::WebRTCSessionDescription, PipelineError> {
        let message = gst_sdp::SDPMessage::parse_buffer(sdp.as_bytes())
            .map_err(|_| PipelineError::Other("the other party sent an SDP we cannot parse"))?;

        let kind = if is_answer {
            gst_webrtc::WebRTCSDPType::Answer
        } else {
            gst_webrtc::WebRTCSDPType::Offer
        };

        Ok(gst_webrtc::WebRTCSessionDescription::new(kind, message))
    }

    /// Add an ICE candidate from the other party.
    ///
    /// An empty candidate means they have finished gathering.
    pub(crate) fn add_ice_candidate(&self, sdp_m_line_index: u32, candidate: &str) {
        // Nothing can be given to the remote ICE agent before there is one.
        // `webrtcbin` discards candidates added before the remote description
        // is applied, and on the answering side that was every one of them:
        // the buffered candidates went in on the line after
        // `set-remote-description` was emitted, and that call is asynchronous.
        //
        // Held first and judged later, because both of the things a candidate
        // is judged against — which section carries the transport, and what
        // that section's ufrag is — come out of the description that has not
        // arrived.
        if !self.remote_description_applied.load(Ordering::Relaxed) {
            debug!("Holding a remote candidate until the remote description is applied");
            if let Ok(mut pending) = self.pending_remote_candidates.lock() {
                pending.push(candidate.to_owned());
            }
            return;
        }

        let index = self.remote_transport_index.load(Ordering::Relaxed);

        if sdp_m_line_index != index {
            debug!(
                "Remote candidate is labelled m-line {sdp_m_line_index}; \
                 this call's transport is on section {index}"
            );
        }

        let ufrag = self.remote_ice_ufrag.lock().ok().and_then(|it| it.clone());
        add_remote_candidate(
            &self.webrtcbin,
            index,
            ufrag.as_deref(),
            &self.remote_addresses,
            candidate,
        );
    }

    /// Set whether our own microphone is muted.
    pub(crate) fn set_microphone_muted(&self, muted: bool) {
        let Some(volume) = &self.mic_volume else {
            return;
        };

        volume.set_property("mute", muted);
    }

    /// Set whether our own camera is muted.
    ///
    /// The frames are dropped rather than replaced with black ones. The other
    /// party is told separately, through `m.call.sdp_stream_metadata_changed`,
    /// so that they can show something better than the last frame we sent.
    pub(crate) fn set_camera_muted(&self, muted: bool) {
        let Some(valve) = &self.camera_valve else {
            return;
        };

        valve.set_property("drop", muted);
    }
}

/// Ask `webrtcbin` for an offer or an answer, and set it as the local
/// description once it arrives.
///
/// A free function rather than a method, because the answer has to be created
/// from inside the promise of `set-remote-description`, where there is no
/// `CallPipeline` left to call a method on.
fn create_description(
    webrtcbin: &gst::Element,
    signal: &str,
    is_answer: bool,
    sender: mpsc::UnboundedSender<PipelineEvent>,
    local_mids: &Arc<Mutex<Vec<String>>>,
) {
    let field = if is_answer { "answer" } else { "offer" };
    let for_local = webrtcbin.clone();
    let mids = local_mids.clone();

    let promise = gst::Promise::with_change_func(move |reply| {
        let description = match reply {
            Ok(Some(reply)) => reply
                .get::<gst_webrtc::WebRTCSessionDescription>(field)
                .ok(),
            Ok(None) => None,
            Err(error) => {
                let _ = sender.unbounded_send(PipelineEvent::Error(format!(
                    "could not create the {field}: {error:?}"
                )));
                return;
            }
        };

        let Some(description) = description else {
            let _ =
                sender.unbounded_send(PipelineEvent::Error(format!("the {field} came back empty")));
            return;
        };

        let sdp = description.sdp().as_text().unwrap_or_default();

        // Ours as well as theirs. What a codec line says about itself is the
        // half of a negotiation this client can actually change, and reading
        // it out of a `webrtcbin` in the state the call put it in is the only
        // way to know what went out — an offer made before the audio chain has
        // negotiated carries the capsfilter's caps and nothing more.
        debug!("Our own {field}:\n{}", sdp.trim_end());

        // The mids of our own sections, so that a candidate can name the one
        // it belongs to and not only its index.
        if let Ok(mut mids) = mids.lock() {
            *mids = section_mids(&sdp);
        }

        for_local.emit_by_name::<()>(
            "set-local-description",
            &[&description, &None::<gst::Promise>],
        );

        let _ = sender.unbounded_send(PipelineEvent::LocalDescription { sdp, is_answer });
    });

    webrtcbin.emit_by_name::<()>(signal, &[&None::<gst::Structure>, &promise]);
}

/// Play an incoming stream.
fn attach_receiver(
    pipeline: &gst::Pipeline,
    pad: &gst::Pad,
    remote_video_sink: &gst::Element,
    sender: &mpsc::UnboundedSender<PipelineEvent>,
) -> Result<(), PipelineError> {
    let decodebin = gst::ElementFactory::make("decodebin").build()?;

    let pipeline_weak = pipeline.downgrade();
    let video_sink = remote_video_sink.clone();
    let sender = sender.clone();
    decodebin.connect_pad_added(move |_, pad| {
        let Some(pipeline) = pipeline_weak.upgrade() else {
            return;
        };

        if let Err(error) = attach_decoded_pad(&pipeline, pad, &video_sink, &sender) {
            error!("Could not play a decoded stream: {error}");
        }
    });

    pipeline.add(&decodebin)?;
    decodebin.sync_state_with_parent()?;

    let Some(sink_pad) = decodebin.static_pad("sink") else {
        return Err(PipelineError::Other("decodebin has no sink pad"));
    };

    pad.link(&sink_pad)
        .map_err(|_| PipelineError::Link("an incoming stream"))?;

    Ok(())
}

/// Send a decoded stream to a speaker or to the screen.
fn attach_decoded_pad(
    pipeline: &gst::Pipeline,
    pad: &gst::Pad,
    remote_video_sink: &gst::Element,
    sender: &mpsc::UnboundedSender<PipelineEvent>,
) -> Result<(), PipelineError> {
    let Some(caps) = pad.current_caps() else {
        return Ok(());
    };
    let Some(structure) = caps.structure(0) else {
        return Ok(());
    };
    let name = structure.name();

    if name.starts_with("audio/") {
        let queue = gst::ElementFactory::make("queue").build()?;
        let convert = gst::ElementFactory::make("audioconvert").build()?;
        let resample = gst::ElementFactory::make("audioresample").build()?;
        let sink = gst::ElementFactory::make("autoaudiosink").build()?;

        let elements = [&queue, &convert, &resample, &sink];
        pipeline.add_many(elements)?;
        gst::Element::link_many(elements)?;

        for element in elements {
            element.sync_state_with_parent()?;
        }

        link_into(pad, &queue)?;
    } else if name.starts_with("video/") {
        let queue = gst::ElementFactory::make("queue").build()?;
        let convert = gst::ElementFactory::make("videoconvert").build()?;

        // The sink was built with the pipeline so that its paintable could be
        // handed to the widget before a single frame arrived.
        if remote_video_sink.parent().is_none() {
            pipeline.add(remote_video_sink)?;
        }

        pipeline.add_many([&queue, &convert])?;
        gst::Element::link_many([&queue, &convert, remote_video_sink])?;

        for element in [&queue, &convert, remote_video_sink] {
            element.sync_state_with_parent()?;
        }

        link_into(pad, &queue)?;

        // Now, and not when the pipeline was built: the paintable has existed
        // since then and has had nothing to draw. A call that starts with
        // audio and gains video partway through says so here.
        debug!("The other party's video is attached to the sink");
        let _ = sender.unbounded_send(PipelineEvent::RemoteVideo);
    }

    Ok(())
}

/// Hand `webrtcbin` one remote candidate, on the section that has a transport.
///
/// Every remote candidate goes on the bundled section whatever index the other
/// party put on it: this client always negotiates `max-bundle`, so a call has
/// exactly one transport, and a candidate placed on a section without one
/// names nothing and is dropped without a word. Which section that is comes
/// from their description — usually the first, and not always.
fn add_remote_candidate(
    webrtcbin: &gst::Element,
    index: u32,
    description_ufrag: Option<&str>,
    seen: &RemoteAddresses,
    candidate: &str,
) {
    if candidate.is_empty() {
        // The spec spells end-of-candidates as an empty string; `webrtcbin`
        // spells it as a NULL candidate, and hands an empty string to its
        // parser like any other, where it is not a candidate and fails.
        debug!("End of candidates from the other party");
        webrtcbin.emit_by_name::<()>("add-ice-candidate", &[&index, &None::<String>]);
        return;
    }

    if !is_usable_candidate(candidate) {
        debug!("Ignoring a remote candidate on an address only we could reach: {candidate}");
        return;
    }

    // One candidate per transport address. A second one for an address we
    // already have offers no path we do not already have, and costs the
    // process: libnice matches a check response to a remote candidate by
    // address, so the second pair's response is credited to the first pair,
    // which stops being `SUCCEEDED` while staying valid — and the nomination
    // tick asserts that no such pair exists.
    //
    // Measured on 23 August 2026: Element for Android offered
    // `192.168.50.234 59098 typ host` and `192.168.50.234 59098 typ srflx`,
    // its own address seen through a STUN server that did not translate it,
    // and the client died the instant checking began.
    if let Some(address) = transport_address(candidate)
        && let Ok(mut seen) = seen.lock()
        && !seen.insert(address.clone())
    {
        debug!("Ignoring a second remote candidate for {address}: {candidate}");
        return;
    }

    // A candidate tagged with an ICE session other than the description's is
    // one libnice discards, and discarding all of them looks exactly like a
    // network that will not carry the call. The tag is dropped and the address
    // kept: an address is what a candidate is for, and the description is what
    // says which session this call is.
    let stripped;
    let candidate = match mismatched_ufrag(candidate, description_ufrag) {
        Some(mismatch) => {
            warn!(
                "Remote candidate names ICE session {mismatch}, which is not the \
                 remote description's; dropping the tag and keeping the address"
            );
            stripped = without_ufrag(candidate);
            stripped.as_str()
        }
        None => candidate,
    };

    // The whole line, because a candidate that does not parse is one
    // `webrtcbin` drops in silence, and "unknown" here is the difference
    // between a pair that failed and a pair that never existed.
    debug!(
        "Adding remote {} candidate on m-line {index}: {candidate}",
        candidate_type(candidate)
    );
    webrtcbin.emit_by_name::<()>("add-ice-candidate", &[&index, &candidate]);
}

/// The `a=mid:` of every media section of a description, in m-line order.
fn section_mids(sdp: &str) -> Vec<String> {
    let mut mids: Vec<String> = Vec::new();
    let mut sections = 0usize;

    for line in sdp.lines().map(str::trim_end) {
        if line.starts_with("m=") {
            sections += 1;
        } else if let Some(mid) = line.strip_prefix("a=mid:")
            && mids.len() < sections
        {
            // The first `a=mid:` of the section it appears in, and one entry
            // per section so that the index of a candidate lines up.
            mids.resize(sections - 1, String::new());
            mids.push(mid.to_owned());
        }
    }

    mids
}

/// Whether a remote candidate is one this machine could ever use.
///
/// The other end's loopback address is not a path to the other end. It is a
/// path to *us*: a check sent to `127.0.0.1` leaves our machine and arrives
/// back at it. Every libwebrtc client offers them and nobody can use them, so
/// they are pairs that exist only to fail.
///
/// Which matters more than the wasted checks. libnice 0.1.23 aborts the whole
/// process — `conncheck.c:959`, `assertion failed: (p->state ==
/// NICE_CHECK_SUCCEEDED)` — when it comes to nominate and finds a pair that is
/// valid and no longer succeeded, and a pair that succeeds and then dies is
/// how one gets into that state. Every pair that cannot work is a chance at
/// it, and the assertion is still on libnice's master branch, so there is no
/// version of it to move to.
fn is_usable_candidate(candidate: &str) -> bool {
    // `candidate:<foundation> <component> <transport> <priority> <address> …`
    let Some(address) = candidate.split_whitespace().nth(4) else {
        return true;
    };

    !(address == "::1" || address.starts_with("127."))
}

/// The transport address a candidate line names, as a key.
///
/// `candidate:<foundation> <component> <transport> <priority> <address>
/// <port>`, plus the TCP type, which is the one thing that distinguishes two
/// TCP candidates on one address — an active one always says port 9.
fn transport_address(candidate: &str) -> Option<String> {
    let words: Vec<&str> = candidate.split_whitespace().collect();
    let transport = words.get(2)?.to_lowercase();
    let address = words.get(4)?;
    let port = words.get(5)?;

    let tcptype = words
        .iter()
        .position(|word| *word == "tcptype")
        .and_then(|at| words.get(at + 1))
        .unwrap_or(&"");

    Some(
        format!("{transport} {address} {port} {tcptype}")
            .trim_end()
            .to_owned(),
    )
}

/// The ufrag a candidate names, when it is not the remote description's.
///
/// Returns `None` when they agree, when the candidate names none, or when the
/// description named none either.
fn mismatched_ufrag(candidate: &str, description_ufrag: Option<&str>) -> Option<String> {
    let theirs = candidate
        .split(" ufrag ")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())?;

    (theirs != description_ufrag?).then(|| theirs.to_owned())
}

/// Hand `webrtcbin` the remote candidates that were waiting for a description.
fn flush_remote_candidates(
    webrtcbin: &gst::Element,
    applied: &Arc<AtomicBool>,
    pending: &PendingCandidates,
    ufrag: &Arc<Mutex<Option<String>>>,
    index: &Arc<AtomicU32>,
    seen: &RemoteAddresses,
) {
    applied.store(true, Ordering::Relaxed);

    let Ok(mut pending) = pending.lock() else {
        return;
    };

    if pending.is_empty() {
        return;
    }

    debug!(
        "The remote description is applied; adding {} candidate(s) that were waiting",
        pending.len()
    );

    let index = index.load(Ordering::Relaxed);
    let ufrag = ufrag.lock().ok().and_then(|it| it.clone());

    for candidate in pending.drain(..) {
        add_remote_candidate(webrtcbin, index, ufrag.as_deref(), seen, &candidate);
    }
}

/// Where the other party's description puts the transport of the call.
#[derive(Debug, Default, PartialEq, Eq)]
struct RemoteTransport {
    /// The m-line index of the section carrying it.
    index: u32,
    /// That section's `ice-ufrag`, which its candidates name too.
    ufrag: Option<String>,
    /// Whether any section at all was accepted.
    ///
    /// A description in which every section has `port 0` is a peer that agreed
    /// to nothing. There is no transport to find, and no call to be had.
    accepted_anything: bool,
}

/// Read the transport out of a session description.
///
/// `a=group:BUNDLE` names the sections sharing one transport, in order, so its
/// first mid is the section that owns it. Without a usable bundle line the
/// first section with a port is the next best answer, and the ufrag is read
/// from whichever section that turns out to be — not from the top of the file,
/// where a rejected section's stale credentials sit waiting to be mistaken for
/// the call's.
fn remote_transport(sdp: &str) -> RemoteTransport {
    struct Section<'a> {
        mid: Option<&'a str>,
        port: &'a str,
        ufrag: Option<&'a str>,
    }

    let mut bundle: Vec<&str> = Vec::new();
    let mut sections: Vec<Section<'_>> = Vec::new();

    for line in sdp.lines().map(str::trim_end) {
        if let Some(media) = line.strip_prefix("m=") {
            sections.push(Section {
                mid: None,
                // `m=<media> <port> <proto> <formats…>`
                port: media.split_whitespace().nth(1).unwrap_or("0"),
                ufrag: None,
            });
        } else if let Some(mids) = line.strip_prefix("a=group:BUNDLE") {
            bundle = mids.split_whitespace().collect();
        } else if let Some(section) = sections.last_mut() {
            if let Some(mid) = line.strip_prefix("a=mid:") {
                section.mid = Some(mid);
            } else if let Some(ufrag) = line.strip_prefix("a=ice-ufrag:") {
                section.ufrag = Some(ufrag);
            }
        }
    }

    let is_live = |section: &Section<'_>| section.port != "0";
    let accepted_anything = sections.iter().any(is_live);

    // The bundled section, when the line names one we can find and that the
    // answer did not go on to reject.
    let bundled = bundle.first().and_then(|mid| {
        sections
            .iter()
            .position(|section| section.mid == Some(*mid) && is_live(section))
    });

    let index = bundled
        .or_else(|| sections.iter().position(is_live))
        .unwrap_or(0);

    RemoteTransport {
        index: u32::try_from(index).unwrap_or(0),
        ufrag: sections
            .get(index)
            .and_then(|section| section.ufrag)
            .map(ToOwned::to_owned),
        accepted_anything,
    }
}

/// A candidate line without its `ufrag` tag.
///
/// The tag is an extension attribute — a pair of words at the end of the line
/// — so removing it leaves a candidate every parser still reads.
fn without_ufrag(candidate: &str) -> String {
    let mut kept = Vec::new();
    let mut words = candidate.split_whitespace();

    while let Some(word) = words.next() {
        if word == "ufrag" {
            words.next();
            continue;
        }

        kept.push(word);
    }

    kept.join(" ")
}

/// The type of an ICE candidate — `host`, `srflx`, `prflx` or `relay`.
///
/// It is the word after `typ` in the candidate line. Worth naming in the log
/// because the absence of `relay` is the difference between "the TURN server
/// was configured" and "the TURN server gave us an allocation", and those look
/// identical everywhere else.
fn candidate_type(candidate: &str) -> &str {
    candidate
        .split(" typ ")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or("unknown")
}

/// A queue that drops what it cannot pass on, for the branches feeding
/// `webrtcbin`.
///
/// `webrtcbin` consumes nothing until the call is connected — there is no
/// answer, no transport, and nowhere for a packet to go. A plain `queue` in
/// front of it fills up while the other end is still deciding whether to pick
/// up, and then blocks the element behind it.
///
/// For audio that element is the microphone, and the seconds recorded while it
/// was blocked would be the first thing the other party hears. For video it is
/// worse than it sounds: the camera feeds a `tee`, one branch of which is the
/// self-view, and a `tee` runs no faster than its slowest branch — so a full
/// send queue stops the preview as well, and what the caller sees is their own
/// face frozen on the first frame for as long as the call is ringing. That is
/// what it did, on Linux and on macOS alike.
///
/// The oldest is the right end to drop from: this is a live call, and a frame
/// that could not be sent while nobody was listening is of no use once somebody
/// is. Nothing is dropped once the call connects, because from then on
/// `webrtcbin` is taking them.
fn leaky_queue() -> Result<gst::Element, PipelineError> {
    Ok(gst::ElementFactory::make("queue")
        .property_from_str("leaky", "downstream")
        .build()?)
}

/// Link a pad into the sink of an element.
fn link_into(pad: &gst::Pad, element: &gst::Element) -> Result<(), PipelineError> {
    let Some(sink_pad) = element.static_pad("sink") else {
        return Err(PipelineError::Other("the element has no sink pad"));
    };

    pad.link(&sink_pad)
        .map_err(|_| PipelineError::Link("a decoded stream"))?;

    Ok(())
}

/// The media sections of an SDP, in order.
///
/// A section we cannot name is a section we cannot make room for, and an answer
/// whose sections do not line up with the offer's is one `webrtcbin` refuses.
/// So an offer that yields nothing here is an offer to reject rather than one
/// to guess at — including one that does not parse, which
/// `SDPMessage::parse_buffer` reports by handing back an empty message rather
/// than an error.
fn media_sections(sdp: &str) -> Vec<MediaKind> {
    let Ok(message) = gst_sdp::SDPMessage::parse_buffer(sdp.as_bytes()) else {
        warn!("Could not parse the offer");
        return Vec::new();
    };

    message
        .medias()
        .filter_map(|media| MediaKind::from_sdp_media(media.media().unwrap_or_default()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The line Element for Android sent on 23 August 2026, tag and all.
    const TAGGED_CANDIDATE: &str = "candidate:2907572369 1 udp 2122194687 \
192.168.50.234 34239 typ host generation 0 ufrag Jlts network-id 6 \
network-cost 10";

    /// Element for Android's answer to a video call, 23 August 2026: the
    /// audio section rejected, the video one kept and bundled alone.
    const ANSWER_WITH_AUDIO_REJECTED: &str = "\
v=0\r\n\
o=- 1645252129759934538 2 IN IP4 127.0.0.1\r\n\
s=-\r\n\
t=0 0\r\n\
a=group:BUNDLE video1\r\n\
m=audio 0 UDP/TLS/RTP/SAVPF 0\r\n\
a=ice-ufrag:zHw1\r\n\
a=mid:audio0\r\n\
m=video 9 UDP/TLS/RTP/SAVPF 96\r\n\
a=ice-ufrag:dv9I\r\n\
a=mid:video1\r\n";

    /// Its answer to a voice call the same minute: nothing accepted at all.
    const ANSWER_WITH_NOTHING_ACCEPTED: &str = "\
v=0\r\n\
o=- 1586226428592016302 2 IN IP4 127.0.0.1\r\n\
s=-\r\n\
t=0 0\r\n\
a=group:BUNDLE\r\n\
m=audio 0 UDP/TLS/RTP/SAVPF 0\r\n\
a=ice-ufrag:Orve\r\n\
a=mid:audio0\r\n";

    #[test]
    fn the_same_address_twice_is_one_address() {
        // The pair Element for Android sent that killed the process: its own
        // address, once as a host candidate and once seen through STUN.
        let host = "candidate:2160058006 1 udp 2122194687 192.168.50.234 59098 typ host \
                    generation 0";
        let srflx = "candidate:1628368629 1 udp 1685987071 192.168.50.234 59098 typ srflx \
                     raddr 192.168.50.234 rport 59098 generation 0";

        assert_eq!(transport_address(host), transport_address(srflx));
    }

    #[test]
    fn tcp_candidates_on_one_port_are_told_apart_by_their_type() {
        let active = "candidate:1 1 tcp 1518018303 100.117.101.3 9 typ host tcptype active";
        let passive = "candidate:2 1 tcp 1518018303 100.117.101.3 9 typ host tcptype passive";
        let elsewhere = "candidate:3 1 tcp 1518018303 192.168.50.234 9 typ host tcptype active";

        assert_ne!(transport_address(active), transport_address(passive));
        assert_ne!(transport_address(active), transport_address(elsewhere));
    }

    #[test]
    fn the_mid_of_each_section_is_read_in_order() {
        // What `webrtcbin` writes for a video call.
        let sdp = "\
v=0\r\n\
a=group:BUNDLE audio0 video1\r\n\
m=audio 9 UDP/TLS/RTP/SAVPF 111\r\n\
a=rtcp-mux\r\n\
a=mid:audio0\r\n\
m=video 0 UDP/TLS/RTP/SAVPF 96\r\n\
a=bundle-only\r\n\
a=mid:video1\r\n";

        assert_eq!(section_mids(sdp), vec!["audio0", "video1"]);
    }

    #[test]
    fn a_section_without_a_mid_still_holds_its_place() {
        let sdp = "\
v=0\r\n\
m=audio 9 UDP/TLS/RTP/SAVPF 111\r\n\
m=video 9 UDP/TLS/RTP/SAVPF 96\r\n\
a=mid:video1\r\n";

        // The index of a candidate has to keep lining up with the section.
        assert_eq!(section_mids(sdp), vec!["", "video1"]);
    }

    #[test]
    fn a_remote_loopback_candidate_is_not_a_path_to_them() {
        let host = "candidate:324044106 1 udp 2122194687 192.168.50.234 48264 typ host";
        let loopback = "candidate:971919295 1 udp 2121867007 127.0.0.1 40986 typ host";
        let loopback6 = "candidate:683826487 1 udp 2121940223 ::1 54784 typ host";

        assert!(is_usable_candidate(host));
        assert!(!is_usable_candidate(loopback));
        assert!(!is_usable_candidate(loopback6));

        // Not a candidate at all: end-of-candidates, which has its own path.
        assert!(is_usable_candidate(""));
    }

    #[test]
    fn the_transport_is_the_section_the_bundle_names() {
        let transport = remote_transport(ANSWER_WITH_AUDIO_REJECTED);

        // Not section 0, whose `ice-ufrag` is the one a call spent five
        // seconds measuring every candidate against.
        assert_eq!(transport.index, 1);
        assert_eq!(transport.ufrag.as_deref(), Some("dv9I"));
        assert!(transport.accepted_anything);
    }

    #[test]
    fn a_description_that_accepts_nothing_says_so() {
        let transport = remote_transport(ANSWER_WITH_NOTHING_ACCEPTED);

        assert!(!transport.accepted_anything);
    }

    #[test]
    fn a_bundle_of_everything_puts_the_transport_first() {
        let sdp = "\
v=0\r\n\
a=group:BUNDLE audio0 video1\r\n\
m=audio 9 UDP/TLS/RTP/SAVPF 111\r\n\
a=ice-ufrag:abcd\r\n\
a=mid:audio0\r\n\
m=video 0 UDP/TLS/RTP/SAVPF 96\r\n\
a=ice-ufrag:efgh\r\n\
a=mid:video1\r\n";

        let transport = remote_transport(sdp);

        assert_eq!(transport.index, 0);
        assert_eq!(transport.ufrag.as_deref(), Some("abcd"));
    }

    #[test]
    fn a_tag_matching_the_description_is_left_alone() {
        assert_eq!(mismatched_ufrag(TAGGED_CANDIDATE, Some("Jlts")), None);
        assert_eq!(
            mismatched_ufrag(TAGGED_CANDIDATE, Some("+sSs")).as_deref(),
            Some("Jlts")
        );
    }

    #[test]
    fn the_ufrag_tag_is_removed_and_the_rest_is_not() {
        assert_eq!(
            without_ufrag(TAGGED_CANDIDATE),
            "candidate:2907572369 1 udp 2122194687 192.168.50.234 34239 typ host \
             generation 0 network-id 6 network-cost 10"
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        );
    }

    #[test]
    fn a_candidate_with_no_tag_is_unchanged() {
        let plain = "candidate:1 1 UDP 2015363327 192.168.50.140 60336 typ host";
        assert_eq!(without_ufrag(plain), plain);
    }

    const AUDIO_VIDEO_OFFER: &str = "\
v=0\r\n\
o=- 0 0 IN IP4 127.0.0.1\r\n\
s=-\r\n\
t=0 0\r\n\
m=audio 9 UDP/TLS/RTP/SAVPF 111\r\n\
c=IN IP4 0.0.0.0\r\n\
a=rtpmap:111 opus/48000/2\r\n\
m=video 9 UDP/TLS/RTP/SAVPF 96\r\n\
c=IN IP4 0.0.0.0\r\n\
a=rtpmap:96 VP8/90000\r\n";

    #[test]
    fn a_candidate_names_its_own_type() {
        assert_eq!(
            candidate_type("candidate:1 1 UDP 2013266431 192.168.1.5 54321 typ host"),
            "host"
        );
        assert_eq!(
            candidate_type(
                "candidate:4 1 UDP 92274687 174.74.218.66 49160 typ relay raddr 0.0.0.0 rport 0"
            ),
            "relay"
        );
        // End-of-candidates is an empty string and names nothing.
        assert_eq!(candidate_type(""), "unknown");
    }

    #[test]
    fn the_offer_decides_the_order_of_the_sections() {
        gst::init().unwrap();

        assert_eq!(
            media_sections(AUDIO_VIDEO_OFFER),
            vec![MediaKind::Audio, MediaKind::Video]
        );
    }

    #[test]
    fn an_offer_we_cannot_read_yields_nothing_to_answer_with() {
        gst::init().unwrap();

        // `parse_buffer` does not fail on this: it hands back a message with no
        // media in it, which is why the emptiness is what gets checked and not
        // the error.
        assert!(media_sections("not an sdp at all").is_empty());
    }
}
