//! The `GStreamer` side of a call.
//!
//! This is where WebRTC actually happens. Everything above it deals in Matrix
//! events; everything here deals in SDP, ICE candidates and media.

use futures_channel::mpsc;
use gst::prelude::*;
use gtk::{gdk, glib};
use tracing::{debug, error, warn};

use super::turn::TurnServer;

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
    },
    /// No more ICE candidates will be gathered.
    IceGatheringDone,
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
        turn_servers: &[TurnServer],
    ) -> Result<(Self, mpsc::UnboundedReceiver<PipelineEvent>), PipelineError> {
        let media = if with_video {
            vec![MediaKind::Audio, MediaKind::Video]
        } else {
            vec![MediaKind::Audio]
        };

        Self::build(media, turn_servers)
    }

    /// Build a pipeline for a call we are answering.
    ///
    /// The media sections have to line up with the offer, section for section,
    /// or `webrtcbin` refuses the answer. So the offer decides, not us: what we
    /// can source we send, and what we cannot we still make room for.
    pub(crate) fn new_for_answer(
        offer_sdp: &str,
        turn_servers: &[TurnServer],
    ) -> Result<(Self, mpsc::UnboundedReceiver<PipelineEvent>), PipelineError> {
        let media = media_sections(offer_sdp);
        Self::build(media, turn_servers)
    }

    fn build(
        media: Vec<MediaKind>,
        turn_servers: &[TurnServer],
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

        // No default STUN server. The usual one is Google's, and pointing every
        // call at a third party to learn our own address tells that third party
        // that a call is happening at all. A TURN server answers STUN binding
        // requests too, so the homeserver's own is enough — and where there is
        // none, host candidates are what is left.
        if turn_servers.is_empty() {
            // Host candidates only. Two people on one network still find each
            // other; anybody behind a NAT does not.
            warn!("No TURN server for this call; only host candidates will be gathered");
        }

        for server in turn_servers {
            let added = webrtcbin.emit_by_name::<bool>("add-turn-server", &[&server.uri]);

            // The credentials are in the URI, so it is not logged.
            if added {
                debug!("webrtcbin accepted a TURN server");
            } else {
                warn!("webrtcbin refused a TURN server");
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

    /// Add the microphone to the pipeline, encoded and packetised for the wire.
    fn add_audio_source(&mut self) -> Result<(), PipelineError> {
        let source = gst::ElementFactory::make("autoaudiosrc").build()?;
        let queue = leaky_queue()?;
        let convert = gst::ElementFactory::make("audioconvert").build()?;
        let resample = gst::ElementFactory::make("audioresample").build()?;
        let volume = gst::ElementFactory::make("volume").build()?;
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
                    .build(),
            )
            .build()?;

        let elements = [
            &source, &queue, &convert, &resample, &volume, &encoder, &payloader, &caps,
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
        self.webrtcbin
            .connect("on-ice-candidate", false, move |values| {
                let sdp_m_line_index = values.get(1)?.get::<u32>().ok()?;
                let candidate = values.get(2)?.get::<String>().ok()?;
                debug!("webrtcbin gathered a candidate for m-line {sdp_m_line_index}");

                let _ = ice_sender.unbounded_send(PipelineEvent::IceCandidate {
                    candidate,
                    sdp_m_line_index,
                });

                None
            });

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

        // Incoming media arrives as a new pad per stream, still packetised.
        let pipeline = self.pipeline.downgrade();
        let error_sender = sender.clone();
        self.webrtcbin.connect_pad_added(move |_, pad| {
            if pad.direction() != gst::PadDirection::Src {
                return;
            }
            let Some(pipeline) = pipeline.upgrade() else {
                return;
            };

            if let Err(error) = attach_receiver(&pipeline, pad, &remote_video_sink) {
                error!("Could not play an incoming stream: {error}");
                let _ = error_sender.unbounded_send(PipelineEvent::Error(error.to_string()));
            }
        });

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
        create_description(&self.webrtcbin, signal, is_answer, sender);
    }

    /// Set the session description the other party sent.
    pub(crate) fn set_remote_description(
        &self,
        sdp: &str,
        is_answer: bool,
    ) -> Result<(), PipelineError> {
        let description = Self::parse_description(sdp, is_answer)?;
        debug!("Setting the remote description");

        self.webrtcbin.emit_by_name::<()>(
            "set-remote-description",
            &[&description, &None::<gst::Promise>],
        );

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
        debug!("Setting the remote offer, and answering it when it is applied");

        let webrtcbin = self.webrtcbin.clone();
        let promise = gst::Promise::with_change_func(move |reply| {
            if let Err(error) = reply {
                let _ = sender.unbounded_send(PipelineEvent::Error(format!(
                    "the other party's offer was refused: {error:?}"
                )));
                return;
            }

            debug!("The remote offer is applied; creating the answer");
            create_description(&webrtcbin, "create-answer", true, sender);
        });

        self.webrtcbin
            .emit_by_name::<()>("set-remote-description", &[&description, &promise]);

        Ok(())
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
        if candidate.is_empty() {
            // The spec spells end-of-candidates as an empty string;
            // `webrtcbin` spells it as a NULL candidate, and hands an empty
            // string to its parser like any other, where it is not a candidate
            // and fails.
            debug!("End of candidates from the other party");
            self.webrtcbin
                .emit_by_name::<()>("add-ice-candidate", &[&sdp_m_line_index, &None::<String>]);
            return;
        }

        debug!("Adding remote candidate for m-line {sdp_m_line_index}");
        self.webrtcbin
            .emit_by_name::<()>("add-ice-candidate", &[&sdp_m_line_index, &candidate]);
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
) {
    let field = if is_answer { "answer" } else { "offer" };
    let for_local = webrtcbin.clone();

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
) -> Result<(), PipelineError> {
    let decodebin = gst::ElementFactory::make("decodebin").build()?;

    let pipeline_weak = pipeline.downgrade();
    let video_sink = remote_video_sink.clone();
    decodebin.connect_pad_added(move |_, pad| {
        let Some(pipeline) = pipeline_weak.upgrade() else {
            return;
        };

        if let Err(error) = attach_decoded_pad(&pipeline, pad, &video_sink) {
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
    }

    Ok(())
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
