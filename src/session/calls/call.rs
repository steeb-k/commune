use std::time::Duration;

use futures_util::StreamExt;
use gtk::{gdk, glib, glib::clone, prelude::*, subclass::prelude::*};
use rand::distr::{Alphanumeric, SampleString};
use ruma::{
    OwnedUserId, OwnedVoipId, UInt, UserId, VoipVersionId,
    events::{
        AnyMessageLikeEventContent,
        call::{
            SessionDescription, StreamMetadata, StreamPurpose,
            answer::CallAnswerEventContent,
            candidates::{CallCandidatesEventContent, Candidate},
            hangup::{CallHangupEventContent, Reason},
            invite::CallInviteEventContent,
            reject::CallRejectEventContent,
            sdp_stream_metadata_changed::CallSdpStreamMetadataChangedEventContent,
            select_answer::CallSelectAnswerEventContent,
        },
    },
};
use tracing::{debug, error, warn};

use super::{
    pipeline::{CallPipeline, PipelineEvent},
    state::{CallEndReason, CallState},
    turn::TurnServer,
};
use crate::{
    session::{Member, Room, UserExt},
    spawn, spawn_tokio,
};

/// The user ID of our own user in the given room.
fn own_user_id(room: &Room) -> OwnedUserId {
    room.own_member().user_id().clone()
}

/// How long an invite of ours is valid for.
///
/// The spec's recommended minimum is 90 seconds, on the grounds that the person
/// on the other end needs time to actually pick up.
const INVITE_LIFETIME: Duration = Duration::from_secs(90);

/// How long to gather candidates before sending the first batch, after an
/// invite.
///
/// The spec suggests two seconds, since there is a natural pause anyway while
/// the other end decides whether to answer.
const CANDIDATE_BATCH_AFTER_INVITE: Duration = Duration::from_secs(2);

/// How long to gather candidates before sending the first batch, after an
/// answer.
///
/// Half a second: here there is no natural pause, and every one of them is
/// between the two people and hearing each other.
const CANDIDATE_BATCH_AFTER_ANSWER: Duration = Duration::from_millis(500);

/// The `VoIP` version we speak.
///
/// Version `1` is what `party_id`, `m.call.select_answer`, `m.call.reject` and
/// the `invitee` field all belong to. Version `0` has none of them and no way
/// to tell two answering devices apart.
fn voip_version() -> VoipVersionId {
    VoipVersionId::V1
}

/// Generate an identifier that fits the Opaque Identifier Grammar.
fn opaque_id(length: usize) -> String {
    Alphanumeric.sample_string(&mut rand::rng(), length)
}

mod imp {
    use std::{
        cell::{Cell, OnceCell, RefCell},
        marker::PhantomData,
    };

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::Call)]
    pub struct Call {
        /// The room the call is taking place in.
        #[property(get, construct_only)]
        pub(super) room: OnceCell<Room>,
        /// The ID of the call, shared by both parties.
        #[property(get = Self::call_id_string)]
        call_id_string: PhantomData<String>,
        pub(super) call_id: OnceCell<OwnedVoipId>,
        /// Our own party ID, which identifies this device for this call.
        pub(super) party_id: OnceCell<OwnedVoipId>,
        /// The party ID of the other end, once we know it.
        pub(super) remote_party_id: RefCell<Option<OwnedVoipId>>,
        /// The user on the other end.
        #[property(get)]
        pub(super) remote_member: RefCell<Option<Member>>,
        /// Whether we placed this call.
        #[property(get, construct_only)]
        pub(super) is_outgoing: Cell<bool>,
        /// Where the call has got to.
        #[property(get, builder(CallState::default()))]
        pub(super) state: Cell<CallState>,
        /// Why the call ended, if it has.
        #[property(get, builder(CallEndReason::default()))]
        pub(super) end_reason: Cell<CallEndReason>,
        /// Whether this call carries video.
        #[property(get)]
        pub(super) has_video: Cell<bool>,
        /// Whether our own microphone is muted.
        #[property(get, set = Self::set_microphone_muted, explicit_notify)]
        pub(super) is_microphone_muted: Cell<bool>,
        /// Whether our own camera is muted.
        #[property(get, set = Self::set_camera_muted, explicit_notify)]
        pub(super) is_camera_muted: Cell<bool>,
        /// Whether the other party has muted their camera.
        #[property(get)]
        pub(super) is_remote_camera_muted: Cell<bool>,
        /// The picture of the other party.
        #[property(get)]
        pub(super) remote_paintable: RefCell<Option<gdk::Paintable>>,
        /// The picture from our own camera.
        #[property(get)]
        pub(super) local_paintable: RefCell<Option<gdk::Paintable>>,
        /// When the call connected, in seconds since the Unix epoch.
        #[property(get)]
        pub(super) connected_at: Cell<u64>,

        /// The WebRTC side of the call.
        pub(super) pipeline: RefCell<Option<CallPipeline>>,
        /// The offer of an incoming call, until it is answered.
        pub(super) pending_offer: RefCell<Option<SessionDescription>>,
        /// Candidates that arrived before there was a pipeline to give them to.
        pub(super) pending_candidates: RefCell<Vec<Candidate>>,
        /// Candidates of our own that have not been sent yet.
        pub(super) outgoing_candidates: RefCell<Vec<Candidate>>,
        /// The timeout that sends the next batch of candidates.
        pub(super) candidate_batch: RefCell<Option<glib::SourceId>>,
        /// The timeout that gives up on an unanswered invite.
        pub(super) lifetime_timeout: RefCell<Option<glib::SourceId>>,
        /// The ID of the stream we send, taken from our own SDP.
        pub(super) local_stream_id: RefCell<Option<String>>,
        /// Whether we have chosen which answer to use.
        pub(super) answer_selected: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Call {
        const NAME: &'static str = "Call";
        type Type = super::Call;
    }

    #[glib::derived_properties]
    impl ObjectImpl for Call {
        fn dispose(&self) {
            self.cancel_timeouts();
        }
    }

    impl Call {
        /// The ID of the call, as a string.
        fn call_id_string(&self) -> String {
            self.call_id
                .get()
                .map(ToString::to_string)
                .unwrap_or_default()
        }

        /// Set whether our own microphone is muted.
        fn set_microphone_muted(&self, muted: bool) {
            if self.is_microphone_muted.get() == muted {
                return;
            }

            self.is_microphone_muted.set(muted);

            if let Some(pipeline) = &*self.pipeline.borrow() {
                pipeline.set_microphone_muted(muted);
            }

            self.obj().notify_is_microphone_muted();
            self.obj().send_stream_metadata();
        }

        /// Set whether our own camera is muted.
        fn set_camera_muted(&self, muted: bool) {
            if self.is_camera_muted.get() == muted {
                return;
            }

            self.is_camera_muted.set(muted);

            if let Some(pipeline) = &*self.pipeline.borrow() {
                pipeline.set_camera_muted(muted);
            }

            self.obj().notify_is_camera_muted();
            self.obj().send_stream_metadata();
        }

        /// Stop both timeouts.
        pub(super) fn cancel_timeouts(&self) {
            if let Some(source) = self.candidate_batch.take() {
                source.remove();
            }
            if let Some(source) = self.lifetime_timeout.take() {
                source.remove();
            }
        }
    }
}

glib::wrapper! {
    /// A one-to-one voice or video call.
    pub struct Call(ObjectSubclass<imp::Call>);
}

impl Call {
    /// Place a call in the given room.
    pub(crate) fn place(room: &Room, with_video: bool, turn_servers: &[TurnServer]) -> Self {
        let obj = glib::Object::builder::<Self>()
            .property("room", room)
            .property("is-outgoing", true)
            .build();
        let imp = obj.imp();

        let _ = imp.call_id.set(OwnedVoipId::from(opaque_id(16)));
        let _ = imp.party_id.set(OwnedVoipId::from(opaque_id(8)));
        imp.state.set(CallState::Dialing);
        imp.has_video.set(with_video);

        obj.load_remote_member();

        let (pipeline, events) = match CallPipeline::new_for_offer(with_video, turn_servers) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                error!("Could not set up the call: {error}");
                obj.end(CallEndReason::MediaFailed);
                return obj;
            }
        };

        obj.adopt_pipeline(pipeline, events);

        let started = imp
            .pipeline
            .borrow()
            .as_ref()
            .map(CallPipeline::start)
            .transpose();

        if let Err(error) = started {
            error!("Could not start the call: {error}");
            obj.end(CallEndReason::MediaFailed);
            return obj;
        }

        obj.create_local_description(false);
        obj.arm_lifetime_timeout();

        obj
    }

    /// Take note of a call somebody is placing to us.
    ///
    /// The pipeline is not built here. Building it opens the microphone and the
    /// camera, and a call that is only ringing has not been accepted.
    pub(crate) fn receive(
        room: &Room,
        call_id: OwnedVoipId,
        remote_party_id: Option<OwnedVoipId>,
        sender: &UserId,
        content: &CallInviteEventContent,
    ) -> Self {
        let obj = glib::Object::builder::<Self>()
            .property("room", room)
            .property("is-outgoing", false)
            .build();
        let imp = obj.imp();

        let _ = imp.call_id.set(call_id);
        let _ = imp.party_id.set(OwnedVoipId::from(opaque_id(8)));
        imp.remote_party_id.replace(remote_party_id);
        imp.state.set(CallState::Ringing);
        imp.pending_offer.replace(Some(content.offer.clone()));
        imp.has_video.set(
            content.offer.sdp.contains("\r\nm=video ") || content.offer.sdp.contains("\nm=video "),
        );

        obj.set_remote_member_from(sender);
        obj.arm_lifetime_timeout();

        obj
    }

    /// The ID of the call.
    pub(crate) fn call_id(&self) -> &OwnedVoipId {
        self.imp().call_id.get().expect("call ID is set")
    }

    /// Our own party ID.
    fn party_id(&self) -> &OwnedVoipId {
        self.imp().party_id.get().expect("party ID is set")
    }

    /// Whether the given party is the one we are talking to.
    ///
    /// A party is a user and a device, and both halves matter: a user can call
    /// themselves, and two of their devices can answer the same invite.
    fn is_remote_party(&self, sender: &UserId, party_id: Option<&OwnedVoipId>) -> bool {
        let own_user_id = own_user_id(&self.room());

        if *sender == own_user_id && party_id == Some(self.party_id()) {
            // Our own event, echoed back through the sync.
            return false;
        }

        match &*self.imp().remote_party_id.borrow() {
            Some(known) => party_id.is_none_or(|id| id == known),
            None => true,
        }
    }

    /// Answer the call.
    pub(crate) fn accept(&self, turn_servers: &[TurnServer]) {
        let imp = self.imp();

        if imp.state.get() != CallState::Ringing {
            return;
        }

        let Some(offer) = imp.pending_offer.take() else {
            self.end(CallEndReason::Failed);
            return;
        };

        let (pipeline, events) = match CallPipeline::new_for_answer(&offer.sdp, turn_servers) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                error!("Could not set up the call: {error}");
                self.hangup_with(Reason::UserMediaFailed, CallEndReason::MediaFailed);
                return;
            }
        };

        self.adopt_pipeline(pipeline, events);

        let borrowed = imp.pipeline.borrow();
        let Some(pipeline) = &*borrowed else {
            return;
        };

        if let Err(error) = pipeline.set_remote_description(&offer.sdp, false) {
            error!("Could not take the offer: {error}");
            drop(borrowed);
            self.hangup_with(Reason::UnknownError, CallEndReason::Failed);
            return;
        }

        if let Err(error) = pipeline.start() {
            error!("Could not start the call: {error}");
            drop(borrowed);
            self.hangup_with(Reason::UserMediaFailed, CallEndReason::MediaFailed);
            return;
        }

        // Candidates that arrived while it was ringing now have somewhere to go.
        for candidate in imp.pending_candidates.take() {
            pipeline.add_ice_candidate(
                candidate.sdp_m_line_index.map_or(0, u64::from) as u32,
                &candidate.candidate,
            );
        }
        drop(borrowed);

        self.set_state(CallState::Connecting);
        self.cancel_lifetime_timeout();
        self.create_local_description(true);
    }

    /// Decline the call, everywhere.
    ///
    /// This is the loud one: it stops the call ringing on every device of ours
    /// and tells the caller that we said no. Simply closing the window does
    /// neither.
    pub(crate) fn reject(&self) {
        if self.imp().state.get() != CallState::Ringing {
            return;
        }

        let content =
            CallRejectEventContent::version_1(self.call_id().clone(), self.party_id().clone());
        self.send(AnyMessageLikeEventContent::CallReject(content));

        self.end(CallEndReason::Declined);
    }

    /// Refuse the call because another one is already happening.
    ///
    /// The spec has no busy signal of its own; `user_busy` is a hangup reason,
    /// so that is what this is. It goes out without a pipeline ever being
    /// built, which is the point: the microphone is already in use.
    pub(crate) fn decline_as_busy(&self) {
        self.hangup_with(Reason::UserBusy, CallEndReason::HungUp);
    }

    /// End the call.
    pub(crate) fn hangup(&self) {
        self.hangup_with(Reason::UserHangup, CallEndReason::HungUp);
    }

    fn hangup_with(&self, reason: Reason, end_reason: CallEndReason) {
        if self.imp().state.get().is_ended() {
            return;
        }

        let content = CallHangupEventContent::version_1(
            self.call_id().clone(),
            self.party_id().clone(),
            reason,
        );
        self.send(AnyMessageLikeEventContent::CallHangup(content));

        self.end(end_reason);
    }

    /// Take ownership of a pipeline and start listening to it.
    fn adopt_pipeline(
        &self,
        pipeline: CallPipeline,
        mut events: futures_channel::mpsc::UnboundedReceiver<PipelineEvent>,
    ) {
        let imp = self.imp();

        imp.remote_paintable
            .replace(Some(pipeline.remote_paintable().clone()));
        imp.local_paintable
            .replace(pipeline.local_paintable().cloned());
        imp.has_video.set(pipeline.has_video());
        imp.pipeline.replace(Some(pipeline));

        self.notify_remote_paintable();
        self.notify_local_paintable();
        self.notify_has_video();

        spawn!(clone!(
            #[weak(rename_to = obj)]
            self,
            async move {
                while let Some(event) = events.next().await {
                    obj.handle_pipeline_event(event);

                    if obj.state().is_ended() {
                        break;
                    }
                }
            }
        ));
    }

    /// Ask the pipeline for an offer or an answer.
    fn create_local_description(&self, is_answer: bool) {
        let borrowed = self.imp().pipeline.borrow();
        let Some(pipeline) = &*borrowed else {
            return;
        };

        // The pipeline talks back over the same channel it was given, so the
        // description arrives as a `PipelineEvent` like everything else.
        let (sender, mut events) = futures_channel::mpsc::unbounded();

        if is_answer {
            pipeline.create_answer(sender);
        } else {
            pipeline.create_offer(sender);
        }

        spawn!(clone!(
            #[weak(rename_to = obj)]
            self,
            async move {
                while let Some(event) = events.next().await {
                    obj.handle_pipeline_event(event);
                }
            }
        ));
    }

    /// Act on something the pipeline said.
    fn handle_pipeline_event(&self, event: PipelineEvent) {
        match event {
            PipelineEvent::LocalDescription { sdp, is_answer } => {
                self.imp().local_stream_id.replace(first_stream_id(&sdp));

                if is_answer {
                    self.send_answer(sdp);
                } else {
                    self.send_invite(sdp);
                }
            }
            PipelineEvent::IceCandidate {
                candidate,
                sdp_m_line_index,
            } => {
                let mut queued = Candidate::new(candidate);
                queued.sdp_m_line_index = Some(UInt::from(sdp_m_line_index));
                self.queue_candidate(queued);
            }
            PipelineEvent::IceGatheringDone => {
                // An empty candidate is how the spec spells "that is all of
                // them", so that a bridge can stop waiting for more.
                let mut end = Candidate::new(String::new());
                end.sdp_m_line_index = Some(UInt::from(0u32));
                self.queue_candidate(end);
                self.flush_candidates();
            }
            PipelineEvent::Connected => {
                if let Some(pipeline) = &mut *self.imp().pipeline.borrow_mut() {
                    pipeline.note_media();
                }

                if !self.state().is_ended() {
                    self.imp().connected_at.set(
                        glib::DateTime::now_utc()
                            .map(|now| now.to_unix().unsigned_abs())
                            .unwrap_or_default(),
                    );
                    self.notify_connected_at();
                    self.set_state(CallState::Connected);

                    // A mute made while it was still ringing went out to
                    // nobody: the invite or the answer carried the state as it
                    // was when the description was made, and
                    // `send_stream_metadata()` refuses to send before there is
                    // somebody to send to. Say it once now that there is.
                    if self.is_microphone_muted() || self.is_camera_muted() {
                        self.send_stream_metadata();
                    }
                }
            }
            PipelineEvent::ConnectionFailed => {
                let had_media = self
                    .imp()
                    .pipeline
                    .borrow()
                    .as_ref()
                    .is_some_and(CallPipeline::had_media);

                // The spec asks for these two to be told apart: a connection
                // that never came up is `ice_failed`, and one that came up and
                // then died is `ice_timeout`.
                let reason = if had_media {
                    Reason::IceTimeout
                } else {
                    Reason::IceFailed
                };
                self.hangup_with(reason, CallEndReason::NoConnection);
            }
            PipelineEvent::Error(error) => {
                error!("The call pipeline failed: {error}");
                self.hangup_with(Reason::UnknownError, CallEndReason::Failed);
            }
        }
    }

    /// Send the invite that starts an outgoing call.
    fn send_invite(&self, sdp: String) {
        let mut content = CallInviteEventContent::version_1(
            self.call_id().clone(),
            self.party_id().clone(),
            UInt::try_from(INVITE_LIFETIME.as_millis() as u64).unwrap_or(UInt::MAX),
            SessionDescription::new("offer".to_owned(), sdp),
        );

        // A call placed to a room without an invitee is a call anybody in that
        // room may answer. Ours are placed to one person, so say so.
        content.invitee = self.remote_member().map(|member| member.user_id().clone());
        content.sdp_stream_metadata = self.stream_metadata();

        self.send(AnyMessageLikeEventContent::CallInvite(content));
        self.schedule_candidate_batch(CANDIDATE_BATCH_AFTER_INVITE);
    }

    /// Send the answer that accepts an incoming call.
    fn send_answer(&self, sdp: String) {
        let mut content = CallAnswerEventContent::version_1(
            SessionDescription::new("answer".to_owned(), sdp),
            self.call_id().clone(),
            self.party_id().clone(),
        );
        content.sdp_stream_metadata = self.stream_metadata();

        self.send(AnyMessageLikeEventContent::CallAnswer(content));
        self.schedule_candidate_batch(CANDIDATE_BATCH_AFTER_ANSWER);
    }

    /// The metadata for the one stream we send.
    ///
    /// One stream, always `m.usermedia`. Screen sharing would be a second one,
    /// and is not implemented; a client that receives a stream it was not told
    /// about is asked by the spec to ignore it, so sending one silently would
    /// be worse than sending none.
    fn stream_metadata(&self) -> std::collections::BTreeMap<String, StreamMetadata> {
        let Some(stream_id) = self.imp().local_stream_id.borrow().clone() else {
            return Default::default();
        };

        let mut metadata = StreamMetadata::new(StreamPurpose::UserMedia);
        metadata.audio_muted = self.is_microphone_muted();
        metadata.video_muted = self.is_camera_muted();

        [(stream_id, metadata)].into_iter().collect()
    }

    /// Tell the other party that we muted something.
    fn send_stream_metadata(&self) {
        if !matches!(self.state(), CallState::Connecting | CallState::Connected) {
            return;
        }

        let metadata = self.stream_metadata();
        if metadata.is_empty() {
            return;
        }

        let content = CallSdpStreamMetadataChangedEventContent::new(
            self.call_id().clone(),
            self.party_id().clone(),
            voip_version(),
            metadata,
        );
        self.send(AnyMessageLikeEventContent::CallSdpStreamMetadataChanged(
            content,
        ));
    }

    /// Hold on to a candidate until the next batch goes out.
    fn queue_candidate(&self, candidate: Candidate) {
        self.imp().outgoing_candidates.borrow_mut().push(candidate);
    }

    /// Send the candidates gathered so far, if there are any.
    fn flush_candidates(&self) {
        let imp = self.imp();

        if let Some(source) = imp.candidate_batch.take() {
            source.remove();
        }

        let candidates = imp.outgoing_candidates.take();
        if candidates.is_empty() || self.state().is_ended() {
            return;
        }

        let content = CallCandidatesEventContent::version_1(
            self.call_id().clone(),
            self.party_id().clone(),
            candidates,
        );
        self.send(AnyMessageLikeEventContent::CallCandidates(content));
    }

    /// Send the next batch of candidates after the given delay.
    ///
    /// Batching is the spec's ask, and it is worth following: a candidate per
    /// event is a dozen events into the room for one call, all of which the
    /// other end has to sync before it can use any of them.
    fn schedule_candidate_batch(&self, after: Duration) {
        let imp = self.imp();

        if imp.candidate_batch.borrow().is_some() {
            return;
        }

        let source = glib::timeout_add_local_once(
            after,
            clone!(
                #[weak(rename_to = obj)]
                self,
                move || {
                    obj.imp().candidate_batch.take();
                    obj.flush_candidates();
                }
            ),
        );
        imp.candidate_batch.replace(Some(source));
    }

    /// Give up on an invite that nobody answered.
    fn arm_lifetime_timeout(&self) {
        let source = glib::timeout_add_local_once(
            INVITE_LIFETIME,
            clone!(
                #[weak(rename_to = obj)]
                self,
                move || {
                    obj.imp().lifetime_timeout.take();

                    if !obj.state().is_pending() {
                        return;
                    }

                    if obj.is_outgoing() {
                        // The other end never picked up, and the spec has a
                        // reason that says exactly that.
                        obj.hangup_with(Reason::InviteTimeout, CallEndReason::NotAnswered);
                    } else {
                        // An invite we let expire is one we neither answered
                        // nor declined. Sending nothing is the third thing the
                        // spec allows, and it leaves the caller's other
                        // devices ringing.
                        obj.end(CallEndReason::NotAnswered);
                    }
                }
            ),
        );
        self.imp().lifetime_timeout.replace(Some(source));
    }

    fn cancel_lifetime_timeout(&self) {
        if let Some(source) = self.imp().lifetime_timeout.take() {
            source.remove();
        }
    }

    /// Handle an `m.call.answer` from the other end.
    pub(super) fn handle_answer(
        &self,
        sender: &UserId,
        party_id: Option<&OwnedVoipId>,
        answer: &SessionDescription,
    ) {
        if !self.is_outgoing() || self.state() != CallState::Dialing {
            return;
        }
        if !self.is_remote_party(sender, party_id) {
            return;
        }

        let imp = self.imp();

        if imp.answer_selected.get() {
            // Two of their devices answered. The first one won; this one is
            // told so by the `m.call.select_answer` we already sent.
            debug!("Ignoring a second answer to our call");
            return;
        }

        let borrowed = imp.pipeline.borrow();
        let Some(pipeline) = &*borrowed else {
            return;
        };

        if let Err(error) = pipeline.set_remote_description(&answer.sdp, true) {
            error!("Could not take the answer: {error}");
            drop(borrowed);
            self.hangup_with(Reason::UnknownError, CallEndReason::Failed);
            return;
        }
        drop(borrowed);

        if let Some(party_id) = party_id {
            imp.remote_party_id.replace(Some(party_id.clone()));

            // Version 1 asks the caller to say which answer it took, so that
            // the devices that did not win stop ringing.
            let content = CallSelectAnswerEventContent::version_1(
                self.call_id().clone(),
                self.party_id().clone(),
                party_id.clone(),
            );
            self.send(AnyMessageLikeEventContent::CallSelectAnswer(content));
        }

        imp.answer_selected.set(true);
        self.cancel_lifetime_timeout();
        self.set_state(CallState::Connecting);
    }

    /// Handle an `m.call.candidates` from the other end.
    pub(super) fn handle_candidates(
        &self,
        sender: &UserId,
        party_id: Option<&OwnedVoipId>,
        candidates: &[Candidate],
    ) {
        if !self.is_remote_party(sender, party_id) {
            return;
        }

        let imp = self.imp();
        let borrowed = imp.pipeline.borrow();

        let Some(pipeline) = &*borrowed else {
            // Still ringing: keep them for when there is a pipeline. Dropping
            // them would mean the call takes an extra round trip to connect,
            // or does not connect at all.
            imp.pending_candidates
                .borrow_mut()
                .extend_from_slice(candidates);
            return;
        };

        for candidate in candidates {
            pipeline.add_ice_candidate(
                candidate.sdp_m_line_index.map_or(0, u64::from) as u32,
                &candidate.candidate,
            );
        }
    }

    /// Handle an `m.call.select_answer` from the other end.
    pub(super) fn handle_select_answer(&self, sender: &UserId, selected_party_id: &OwnedVoipId) {
        if self.is_outgoing() {
            return;
        }
        // Only the caller sends this, and only about our own answer.
        if *sender == own_user_id(&self.room()) {
            return;
        }

        if selected_party_id != self.party_id() {
            // Another of our devices took the call.
            self.end(CallEndReason::AnsweredElsewhere);
        }
    }

    /// Handle an `m.call.hangup` from the other end.
    pub(super) fn handle_hangup(&self, sender: &UserId, party_id: Option<&OwnedVoipId>) {
        if !self.is_remote_party(sender, party_id) {
            return;
        }

        self.end(CallEndReason::HungUp);
    }

    /// Handle an `m.call.reject` from the other end.
    pub(super) fn handle_reject(&self, sender: &UserId, party_id: &OwnedVoipId) {
        let own_user_id = own_user_id(&self.room());

        if *sender == own_user_id && party_id == self.party_id() {
            return;
        }

        if *sender == own_user_id {
            // We declined it on another device.
            self.end(CallEndReason::AnsweredElsewhere);
            return;
        }

        self.end(CallEndReason::Declined);
    }

    /// Handle an `m.call.sdp_stream_metadata_changed` from the other end.
    pub(super) fn handle_stream_metadata(
        &self,
        sender: &UserId,
        party_id: &OwnedVoipId,
        metadata: &std::collections::BTreeMap<String, StreamMetadata>,
    ) {
        if !self.is_remote_party(sender, Some(party_id)) {
            return;
        }

        // The spec asks that a muted camera be muted locally too, so that the
        // other person sees an avatar rather than the last frame we were sent
        // or a black rectangle. It asks the opposite for audio, because
        // unmuting takes a round trip and the words in between would be lost.
        let video_muted = metadata.values().any(|metadata| metadata.video_muted);

        if self.imp().is_remote_camera_muted.get() != video_muted {
            self.imp().is_remote_camera_muted.set(video_muted);
            self.notify_is_remote_camera_muted();
        }
    }

    /// The other party left the room.
    ///
    /// The spec asks that this be treated as a hangup, which it effectively is:
    /// they cannot send one from outside the room.
    pub(super) fn handle_remote_left(&self) {
        self.end(CallEndReason::HungUp);
    }

    /// Send a call event into the room.
    ///
    /// Straight to the homeserver rather than through the send queue. A queued
    /// invite is one that arrives after the person has stopped waiting, and a
    /// queued hangup is a call the other end thinks is still running.
    fn send(&self, content: AnyMessageLikeEventContent) {
        let matrix_room = self.room().matrix_room().clone();

        spawn!(async move {
            let handle = spawn_tokio!(async move { matrix_room.send(content).await });

            if let Err(error) = handle.await.expect("task was not aborted") {
                warn!("Could not send a call event: {error}");
            }
        });
    }

    /// Set the state, unless the call is already over.
    fn set_state(&self, state: CallState) {
        let imp = self.imp();

        if imp.state.get() == state || imp.state.get().is_ended() {
            return;
        }

        imp.state.set(state);
        self.notify_state();
    }

    /// End the call locally, without telling anybody.
    fn end(&self, reason: CallEndReason) {
        let imp = self.imp();

        if imp.state.get().is_ended() {
            return;
        }

        imp.cancel_timeouts();
        // Dropping the pipeline closes the microphone and the camera.
        imp.pipeline.take();
        imp.pending_offer.take();
        imp.pending_candidates.take();
        imp.outgoing_candidates.take();

        imp.end_reason.set(reason);
        imp.state.set(CallState::Ended);

        self.notify_end_reason();
        self.notify_state();
    }

    /// Find the member on the other end of an incoming call.
    fn set_remote_member_from(&self, user_id: &UserId) {
        let room = self.room();
        let user_id = user_id.to_owned();

        let member = room.get_or_create_members().get_or_create(user_id);

        self.imp().remote_member.replace(Some(member));
        self.notify_remote_member();
    }

    /// Find the one other member of the room we are calling.
    ///
    /// A call is placed to a room, and the room has to have exactly one other
    /// person in it for that to mean anything. The caller checks that before
    /// getting here; this decides whose name is on the window and who the
    /// invite is addressed to.
    fn load_remote_member(&self) {
        let Some(member) = super::other_member(&self.room()) else {
            return;
        };

        self.imp().remote_member.replace(Some(member));
        self.notify_remote_member();
    }
}

/// The ID of the first media stream in an SDP.
///
/// `sdp_stream_metadata` is keyed on the stream ID, which is the first field of
/// an `msid`. There are two places an SDP can carry one, and this reads both
/// because the one the spec's examples show is not the one we write.
///
/// A browser puts it at media level:
///
/// ```text
/// a=msid:<stream id> <track id>
/// ```
///
/// `webrtcbin` writes no such line anywhere. What it writes is the per-source
/// form, once for each `ssrc`:
///
/// ```text
/// a=ssrc:2181993077 msid:user217149580@host-e5ab91bf webrtctransceiver0
/// ```
///
/// Reading only the first form is what left `local_stream_id` empty, and an
/// empty one means `stream_metadata()` returns nothing, and nothing means
/// muting was never announced to the other end at all — the microphone and the
/// camera stopped, and the far side was never told why.
///
/// Both media sections share one stream ID, which is what a single
/// `m.usermedia` stream should look like, so the first one found is the one.
fn first_stream_id(sdp: &str) -> Option<String> {
    sdp.lines()
        .filter_map(stream_id_of_line)
        .find(|stream_id| *stream_id != "-")
        .map(ToOwned::to_owned)
}

/// The stream ID carried by one SDP attribute line, if it carries one.
fn stream_id_of_line(line: &str) -> Option<&str> {
    let attribute = line.trim_end().strip_prefix("a=")?;

    let msid = if let Some(msid) = attribute.strip_prefix("msid:") {
        msid
    } else {
        // `ssrc:<id> msid:<stream id> <track id>`. The same line shape also
        // carries `cname:`, which is not an msid and must not be read as one.
        let (_, rest) = attribute
            .strip_prefix("ssrc:")?
            .split_once(char::is_whitespace)?;
        rest.trim_start().strip_prefix("msid:")?
    };

    msid.split_whitespace().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An offer as `webrtcbin` 1.28.6 actually writes one, trimmed to the
    /// lines that matter here. There is no media-level `a=msid:` anywhere in
    /// it; the msid is an attribute of the source.
    const WEBRTCBIN_OFFER: &str = "\
v=0\r\n\
o=- 8423898717797077664 0 IN IP4 0.0.0.0\r\n\
a=group:BUNDLE audio0 video1\r\n\
m=audio 9 UDP/TLS/RTP/SAVPF 111\r\n\
a=sendrecv\r\n\
a=rtpmap:111 OPUS/48000/2\r\n\
a=ssrc:2181993077 msid:user217149580@host-e5ab91bf webrtctransceiver0\r\n\
a=ssrc:2181993077 cname:user217149580@host-e5ab91bf\r\n\
a=mid:audio0\r\n\
m=video 0 UDP/TLS/RTP/SAVPF 96\r\n\
a=sendrecv\r\n\
a=rtpmap:96 VP8/90000\r\n\
a=ssrc:3666412715 msid:user217149580@host-e5ab91bf webrtctransceiver1\r\n\
a=ssrc:3666412715 cname:user217149580@host-e5ab91bf\r\n\
a=mid:video1\r\n";

    #[test]
    fn the_stream_id_comes_from_the_msid_attribute() {
        let sdp = "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=msid:stream0 track0\r\n";
        assert_eq!(first_stream_id(sdp), Some("stream0".to_owned()));
    }

    #[test]
    fn the_stream_id_is_found_in_what_webrtcbin_writes() {
        // This is the case that was broken: no media-level `a=msid:` line, so
        // the whole of `sdp_stream_metadata` was silently empty and muting was
        // never announced.
        assert_eq!(
            first_stream_id(WEBRTCBIN_OFFER),
            Some("user217149580@host-e5ab91bf".to_owned())
        );
    }

    #[test]
    fn both_sections_of_that_offer_name_one_stream() {
        // One `m.usermedia` stream carrying audio and video, which is what the
        // single entry in `sdp_stream_metadata` is supposed to describe.
        let ids = WEBRTCBIN_OFFER
            .lines()
            .filter_map(stream_id_of_line)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(ids.len(), 1);
    }

    #[test]
    fn a_cname_on_the_same_ssrc_is_not_a_stream_id() {
        // `a=ssrc:N cname:…` has the shape of the line we read and is not one.
        assert_eq!(
            stream_id_of_line("a=ssrc:2181993077 cname:user217149580@host-e5ab91bf"),
            None
        );
    }

    #[test]
    fn a_placeholder_stream_id_is_not_one() {
        // `a=msid:- <track>` means the track belongs to no stream, which is
        // not a thing `sdp_stream_metadata` can be keyed on.
        let sdp = "a=msid:- track0\r\na=msid:stream1 track1\r\n";
        assert_eq!(first_stream_id(sdp), Some("stream1".to_owned()));
    }

    #[test]
    fn an_sdp_without_msid_has_no_stream_id() {
        assert_eq!(first_stream_id("v=0\r\nm=audio 9 RTP/AVP 111\r\n"), None);
    }

    #[test]
    fn an_opaque_id_is_the_length_it_was_asked_for() {
        let id = opaque_id(8);
        assert_eq!(id.len(), 8);
        assert!(id.chars().all(|c| c.is_ascii_alphanumeric()));
    }
}
