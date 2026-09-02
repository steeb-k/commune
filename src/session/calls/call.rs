use std::time::Duration;

use commune_core::session::{Call as CoreCall, CallEvent, Calls as CoreCalls};
use futures_util::StreamExt;
use gtk::{gdk, glib, glib::clone, prelude::*, subclass::prelude::*};
use ruma::{OwnedUserId, OwnedVoipId, UInt, events::call::candidates::Candidate};
use tokio::{sync::broadcast::error::RecvError, task::AbortHandle};
use tracing::{debug, error, warn};

use super::{
    IceServers,
    pipeline::{CallPipeline, PipelineEvent},
    state::{CallEndReason, CallState},
};
use crate::{
    core_bridge::ObjectWatcher,
    session::{Member, Room},
    spawn, spawn_tokio,
};

/// How long a call that reports ICE failure is given to recover.
///
/// libnice says `Failed` the moment it has no pair left to check, which on a
/// trickling call is a statement about this instant and not about the call:
/// the other end sends its candidates in batches, over the room, and a batch
/// that lands afterwards can still make the call. Hanging up on the first one
/// is how a call that was five seconds from connecting gets ended for the
/// person waiting on it.
const ICE_FAILURE_GRACE: Duration = Duration::from_secs(15);

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::Call)]
    pub struct Call {
        /// The room the call is taking place in.
        #[property(get, construct_only)]
        pub(super) room: OnceCell<Room>,
        /// The call, as the core runs it.
        ///
        /// A call we place has none until the pipeline has produced the
        /// offer the core places it with.
        pub(super) core: RefCell<Option<CoreCall>>,
        /// The ID of the call, shared by both parties.
        #[property(get = Self::call_id_string)]
        call_id_string: std::marker::PhantomData<String>,
        pub(super) call_id: RefCell<Option<OwnedVoipId>>,
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
        /// Whether the other party has muted their microphone.
        ///
        /// Shown and not acted on: the spec asks that their audio not be muted
        /// locally, since unmuting takes a round trip and the words said in
        /// between would be lost.
        #[property(get)]
        pub(super) is_remote_microphone_muted: Cell<bool>,
        /// Whether the other party's video is actually being drawn.
        ///
        /// Not the same as [`Self::has_video`], which is about the sections
        /// this call negotiated. A video call whose other end has no camera
        /// negotiates video and never carries any.
        #[property(get)]
        pub(super) has_remote_video: Cell<bool>,
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
        /// Candidates of our own gathered before the core had the call.
        pub(super) early_local_candidates: RefCell<Vec<Candidate>>,
        /// Whether our own gathering finished before the core had the call.
        pub(super) early_gathering_done: Cell<bool>,
        /// The timeout that gives up on a call ICE says has failed.
        pub(super) ice_failure_timeout: RefCell<Option<glib::SourceId>>,
        /// The task following the core's call.
        pub(super) watch_handle: RefCell<Option<AbortHandle>>,
        /// The task carrying what the other end sends to the pipeline.
        pub(super) events_handle: RefCell<Option<AbortHandle>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Call {
        const NAME: &'static str = "Call";
        type Type = super::Call;
    }

    #[glib::derived_properties]
    impl ObjectImpl for Call {
        fn dispose(&self) {
            self.cancel_ice_failure_timeout();
            if let Some(handle) = self.watch_handle.take() {
                handle.abort();
            }
            if let Some(handle) = self.events_handle.take() {
                handle.abort();
            }
        }
    }

    impl Call {
        /// The ID of the call, as a string.
        fn call_id_string(&self) -> String {
            self.call_id
                .borrow()
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default()
        }

        /// The call, as the core runs it, if it has it yet.
        pub(super) fn core(&self) -> Option<CoreCall> {
            self.core.borrow().clone()
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
            self.tell_core_muted();
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
            self.tell_core_muted();
        }

        /// Tell the core what we muted, so that it tells the other party.
        pub(super) fn tell_core_muted(&self) {
            if let Some(core) = self.core() {
                core.set_muted(self.is_microphone_muted.get(), self.is_camera_muted.get());
            }
        }

        /// Stop waiting for ICE to recover.
        pub(super) fn cancel_ice_failure_timeout(&self) {
            if let Some(source) = self.ice_failure_timeout.take() {
                source.remove();
            }
        }

        /// Mirror where the core says the call has got to.
        pub(super) fn set_state(&self, state: CallState) {
            if self.state.get() == state {
                return;
            }

            self.state.set(state);

            if state.is_ended() {
                self.cancel_ice_failure_timeout();
                // Dropping the pipeline closes the microphone and the camera.
                self.pipeline.take();
                self.early_local_candidates.take();
            }

            self.obj().notify_state();
        }

        /// Mirror why the call ended.
        pub(super) fn set_end_reason(&self, reason: CallEndReason) {
            if self.end_reason.get() == reason {
                return;
            }

            self.end_reason.set(reason);
            self.obj().notify_end_reason();
        }

        /// Mirror whether the call carries video.
        pub(super) fn set_has_video(&self, has_video: bool) {
            if self.has_video.get() == has_video {
                return;
            }

            self.has_video.set(has_video);
            self.obj().notify_has_video();
        }

        /// Mirror whether the other party muted their camera.
        pub(super) fn set_remote_camera_muted(&self, muted: bool) {
            if self.is_remote_camera_muted.get() == muted {
                return;
            }

            self.is_remote_camera_muted.set(muted);
            self.obj().notify_is_remote_camera_muted();
        }

        /// Mirror whether the other party muted their microphone.
        pub(super) fn set_remote_microphone_muted(&self, muted: bool) {
            if self.is_remote_microphone_muted.get() == muted {
                return;
            }

            self.is_remote_microphone_muted.set(muted);
            self.obj().notify_is_remote_microphone_muted();
        }

        /// Mirror when the call connected.
        pub(super) fn set_connected_at(&self, connected_at: u64) {
            if self.connected_at.get() == connected_at {
                return;
            }

            self.connected_at.set(connected_at);
            self.obj().notify_connected_at();
        }

        /// Present the given user as the other end.
        pub(super) fn set_remote_user(&self, user_id: Option<OwnedUserId>) {
            let member = user_id.map(|user_id| {
                let room = self.room.get().expect("room is set");
                room.get_or_create_members().get_or_create(user_id)
            });

            if self.remote_member.borrow().as_ref() == member.as_ref() {
                return;
            }

            self.remote_member.replace(member);
            self.obj().notify_remote_member();
        }
    }
}

glib::wrapper! {
    /// A one-to-one voice or video call.
    ///
    /// The signalling is the core's; this drives the `webrtcbin` pipeline
    /// with what the core says arrived, and hands the core what the
    /// pipeline produced.
    pub struct Call(ObjectSubclass<imp::Call>);
}

impl Call {
    /// Place a call in the given room.
    ///
    /// The pipeline is built and asked for an offer; the core places the
    /// call with that offer when it arrives.
    pub(crate) fn place(room: &Room, with_video: bool, servers: &IceServers) -> Self {
        let obj = glib::Object::builder::<Self>()
            .property("room", room)
            .property("is-outgoing", true)
            .build();
        let imp = obj.imp();

        imp.state.set(CallState::Dialing);
        imp.has_video.set(with_video);

        obj.load_remote_member();

        let (pipeline, events) = match CallPipeline::new_for_offer(with_video, servers) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                error!("Could not set up the call: {error}");
                obj.end_locally(CallEndReason::MediaFailed);
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
            obj.end_locally(CallEndReason::MediaFailed);
            return obj;
        }

        obj.create_local_description(false);

        obj
    }

    /// Present a call the core has: one somebody is placing to us, or one
    /// that took over from ours in a glare.
    ///
    /// The pipeline is not built here. Building it opens the microphone and
    /// the camera, and a call that is only ringing has not been accepted.
    pub(super) fn from_core(room: &Room, core: &CoreCall) -> Self {
        let obj = glib::Object::builder::<Self>()
            .property("room", room)
            .property("is-outgoing", core.is_outgoing())
            .build();

        obj.attach(core);

        obj
    }

    /// Whether this presents the given call of the core.
    pub(super) fn presents(&self, core: &CoreCall) -> bool {
        self.imp().core().as_ref() == Some(core)
    }

    /// Follow the given call of the core, and carry what the other end
    /// sends to the pipeline.
    fn attach(&self, core: &CoreCall) {
        type C = Call;

        let imp = self.imp();
        imp.core.replace(Some(core.clone()));
        imp.call_id.replace(Some(core.call_id().clone()));
        self.notify_call_id_string();

        let handle = ObjectWatcher::new(self)
            .follow(core.subscribe_state(), |obj: &C, state| {
                obj.imp().set_state(state.into());
            })
            .follow(core.subscribe_end_reason(), |obj: &C, reason| {
                obj.imp().set_end_reason(reason.into());
            })
            .follow(core.subscribe_has_video(), |obj: &C, has_video| {
                obj.imp().set_has_video(has_video);
            })
            .follow(core.subscribe_is_remote_camera_muted(), |obj: &C, muted| {
                obj.imp().set_remote_camera_muted(muted);
            })
            .follow(
                core.subscribe_is_remote_microphone_muted(),
                |obj: &C, muted| {
                    obj.imp().set_remote_microphone_muted(muted);
                },
            )
            .follow(core.subscribe_connected_at(), |obj: &C, connected_at| {
                obj.imp().set_connected_at(connected_at);
            })
            .follow(core.subscribe_remote_user_id(), |obj: &C, user_id| {
                obj.imp().set_remote_user(user_id);
            })
            .spawn();
        imp.watch_handle.replace(Some(handle));

        // What the other end sends, carried to the pipeline on the main
        // thread. A receiver that fell behind skips what it missed: a
        // candidate lost this way is one the next batch does not carry
        // again, so the buffer is sized for a call and this is the
        // fallback.
        let obj_weak = glib::SendWeakRef::from(self.downgrade());
        let mut receiver = core.subscribe_events();
        let events_handle = spawn_tokio!(async move {
            loop {
                let event = match receiver.recv().await {
                    Ok(event) => event,
                    Err(RecvError::Lagged(missed)) => {
                        warn!("Missed {missed} call event(s) from the core");
                        continue;
                    }
                    Err(RecvError::Closed) => break,
                };

                let obj_weak = obj_weak.clone();
                let ctx = glib::MainContext::default();
                ctx.spawn(async move {
                    if let Some(obj) = obj_weak.upgrade() {
                        obj.handle_call_event(event);
                    }
                });
            }
        })
        .abort_handle();
        imp.events_handle.replace(Some(events_handle));

        // What the core already knows, after subscribing so that nothing
        // between the two is lost.
        imp.set_remote_user(core.remote_user_id());
        imp.set_has_video(core.has_video());
        imp.set_remote_camera_muted(core.is_remote_camera_muted());
        imp.set_remote_microphone_muted(core.is_remote_microphone_muted());
        imp.set_connected_at(core.connected_at());
        imp.set_end_reason(core.end_reason().into());
        imp.set_state(core.state().into());

        // What the pipeline gathered while the core had no call yet.
        let early = imp.early_local_candidates.take();
        if !early.is_empty() {
            core.add_local_candidates(early);
        }
        if imp.early_gathering_done.replace(false) {
            core.local_gathering_done();
        }

        // A mute made before the core had the call.
        imp.tell_core_muted();
    }

    /// The ID of the call, once the core has it.
    pub(crate) fn call_id(&self) -> Option<OwnedVoipId> {
        self.imp().call_id.borrow().clone()
    }

    /// Answer the call.
    pub(crate) fn accept(&self, servers: &IceServers) {
        let imp = self.imp();

        if imp.state.get() != CallState::Ringing {
            return;
        }

        let Some(core) = imp.core() else {
            return;
        };

        let Some(offer) = core.pending_offer() else {
            core.hangup_failed();
            return;
        };

        let (pipeline, events) = match CallPipeline::new_for_answer(&offer.sdp, servers) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                error!("Could not set up the call: {error}");
                core.hangup_media_failed();
                return;
            }
        };

        self.adopt_pipeline(pipeline, events);

        let borrowed = imp.pipeline.borrow();
        let Some(pipeline) = &*borrowed else {
            return;
        };

        if let Err(error) = pipeline.start() {
            error!("Could not start the call: {error}");
            drop(borrowed);
            core.hangup_media_failed();
            return;
        }

        // Take the offer and answer it, in that order and not merely in that
        // sequence: `set-remote-description` is asynchronous, and answering an
        // offer that has not been applied yet gets an empty answer back — which
        // is never sent, so the caller waits until its invite expires. The
        // answer is created from inside the description's promise, and the
        // core accepts the call with it when it arrives.
        let (sender, mut events) = futures_channel::mpsc::unbounded();

        if let Err(error) = pipeline.answer_remote_offer(&offer.sdp, sender) {
            error!("Could not take the offer: {error}");
            drop(borrowed);
            core.hangup_failed();
            return;
        }
        drop(borrowed);

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

    /// Decline the call, everywhere.
    ///
    /// This is the loud one: it stops the call ringing on every device of ours
    /// and tells the caller that we said no. Simply closing the window does
    /// neither.
    pub(crate) fn reject(&self) {
        if let Some(core) = self.imp().core() {
            core.reject();
        }
    }

    /// End the call.
    pub(crate) fn hangup(&self) {
        match self.imp().core() {
            Some(core) => core.hangup(),
            // The core never had it: nothing was sent, so there is nothing
            // to hang up but the microphone.
            None => self.end_locally(CallEndReason::HungUp),
        }
    }

    /// Turn the camera on partway through a call placed without one.
    ///
    /// The one thing in this client that asks for a renegotiation: the camera
    /// goes into the pipeline, `webrtcbin` notices that what it sends no
    /// longer matches what it last offered, and the offer that follows leaves
    /// as an `m.call.negotiate`. Nothing here writes SDP.
    ///
    /// Returns whether the camera was added.
    pub(crate) fn add_video(&self) -> bool {
        let imp = self.imp();

        if self.has_video() || self.state() != CallState::Connected {
            return false;
        }

        let mut borrowed = imp.pipeline.borrow_mut();
        let Some(pipeline) = &mut *borrowed else {
            return false;
        };

        if let Err(error) = pipeline.enable_video() {
            warn!("Could not add the camera to the call: {error}");
            return false;
        }

        imp.local_paintable
            .replace(pipeline.local_paintable().cloned());
        imp.has_video.set(true);
        drop(borrowed);

        self.notify_local_paintable();
        self.notify_has_video();

        true
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

    /// Act on something the other end sent, as the core relays it.
    fn handle_call_event(&self, event: CallEvent) {
        let imp = self.imp();
        let borrowed = imp.pipeline.borrow();
        let Some(pipeline) = &*borrowed else {
            return;
        };

        match event {
            CallEvent::Answer { sdp } => {
                if let Err(error) = pipeline.set_remote_description(&sdp, true) {
                    error!("Could not take the answer: {error}");
                    drop(borrowed);
                    if let Some(core) = imp.core() {
                        core.hangup_failed();
                    }
                }
            }
            CallEvent::Candidates(candidates) => {
                for candidate in &candidates {
                    pipeline.add_ice_candidate(
                        candidate.sdp_m_line_index.map_or(0, u64::from) as u32,
                        &candidate.candidate,
                    );
                }
            }
            CallEvent::Negotiate {
                sdp,
                is_answer: true,
            } => {
                if let Err(error) = pipeline.set_remote_description(&sdp, true) {
                    // A renegotiation that fails is not a call that fails. What
                    // was flowing before it is still flowing, and hanging up
                    // would take away a working call over a camera that could
                    // not be added.
                    warn!("Could not take the renegotiation answer: {error}");
                }
            }
            CallEvent::Negotiate {
                sdp,
                is_answer: false,
            } => {
                let (event_sender, mut events) = futures_channel::mpsc::unbounded();

                if let Err(error) = pipeline.answer_remote_offer(&sdp, event_sender) {
                    warn!("Could not take the renegotiation offer: {error}");
                    return;
                }
                drop(borrowed);

                spawn!(clone!(
                    #[weak(rename_to = obj)]
                    self,
                    async move {
                        while let Some(event) = events.next().await {
                            match event {
                                PipelineEvent::LocalDescription { sdp, .. } => {
                                    if let Some(core) = obj.imp().core() {
                                        core.send_negotiate(sdp, true);
                                    }
                                }
                                other => obj.handle_pipeline_event(other),
                            }
                        }
                    }
                ));
            }
            CallEvent::RollbackLocalDescription => pipeline.rollback_local_description(),
        }
    }

    /// Act on something the pipeline said.
    fn handle_pipeline_event(&self, event: PipelineEvent) {
        let imp = self.imp();

        match event {
            PipelineEvent::LocalDescription { sdp, is_answer } => {
                if is_answer {
                    if let Some(core) = imp.core() {
                        core.accept(sdp);
                    }
                } else {
                    self.place_with_offer(sdp);
                }
            }
            PipelineEvent::IceCandidate {
                candidate,
                sdp_m_line_index,
                sdp_mid,
            } => {
                // Both, because the spec asks for one of the two and clients
                // in the wild want the mid: a candidate handed to libwebrtc as
                // `IceCandidate(null, index, line)` is one the far end can
                // drop without saying anything, and a peer with no remote
                // candidates never sends a check and never installs a relay
                // permission for us — which looks exactly like a network that
                // will not carry the call.
                let mut queued = Candidate::new(candidate);
                queued.sdp_m_line_index = Some(UInt::from(sdp_m_line_index));
                queued.sdp_mid = sdp_mid;

                match imp.core() {
                    Some(core) => core.add_local_candidates(vec![queued]),
                    None => imp.early_local_candidates.borrow_mut().push(queued),
                }
            }
            PipelineEvent::IceGatheringDone => match imp.core() {
                Some(core) => core.local_gathering_done(),
                None => imp.early_gathering_done.set(true),
            },
            PipelineEvent::Connected => {
                if let Some(pipeline) = &mut *imp.pipeline.borrow_mut() {
                    pipeline.note_media();
                    // From here on, `webrtcbin` asking for a negotiation is a
                    // renegotiation, and this client answers those.
                    pipeline.arm_renegotiation();
                }

                // A failure it recovered from, which is the whole reason the
                // first one is not acted on.
                imp.cancel_ice_failure_timeout();

                if let Some(core) = imp.core() {
                    core.note_connected();
                }
            }
            PipelineEvent::NegotiationNeeded => self.send_renegotiation_offer(),
            PipelineEvent::RemoteVideo => {
                if !imp.has_remote_video.replace(true) {
                    self.notify_has_remote_video();
                }
            }
            PipelineEvent::ConnectionFailed => self.handle_connection_failed(),
            PipelineEvent::Error(error) => {
                error!("The call pipeline failed: {error}");
                match imp.core() {
                    Some(core) => core.hangup_failed(),
                    None => self.end_locally(CallEndReason::Failed),
                }
            }
        }
    }

    /// Hand the core the offer the pipeline produced, which places the call.
    fn place_with_offer(&self, sdp: String) {
        let room = self.room();
        let Some(session) = room.session() else {
            self.end_locally(CallEndReason::Failed);
            return;
        };
        let core_calls: CoreCalls = session.core().calls().clone();
        let core_room = room.core().clone();

        spawn!(clone!(
            #[weak(rename_to = obj)]
            self,
            async move {
                let placed = spawn_tokio!(async move { core_calls.place(&core_room, sdp).await })
                    .await
                    .expect("task was not aborted");

                match placed {
                    Ok(core) => {
                        if obj.state().is_ended() {
                            // Hung up before the offer was ready.
                            core.hangup();
                            return;
                        }
                        obj.attach(&core);
                    }
                    Err(error) => {
                        error!("Could not place the call: {error}");
                        obj.end_locally(CallEndReason::Failed);
                    }
                }
            }
        ));
    }

    /// Act on ICE reporting that it has failed.
    ///
    /// A call that carried media and then stopped is over: something that was
    /// working has died, and there is nothing to wait for. A call that never
    /// connected is a different thing — `Failed` there means only that there
    /// is nothing left to check *yet*, and the other end trickles its
    /// candidates through the room at its own pace. So that one is given
    /// [`ICE_FAILURE_GRACE`] to recover before the person is told it will not.
    fn handle_connection_failed(&self) {
        let imp = self.imp();
        let had_media = imp
            .pipeline
            .borrow()
            .as_ref()
            .is_some_and(CallPipeline::had_media);

        if had_media {
            if let Some(core) = imp.core() {
                core.hangup_no_connection();
            }
            return;
        }

        if imp.ice_failure_timeout.borrow().is_some() {
            // Already waiting. Both state machines report a failure, and the
            // second one is not a second failure.
            return;
        }

        debug!("ICE has nothing left to check; waiting to see if more arrives");

        let source = glib::timeout_add_local_once(
            ICE_FAILURE_GRACE,
            clone!(
                #[weak(rename_to = obj)]
                self,
                move || {
                    obj.imp().ice_failure_timeout.take();

                    if obj.state() == CallState::Connected || obj.state().is_ended() {
                        return;
                    }

                    match obj.imp().core() {
                        Some(core) => core.hangup_no_connection(),
                        None => obj.end_locally(CallEndReason::NoConnection),
                    }
                }
            ),
        );
        imp.ice_failure_timeout.replace(Some(source));
    }

    /// Offer the other end a new session description.
    ///
    /// Called when `webrtcbin` says the session no longer matches what it last
    /// described, which in this client means the camera was turned on midway
    /// through a voice call.
    fn send_renegotiation_offer(&self) {
        let imp = self.imp();

        let Some(core) = imp.core() else {
            return;
        };

        if !core.can_send_negotiate_offer() {
            debug!("A renegotiation of ours is already waiting for an answer");
            return;
        }

        let borrowed = imp.pipeline.borrow();
        let Some(pipeline) = &*borrowed else {
            return;
        };

        // A dedicated channel, because the description that comes back is
        // neither an invite nor an answer and the general handler would send
        // it as one.
        let (event_sender, mut events) = futures_channel::mpsc::unbounded();
        pipeline.create_offer(event_sender);
        drop(borrowed);

        spawn!(clone!(
            #[weak(rename_to = obj)]
            self,
            async move {
                while let Some(event) = events.next().await {
                    match event {
                        PipelineEvent::LocalDescription { sdp, .. } => {
                            if let Some(core) = obj.imp().core() {
                                core.send_negotiate(sdp, false);
                            }
                        }
                        other => obj.handle_pipeline_event(other),
                    }
                }
            }
        ));
    }

    /// End the call here, without telling anybody.
    ///
    /// For a call the core never had: nothing was sent about it, so there
    /// is nothing to send about its end. A call the core has ends through
    /// the core.
    fn end_locally(&self, reason: CallEndReason) {
        let imp = self.imp();

        if imp.state.get().is_ended() {
            return;
        }

        imp.set_end_reason(reason);
        imp.set_state(CallState::Ended);
    }

    /// Find the one other member of the room we are calling.
    ///
    /// A call is placed to a room, and the room has to have exactly one other
    /// person in it for that to mean anything. The caller checks that before
    /// getting here; this decides whose name is on the window until the core
    /// has the call and says who it went to.
    fn load_remote_member(&self) {
        let Some(member) = super::other_member(&self.room()) else {
            return;
        };

        self.imp().remote_member.replace(Some(member));
        self.notify_remote_member();
    }
}
