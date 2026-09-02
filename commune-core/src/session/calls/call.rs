//! One call, headless.
//!
//! The application's `Call` is a state machine over `m.call.*` signalling
//! and a `webrtcbin` pipeline. The pipeline is the embedder's — the desktop
//! has `GStreamer`, Android has `libwebrtc` — so what is here is the signalling
//! half: the states, the party rule, the invite lifetime, candidate
//! batching, renegotiation, stream metadata in both directions, and every
//! handler for what the other end sends. Where the application hands a
//! description or a candidate to its pipeline, this hands it to the
//! embedder as a [`CallEvent`]; where the pipeline handed the application a
//! description, the embedder calls in with it.

use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use eyeball::{SharedObservable, Subscriber};
use rand::distr::{Alphanumeric, SampleString};
use ruma::{
    OwnedUserId, OwnedVoipId, UInt, UserId, VoipVersionId,
    events::{
        AnyMessageLikeEventContent, MessageLikeEventContent as _,
        call::{
            SessionDescription, StreamMetadata, StreamPurpose,
            answer::CallAnswerEventContent,
            candidates::{CallCandidatesEventContent, Candidate},
            hangup::{CallHangupEventContent, Reason},
            invite::CallInviteEventContent,
            negotiate::CallNegotiateEventContent,
            reject::CallRejectEventContent,
            sdp_stream_metadata_changed::CallSdpStreamMetadataChangedEventContent,
            select_answer::CallSelectAnswerEventContent,
        },
    },
};
use tokio::{sync::broadcast, task::AbortHandle};
use tracing::{debug, warn};

use super::{
    sdp_has_video,
    state::{CallEndReason, CallState},
};
use crate::{RUNTIME, session::Room};

/// How long an invite of ours is valid for.
///
/// The spec's recommended minimum is 90 seconds, on the grounds that the
/// person on the other end needs time to actually pick up.
pub const INVITE_LIFETIME: Duration = Duration::from_secs(90);

/// How long to gather candidates before sending the first batch, after an
/// invite.
///
/// The spec suggests two seconds, since there is a natural pause anyway
/// while the other end decides whether to answer.
const CANDIDATE_BATCH_AFTER_INVITE: Duration = Duration::from_secs(2);

/// How long to gather candidates before sending the first batch, after an
/// answer.
///
/// Half a second: here there is no natural pause, and every one of them is
/// between the two people and hearing each other.
const CANDIDATE_BATCH_AFTER_ANSWER: Duration = Duration::from_millis(500);

/// How long a renegotiation of ours is valid for.
///
/// Shorter than an invite's lifetime, and for the opposite reason: nobody
/// has to decide anything. The call is already up, both ends are at their
/// screens, and an offer that goes unanswered for this long has been lost
/// rather than left to ring.
pub const NEGOTIATE_LIFETIME: Duration = Duration::from_secs(30);

/// The `VoIP` version we speak.
///
/// Version `1` is what `party_id`, `m.call.select_answer`, `m.call.reject`
/// and the `invitee` field all belong to. Version `0` has none of them and
/// no way to tell two answering devices apart.
fn voip_version() -> VoipVersionId {
    VoipVersionId::V1
}

/// Generate an identifier that fits the Opaque Identifier Grammar.
fn opaque_id(length: usize) -> String {
    Alphanumeric.sample_string(&mut rand::rng(), length)
}

/// What the other end sent, for the embedder's WebRTC to act on.
///
/// The application hands these to its pipeline; an embedder without one of
/// its own in the core hands them to whatever it has.
#[derive(Debug, Clone)]
pub enum CallEvent {
    /// The call we placed was answered: apply this description.
    Answer {
        /// The answer.
        sdp: String,
    },
    /// The other end gathered candidates.
    Candidates(Vec<Candidate>),
    /// The other end sent one half of a renegotiation: apply the
    /// description, and if it is an offer, answer it with
    /// [`Call::send_negotiate`].
    Negotiate {
        /// The description.
        sdp: String,
        /// Whether it is the answer to an offer of ours.
        is_answer: bool,
    },
    /// A renegotiation offer of ours crossed theirs and, as the callee,
    /// ours gives way: roll back the local description before applying the
    /// offer that follows.
    RollbackLocalDescription,
}

/// A one-to-one voice or video call.
///
/// Cheap to clone; every clone shares the same state.
#[derive(Debug, Clone)]
pub struct Call {
    inner: Arc<CallInner>,
}

impl PartialEq for Call {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

#[derive(Debug)]
struct CallInner {
    /// The room the call is taking place in.
    room: Room,
    /// The ID of the call, shared by both parties.
    call_id: OwnedVoipId,
    /// Our own party ID, which identifies this device for this call.
    party_id: OwnedVoipId,
    /// The party ID of the other end, once we know it.
    remote_party_id: Mutex<Option<OwnedVoipId>>,
    /// The user on the other end.
    remote_user_id: SharedObservable<Option<OwnedUserId>>,
    /// Whether we placed this call.
    is_outgoing: bool,
    /// Where the call has got to.
    state: SharedObservable<CallState>,
    /// Why the call ended, if it has.
    end_reason: SharedObservable<CallEndReason>,
    /// Whether this call carries video.
    has_video: SharedObservable<bool>,
    /// Whether our own microphone is muted.
    is_microphone_muted: SharedObservable<bool>,
    /// Whether our own camera is muted.
    is_camera_muted: SharedObservable<bool>,
    /// Whether the other party has muted their camera.
    is_remote_camera_muted: SharedObservable<bool>,
    /// Whether the other party has muted their microphone.
    ///
    /// Shown and not acted on: the spec asks that their audio not be muted
    /// locally, since unmuting takes a round trip and the words said in
    /// between would be lost.
    is_remote_microphone_muted: SharedObservable<bool>,
    /// When the call connected, in seconds since the Unix epoch.
    connected_at: SharedObservable<u64>,
    /// The offer of an incoming call, until it is answered.
    pending_offer: Mutex<Option<SessionDescription>>,
    /// Candidates that arrived before the call was answered, when the
    /// embedder has nothing to give them to yet.
    pending_candidates: Mutex<Vec<Candidate>>,
    /// Candidates of our own that have not been sent yet.
    outgoing_candidates: Mutex<Vec<Candidate>>,
    /// The timeout that sends the next batch of candidates.
    candidate_batch: Mutex<Option<AbortHandle>>,
    /// The timeout that gives up on an unanswered invite.
    lifetime_timeout: Mutex<Option<AbortHandle>>,
    /// The timeout that gives up on a renegotiation nobody answered.
    negotiation_timeout: Mutex<Option<AbortHandle>>,
    /// The ID of the stream we send, taken from our own SDP.
    local_stream_id: Mutex<Option<String>>,
    /// Whether we have chosen which answer to use.
    answer_selected: AtomicBool,
    /// Whether a renegotiation offer of ours is waiting for an answer.
    local_offer_pending: AtomicBool,
    /// Whether this call replaced one of ours in a glare and wants
    /// answering at once, as the application answers it.
    wants_immediate_answer: AtomicBool,
    /// What the other end sent, for the embedder.
    events: broadcast::Sender<CallEvent>,
}

impl Drop for CallInner {
    fn drop(&mut self) {
        for slot in [
            &mut self.candidate_batch,
            &mut self.lifetime_timeout,
            &mut self.negotiation_timeout,
        ] {
            if let Ok(Some(handle)) = slot.get_mut().map(Option::take) {
                handle.abort();
            }
        }
    }
}

impl Call {
    fn new(room: &Room, call_id: OwnedVoipId, is_outgoing: bool, state: CallState) -> Self {
        let (events, _) = broadcast::channel(32);

        Self {
            inner: Arc::new(CallInner {
                room: room.clone(),
                call_id,
                party_id: OwnedVoipId::from(opaque_id(8)),
                remote_party_id: Mutex::new(None),
                remote_user_id: SharedObservable::new(None),
                is_outgoing,
                state: SharedObservable::new(state),
                end_reason: SharedObservable::new(CallEndReason::default()),
                has_video: SharedObservable::new(false),
                is_microphone_muted: SharedObservable::new(false),
                is_camera_muted: SharedObservable::new(false),
                is_remote_camera_muted: SharedObservable::new(false),
                is_remote_microphone_muted: SharedObservable::new(false),
                connected_at: SharedObservable::new(0),
                pending_offer: Mutex::new(None),
                pending_candidates: Mutex::new(Vec::new()),
                outgoing_candidates: Mutex::new(Vec::new()),
                candidate_batch: Mutex::new(None),
                lifetime_timeout: Mutex::new(None),
                negotiation_timeout: Mutex::new(None),
                local_stream_id: Mutex::new(None),
                answer_selected: AtomicBool::new(false),
                local_offer_pending: AtomicBool::new(false),
                wants_immediate_answer: AtomicBool::new(false),
                events,
            }),
        }
    }

    /// Place a call in the given room, with the offer the embedder's WebRTC
    /// produced, to the given member.
    ///
    /// The application builds its pipeline first and gets the offer from
    /// it; here the offer comes in, and what the application does with it
    /// follows: the invite goes out and the invite lifetime starts.
    pub(crate) fn place(
        room: &Room,
        offer_sdp: String,
        remote_user_id: Option<OwnedUserId>,
    ) -> Self {
        let call = Self::new(
            room,
            OwnedVoipId::from(opaque_id(16)),
            true,
            CallState::Dialing,
        );
        let inner = &call.inner;

        inner.has_video.set_if_not_eq(sdp_has_video(&offer_sdp));
        inner.remote_user_id.set(remote_user_id);
        *inner.local_stream_id.lock().expect("mutex is not poisoned") = first_stream_id(&offer_sdp);

        call.send_invite(offer_sdp);
        call.arm_lifetime_timeout();

        call
    }

    /// Take note of a call somebody is placing to us.
    ///
    /// Nothing is opened here: a call that is only ringing has not been
    /// accepted, and the embedder's microphone and camera stay closed
    /// until it is.
    pub(crate) fn receive(
        room: &Room,
        call_id: OwnedVoipId,
        remote_party_id: Option<OwnedVoipId>,
        sender: &UserId,
        content: &CallInviteEventContent,
    ) -> Self {
        let call = Self::new(room, call_id, false, CallState::Ringing);
        let inner = &call.inner;

        *inner.remote_party_id.lock().expect("mutex is not poisoned") = remote_party_id;
        *inner.pending_offer.lock().expect("mutex is not poisoned") = Some(content.offer.clone());
        inner
            .has_video
            .set_if_not_eq(sdp_has_video(&content.offer.sdp));
        inner.remote_user_id.set(Some(sender.to_owned()));

        call.apply_stream_metadata(&content.sdp_stream_metadata);
        call.arm_lifetime_timeout();

        call
    }

    /// The room the call is taking place in.
    #[must_use]
    pub fn room(&self) -> &Room {
        &self.inner.room
    }

    /// The ID of the call.
    #[must_use]
    pub fn call_id(&self) -> &OwnedVoipId {
        &self.inner.call_id
    }

    /// Our own party ID.
    pub(crate) fn party_id(&self) -> &OwnedVoipId {
        &self.inner.party_id
    }

    /// The user on the other end, if known.
    #[must_use]
    pub fn remote_user_id(&self) -> Option<OwnedUserId> {
        self.inner.remote_user_id.get()
    }

    /// Whether we placed this call.
    #[must_use]
    pub fn is_outgoing(&self) -> bool {
        self.inner.is_outgoing
    }

    /// Where the call has got to.
    #[must_use]
    pub fn state(&self) -> CallState {
        self.inner.state.get()
    }

    /// Subscribe to where the call has got to.
    pub fn subscribe_state(&self) -> Subscriber<CallState> {
        self.inner.state.subscribe()
    }

    /// Why the call ended, if it has.
    #[must_use]
    pub fn end_reason(&self) -> CallEndReason {
        self.inner.end_reason.get()
    }

    /// Whether this call carries video.
    #[must_use]
    pub fn has_video(&self) -> bool {
        self.inner.has_video.get()
    }

    /// Whether our own microphone is muted.
    #[must_use]
    pub fn is_microphone_muted(&self) -> bool {
        self.inner.is_microphone_muted.get()
    }

    /// Whether our own camera is muted.
    #[must_use]
    pub fn is_camera_muted(&self) -> bool {
        self.inner.is_camera_muted.get()
    }

    /// Whether the other party has muted their camera.
    #[must_use]
    pub fn is_remote_camera_muted(&self) -> bool {
        self.inner.is_remote_camera_muted.get()
    }

    /// Subscribe to whether the other party has muted their camera.
    pub fn subscribe_is_remote_camera_muted(&self) -> Subscriber<bool> {
        self.inner.is_remote_camera_muted.subscribe()
    }

    /// Whether the other party has muted their microphone.
    #[must_use]
    pub fn is_remote_microphone_muted(&self) -> bool {
        self.inner.is_remote_microphone_muted.get()
    }

    /// Subscribe to whether the other party has muted their microphone.
    pub fn subscribe_is_remote_microphone_muted(&self) -> Subscriber<bool> {
        self.inner.is_remote_microphone_muted.subscribe()
    }

    /// When the call connected, in seconds since the Unix epoch, or zero.
    #[must_use]
    pub fn connected_at(&self) -> u64 {
        self.inner.connected_at.get()
    }

    /// Whether this call took over from one of ours in a glare, and so
    /// wants answering at once rather than ringing.
    ///
    /// The application answers it on the spot, which should look to the
    /// user as though the person they called simply picked up.
    #[must_use]
    pub fn wants_immediate_answer(&self) -> bool {
        self.inner.wants_immediate_answer.load(Ordering::SeqCst)
    }

    pub(crate) fn mark_wants_immediate_answer(&self) {
        self.inner
            .wants_immediate_answer
            .store(true, Ordering::SeqCst);
    }

    /// The offer of an incoming call that has not been answered yet.
    #[must_use]
    pub fn pending_offer(&self) -> Option<SessionDescription> {
        self.inner
            .pending_offer
            .lock()
            .expect("mutex is not poisoned")
            .clone()
    }

    /// Subscribe to what the other end sends.
    ///
    /// Only what arrives after subscribing is delivered; the candidates
    /// that arrive while a call rings are replayed when it is accepted.
    #[must_use]
    pub fn subscribe_events(&self) -> broadcast::Receiver<CallEvent> {
        self.inner.events.subscribe()
    }

    /// Whether the given party is the one we are talking to.
    ///
    /// A party is a user and a device, and both halves matter: a user can
    /// call themselves, and two of their devices can answer the same
    /// invite.
    fn is_remote_party(&self, sender: &UserId, party_id: Option<&OwnedVoipId>) -> bool {
        let own_user_id = self.inner.room.matrix_room().own_user_id();

        if sender == own_user_id && party_id == Some(self.party_id()) {
            // Our own event, echoed back through the sync.
            return false;
        }

        match &*self
            .inner
            .remote_party_id
            .lock()
            .expect("mutex is not poisoned")
        {
            Some(known) => party_id.is_none_or(|id| id == known),
            None => true,
        }
    }

    /// Answer the call, with the answer the embedder's WebRTC produced for
    /// the pending offer.
    ///
    /// The candidates that arrived while it rang are handed over now that
    /// there is something to give them to.
    pub fn accept(&self, answer_sdp: String) {
        let inner = &self.inner;

        if inner.state.get() != CallState::Ringing {
            return;
        }

        if inner
            .pending_offer
            .lock()
            .expect("mutex is not poisoned")
            .take()
            .is_none()
        {
            self.end(CallEndReason::Failed);
            return;
        }

        *inner.local_stream_id.lock().expect("mutex is not poisoned") =
            first_stream_id(&answer_sdp);

        self.send_answer(answer_sdp);

        let pending = std::mem::take(
            &mut *inner
                .pending_candidates
                .lock()
                .expect("mutex is not poisoned"),
        );
        if !pending.is_empty() {
            let _ = inner.events.send(CallEvent::Candidates(pending));
        }

        self.set_state(CallState::Connecting);
        self.cancel_lifetime_timeout();
    }

    /// Decline the call, everywhere.
    ///
    /// This is the loud one: it stops the call ringing on every device of
    /// ours and tells the caller that we said no. Simply closing the window
    /// does neither.
    pub fn reject(&self) {
        if self.inner.state.get() != CallState::Ringing {
            return;
        }

        let content =
            CallRejectEventContent::version_1(self.call_id().clone(), self.party_id().clone());
        self.send(AnyMessageLikeEventContent::CallReject(content));

        self.end(CallEndReason::Declined);
    }

    /// Refuse the call because another one is already happening.
    ///
    /// The spec has no busy signal of its own; `user_busy` is a hangup
    /// reason, so that is what this is. It goes out without anything ever
    /// being opened, which is the point: the microphone is already in use.
    pub(crate) fn decline_as_busy(&self) {
        self.hangup_with(Reason::UserBusy, CallEndReason::HungUp);
    }

    /// End the call.
    pub fn hangup(&self) {
        self.hangup_with(Reason::UserHangup, CallEndReason::HungUp);
    }

    /// End the call because the embedder's media failed.
    ///
    /// The application's pipeline reports this itself; an embedder whose
    /// WebRTC lives outside the core says so here.
    pub fn hangup_media_failed(&self) {
        self.hangup_with(Reason::UserMediaFailed, CallEndReason::MediaFailed);
    }

    /// End the call because the two ends could not find a path to each
    /// other.
    ///
    /// The spec asks for these two to be told apart: a connection that
    /// never came up is `ice_failed`, and one that came up and then died is
    /// `ice_timeout`.
    pub fn hangup_no_connection(&self) {
        let had_media = self.inner.state.get() == CallState::Connected;
        let reason = if had_media {
            Reason::IceTimeout
        } else {
            Reason::IceFailed
        };
        self.hangup_with(reason, CallEndReason::NoConnection);
    }

    fn hangup_with(&self, reason: Reason, end_reason: CallEndReason) {
        if self.inner.state.get().is_ended() {
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

    /// Note that media is flowing.
    ///
    /// The application's pipeline reports this itself. Two state machines
    /// can both say connected — the aggregate peer connection state and the
    /// ICE one — so whichever gets there first wins and the second is a
    /// no-op.
    pub fn note_connected(&self) {
        let inner = &self.inner;

        if inner.state.get() == CallState::Connected || inner.state.get().is_ended() {
            return;
        }

        inner.connected_at.set_if_not_eq(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_secs()),
        );
        self.set_state(CallState::Connected);

        // A mute made while it was still ringing went out to nobody: the
        // invite or the answer carried the state as it was when the
        // description was made, and `send_stream_metadata()` refuses to
        // send before there is somebody to send to. Say it once now that
        // there is.
        if self.is_microphone_muted() || self.is_camera_muted() {
            self.send_stream_metadata();
        }
    }

    /// Set whether our own microphone and camera are muted, and tell the
    /// other party.
    pub fn set_muted(&self, microphone_muted: bool, camera_muted: bool) {
        let inner = &self.inner;
        let changed = inner.is_microphone_muted.get() != microphone_muted
            || inner.is_camera_muted.get() != camera_muted;

        inner.is_microphone_muted.set_if_not_eq(microphone_muted);
        inner.is_camera_muted.set_if_not_eq(camera_muted);

        if changed {
            self.send_stream_metadata();
        }
    }

    /// Name the stream we send, when our own SDP did not.
    ///
    /// The stream ID is read from the offer or the answer the embedder
    /// produced; an embedder whose SDP carries no `msid` says it here.
    pub fn set_local_stream_id_if_missing(&self, stream_id: String) {
        let mut local = self
            .inner
            .local_stream_id
            .lock()
            .expect("mutex is not poisoned");
        if local.is_none() && !stream_id.is_empty() {
            *local = Some(stream_id);
        }
    }

    /// Send the invite that starts an outgoing call.
    fn send_invite(&self, sdp: String) {
        let mut content = CallInviteEventContent::version_1(
            self.call_id().clone(),
            self.party_id().clone(),
            UInt::try_from(INVITE_LIFETIME.as_millis()).unwrap_or(UInt::MAX),
            SessionDescription::new("offer".to_owned(), sdp),
        );

        // A call placed to a room without an invitee is a call anybody in
        // that room may answer. Ours are placed to one person, so say so.
        content.invitee = self.remote_user_id();
        content.sdp_stream_metadata = self.stream_metadata();

        // A call that never rings is usually a call addressed to the wrong
        // person: "the invite should be ignored if the invitee is set and
        // doesn't match the user's ID", so a wrong `invitee` is silence
        // rather than an error.
        debug!(
            "Placing call {} in {} to {:?} with {} media section(s)",
            self.call_id(),
            self.room().room_id(),
            content.invitee,
            media_section_count(&content.offer.sdp)
        );

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

    /// Send one half of a renegotiation, with the description the
    /// embedder's WebRTC produced.
    ///
    /// `m.call.negotiate` carries both halves: an offer first, and the
    /// answer to it in an event of the same type. The two are told apart
    /// by the `type` of the description, which is the only thing that says
    /// which one this is. An offer is refused while the call is not
    /// established or while an offer of ours is still waiting for an
    /// answer, as the application refuses to make one then.
    pub fn send_negotiate(&self, sdp: String, is_answer: bool) {
        let inner = &self.inner;

        if !matches!(self.state(), CallState::Connecting | CallState::Connected) {
            return;
        }

        if !is_answer && inner.local_offer_pending.swap(true, Ordering::SeqCst) {
            debug!("A renegotiation of ours is already waiting for an answer");
            return;
        }

        let kind = if is_answer { "answer" } else { "offer" };
        let mut content = CallNegotiateEventContent::version_1(
            self.call_id().clone(),
            self.party_id().clone(),
            UInt::try_from(NEGOTIATE_LIFETIME.as_millis()).unwrap_or(UInt::MAX),
            SessionDescription::new(kind.to_owned(), sdp),
        );
        content.sdp_stream_metadata = self.stream_metadata();

        debug!(
            "Sending a renegotiation {kind} for call {} with {} media section(s)",
            self.call_id(),
            media_section_count(&content.description.sdp)
        );

        self.send(AnyMessageLikeEventContent::CallNegotiate(content));

        // A new section can bring new candidates with it, and nothing else
        // would ever send them: the batch that followed the invite has long
        // since gone out and gathering finished with it.
        self.schedule_candidate_batch(CANDIDATE_BATCH_AFTER_ANSWER);

        if !is_answer {
            self.arm_negotiation_timeout();
        }
    }

    /// Stop waiting for an answer to a renegotiation of ours.
    ///
    /// Without this, a renegotiation the other end never answers leaves the
    /// call unable to attempt another one for as long as it lasts — and the
    /// call itself carries on perfectly well, so nothing else would ever
    /// notice.
    fn arm_negotiation_timeout(&self) {
        let weak = Arc::downgrade(&self.inner);
        let handle = RUNTIME
            .spawn(async move {
                tokio::time::sleep(NEGOTIATE_LIFETIME).await;

                let Some(inner) = weak.upgrade() else {
                    return;
                };
                inner
                    .negotiation_timeout
                    .lock()
                    .expect("mutex is not poisoned")
                    .take();

                if inner.local_offer_pending.swap(false, Ordering::SeqCst) {
                    warn!("A renegotiation of ours went unanswered");
                }
            })
            .abort_handle();

        if let Some(previous) = self
            .inner
            .negotiation_timeout
            .lock()
            .expect("mutex is not poisoned")
            .replace(handle)
        {
            previous.abort();
        }
    }

    /// The metadata for the one stream we send.
    ///
    /// One stream, always `m.usermedia`. Screen sharing would be a second
    /// one, and is not implemented; a client that receives a stream it was
    /// not told about is asked by the spec to ignore it, so sending one
    /// silently would be worse than sending none.
    fn stream_metadata(&self) -> BTreeMap<String, StreamMetadata> {
        let Some(stream_id) = self
            .inner
            .local_stream_id
            .lock()
            .expect("mutex is not poisoned")
            .clone()
        else {
            return BTreeMap::default();
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

        debug!(
            "Telling the other party that our microphone is {} and our camera is {}",
            if self.is_microphone_muted() {
                "muted"
            } else {
                "live"
            },
            if self.is_camera_muted() { "off" } else { "on" },
        );

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

    /// Hold on to candidates of our own until the next batch goes out.
    ///
    /// Both spellings ride along, because the spec asks for one of the two
    /// and clients in the wild want the mid: a candidate handed to
    /// libwebrtc as `IceCandidate(null, index, line)` is one the far end
    /// can drop without saying anything.
    pub fn add_local_candidates(&self, candidates: Vec<Candidate>) {
        self.inner
            .outgoing_candidates
            .lock()
            .expect("mutex is not poisoned")
            .extend(candidates);
    }

    /// Note that our own gathering is done, and send what is left.
    ///
    /// An empty candidate is how the spec spells "that is all of them", so
    /// that a bridge can stop waiting for more. Neither field is required
    /// for it, and the index is sent anyway because a client that insists
    /// on one gets one.
    pub fn local_gathering_done(&self) {
        let mut end = Candidate::new(String::new());
        end.sdp_m_line_index = Some(UInt::from(0u32));
        self.add_local_candidates(vec![end]);
        self.flush_candidates();
    }

    /// Send the candidates gathered so far, if there are any.
    fn flush_candidates(&self) {
        let inner = &self.inner;

        if let Some(handle) = inner
            .candidate_batch
            .lock()
            .expect("mutex is not poisoned")
            .take()
        {
            handle.abort();
        }

        let candidates = std::mem::take(
            &mut *inner
                .outgoing_candidates
                .lock()
                .expect("mutex is not poisoned"),
        );
        if candidates.is_empty() || self.state().is_ended() {
            return;
        }

        // The m-line indices matter as much as the count: with `max-bundle`
        // every candidate belongs to the first section, and one that says
        // otherwise lands on a `bundle-only` section with no transport of
        // its own.
        debug!(
            "{}: sending {} ICE candidate(s) for call {} on m-line(s) {:?}",
            inner.room.matrix_room().own_user_id(),
            candidates.len(),
            self.call_id(),
            candidates
                .iter()
                .map(|c| c.sdp_m_line_index.map_or(0, u64::from))
                .collect::<std::collections::BTreeSet<_>>(),
        );

        let content = CallCandidatesEventContent::version_1(
            self.call_id().clone(),
            self.party_id().clone(),
            candidates,
        );
        self.send(AnyMessageLikeEventContent::CallCandidates(content));
    }

    /// Send the next batch of candidates after the given delay.
    ///
    /// Batching is the spec's ask, and it is worth following: a candidate
    /// per event is a dozen events into the room for one call, all of which
    /// the other end has to sync before it can use any of them.
    fn schedule_candidate_batch(&self, after: Duration) {
        let mut slot = self
            .inner
            .candidate_batch
            .lock()
            .expect("mutex is not poisoned");
        if slot.is_some() {
            return;
        }

        let weak = Arc::downgrade(&self.inner);
        let handle = RUNTIME
            .spawn(async move {
                tokio::time::sleep(after).await;

                let Some(inner) = weak.upgrade() else {
                    return;
                };
                inner
                    .candidate_batch
                    .lock()
                    .expect("mutex is not poisoned")
                    .take();
                Call { inner }.flush_candidates();
            })
            .abort_handle();
        *slot = Some(handle);
    }

    /// Give up on an invite that nobody answered.
    fn arm_lifetime_timeout(&self) {
        let weak = Arc::downgrade(&self.inner);
        let handle = RUNTIME
            .spawn(async move {
                tokio::time::sleep(INVITE_LIFETIME).await;

                let Some(inner) = weak.upgrade() else {
                    return;
                };
                inner
                    .lifetime_timeout
                    .lock()
                    .expect("mutex is not poisoned")
                    .take();
                let call = Call { inner };

                if !call.state().is_pending() {
                    return;
                }

                if call.is_outgoing() {
                    // The other end never picked up, and the spec has a
                    // reason that says exactly that.
                    call.hangup_with(Reason::InviteTimeout, CallEndReason::NotAnswered);
                } else {
                    // An invite we let expire is one we neither answered
                    // nor declined. Sending nothing is the third thing the
                    // spec allows, and it leaves the caller's other devices
                    // ringing.
                    call.end(CallEndReason::NotAnswered);
                }
            })
            .abort_handle();

        if let Some(previous) = self
            .inner
            .lifetime_timeout
            .lock()
            .expect("mutex is not poisoned")
            .replace(handle)
        {
            previous.abort();
        }
    }

    fn cancel_lifetime_timeout(&self) {
        if let Some(handle) = self
            .inner
            .lifetime_timeout
            .lock()
            .expect("mutex is not poisoned")
            .take()
        {
            handle.abort();
        }
    }

    /// Stop every timeout.
    fn cancel_timeouts(&self) {
        let inner = &self.inner;
        for slot in [
            &inner.candidate_batch,
            &inner.lifetime_timeout,
            &inner.negotiation_timeout,
        ] {
            if let Some(handle) = slot.lock().expect("mutex is not poisoned").take() {
                handle.abort();
            }
        }
    }

    /// Handle an `m.call.answer` from the other end.
    pub(crate) fn handle_answer(
        &self,
        sender: &UserId,
        party_id: Option<&OwnedVoipId>,
        content: &CallAnswerEventContent,
    ) {
        if !self.is_outgoing() || self.state() != CallState::Dialing {
            return;
        }
        if !self.is_remote_party(sender, party_id) {
            return;
        }

        let inner = &self.inner;
        let answer = &content.answer;

        // What they say about their own microphone and camera arrives with
        // the answer, before either of them has been muted.
        self.apply_stream_metadata(&content.sdp_stream_metadata);

        debug!(
            "Answer for call {} from party {:?}",
            self.call_id(),
            party_id
        );

        if inner.answer_selected.swap(true, Ordering::SeqCst) {
            // Two of their devices answered. The first one won; this one
            // is told so by the `m.call.select_answer` we already sent.
            debug!("Ignoring a second answer to our call");
            return;
        }

        let _ = inner.events.send(CallEvent::Answer {
            sdp: answer.sdp.clone(),
        });

        if let Some(party_id) = party_id {
            *inner.remote_party_id.lock().expect("mutex is not poisoned") = Some(party_id.clone());

            // Version 1 asks the caller to say which answer it took, so
            // that the devices that did not win stop ringing.
            let content = CallSelectAnswerEventContent::version_1(
                self.call_id().clone(),
                self.party_id().clone(),
                party_id.clone(),
            );
            self.send(AnyMessageLikeEventContent::CallSelectAnswer(content));
        }

        self.cancel_lifetime_timeout();
        self.set_state(CallState::Connecting);
    }

    /// Handle an `m.call.negotiate` from the other end.
    ///
    /// A call that is already up, described again: this is what adding
    /// video to a voice call, putting a call on hold or restarting ICE
    /// looks like from the other end. Both halves of it are this event —
    /// an offer first, then an answer of the same type — and which half
    /// this is comes from the `type` of the description.
    pub(crate) fn handle_negotiate(
        &self,
        sender: &UserId,
        party_id: &OwnedVoipId,
        content: &CallNegotiateEventContent,
    ) {
        if !self.is_remote_party(sender, Some(party_id)) {
            return;
        }

        if !matches!(self.state(), CallState::Connecting | CallState::Connected) {
            // "This event is sent by either party after the call is
            // established": before that there is a description in flight
            // already, and applying a second one on top of it is how a
            // call that was about to connect stops.
            debug!("Ignoring a renegotiation of a call that is not established yet");
            return;
        }

        self.apply_stream_metadata(&content.sdp_stream_metadata);

        let inner = &self.inner;
        let is_answer = content.description.session_type == "answer";

        debug!(
            "Renegotiation {} for call {} from party {party_id}",
            content.description.session_type,
            self.call_id()
        );

        if is_answer {
            if !inner.local_offer_pending.swap(false, Ordering::SeqCst) {
                debug!("Ignoring a renegotiation answer to an offer that is not ours");
                return;
            }

            if let Some(handle) = inner
                .negotiation_timeout
                .lock()
                .expect("mutex is not poisoned")
                .take()
            {
                handle.abort();
            }

            // A renegotiation that fails is not a call that fails: what
            // was flowing before it is still flowing, and the embedder
            // applying this is asked to leave the call alone if it cannot.
            let _ = inner.events.send(CallEvent::Negotiate {
                sdp: content.description.sdp.clone(),
                is_answer: true,
            });
            return;
        }

        // An offer, and possibly one that crossed an offer of ours. Perfect
        // negotiation settles that without either end asking the other:
        // "the callee is always the polite party", and the polite party is
        // the one that gives way.
        if inner.local_offer_pending.load(Ordering::SeqCst) {
            if self.is_outgoing() {
                debug!("A renegotiation offer crossed ours; as the caller, ours stands");
                return;
            }

            debug!("A renegotiation offer crossed ours; as the callee, ours gives way");
            let _ = inner.events.send(CallEvent::RollbackLocalDescription);
            inner.local_offer_pending.store(false, Ordering::SeqCst);
        }

        let _ = inner.events.send(CallEvent::Negotiate {
            sdp: content.description.sdp.clone(),
            is_answer: false,
        });
    }

    /// Handle an `m.call.candidates` from the other end.
    pub(crate) fn handle_candidates(
        &self,
        sender: &UserId,
        party_id: Option<&OwnedVoipId>,
        candidates: &[Candidate],
    ) {
        if !self.is_remote_party(sender, party_id) {
            // Silently dropping these is indistinguishable from none
            // arriving.
            debug!(
                "Ignoring {} ICE candidate(s) from a party we are not talking to",
                candidates.len()
            );
            return;
        }

        let inner = &self.inner;

        if self.state() == CallState::Ringing {
            // Still ringing: keep them for when there is something to give
            // them to. Dropping them would mean the call takes an extra
            // round trip to connect, or does not connect at all.
            debug!(
                "Holding {} ICE candidate(s) until the call is answered",
                candidates.len()
            );
            inner
                .pending_candidates
                .lock()
                .expect("mutex is not poisoned")
                .extend_from_slice(candidates);
            return;
        }

        debug!(
            "{}: received {} ICE candidate(s) for call {} from party {:?} on m-line(s) {:?}",
            inner.room.matrix_room().own_user_id(),
            candidates.len(),
            self.call_id(),
            party_id,
            candidates
                .iter()
                .map(|c| c.sdp_m_line_index.map_or(0, u64::from))
                .collect::<std::collections::BTreeSet<_>>(),
        );

        let _ = inner
            .events
            .send(CallEvent::Candidates(candidates.to_vec()));
    }

    /// Handle an `m.call.select_answer` from the other end.
    pub(crate) fn handle_select_answer(&self, sender: &UserId, selected_party_id: &OwnedVoipId) {
        if self.is_outgoing() {
            return;
        }
        // Only the caller sends this, and only about our own answer.
        if sender == self.inner.room.matrix_room().own_user_id() {
            return;
        }

        if selected_party_id != self.party_id() {
            // Another of our devices took the call.
            self.end(CallEndReason::AnsweredElsewhere);
        }
    }

    /// Handle an `m.call.hangup` from the other end.
    pub(crate) fn handle_hangup(&self, sender: &UserId, party_id: Option<&OwnedVoipId>) {
        if !self.is_remote_party(sender, party_id) {
            return;
        }

        self.end(CallEndReason::HungUp);
    }

    /// Handle an `m.call.reject` from the other end.
    pub(crate) fn handle_reject(&self, sender: &UserId, party_id: &OwnedVoipId) {
        let own_user_id = self.inner.room.matrix_room().own_user_id();

        if sender == own_user_id && party_id == self.party_id() {
            return;
        }

        if sender == own_user_id {
            // We declined it on another device.
            self.end(CallEndReason::AnsweredElsewhere);
            return;
        }

        self.end(CallEndReason::Declined);
    }

    /// Handle an `m.call.sdp_stream_metadata_changed` from the other end.
    pub(crate) fn handle_stream_metadata(
        &self,
        sender: &UserId,
        party_id: &OwnedVoipId,
        metadata: &BTreeMap<String, StreamMetadata>,
    ) {
        if !self.is_remote_party(sender, Some(party_id)) {
            return;
        }

        self.apply_stream_metadata(metadata);
    }

    /// Take what the other end says about the streams it is sending.
    ///
    /// Four events carry this and all four mean the same thing: the
    /// invite, the answer, a renegotiation, and the one whose only job is
    /// to carry it when nothing else has to be negotiated.
    fn apply_stream_metadata(&self, metadata: &BTreeMap<String, StreamMetadata>) {
        // Logged including the empty case, because "they muted and we did
        // not notice" and "they never said anything" are the same silence
        // from here, and only one of them is ours to fix.
        debug!(
            "The other party describes {} stream(s): {:?}",
            metadata.len(),
            metadata
                .iter()
                .map(|(id, stream)| (
                    id.as_str(),
                    stream.purpose.as_str(),
                    stream.audio_muted,
                    stream.video_muted
                ))
                .collect::<Vec<_>>()
        );

        if metadata.is_empty() {
            // "For backwards compatibility, if `sdp_stream_metadata` is not
            // present ... the client should assume that this property is
            // not supported by the other party." Not "everything is
            // unmuted": a client that never sends this has told us nothing,
            // and forgetting what an earlier event said would be inventing
            // an answer.
            return;
        }

        // "If a stream has a `purpose` of an unknown type, it should also
        // be ignored." Screen sharing is the other purpose the spec names
        // and is not implemented here, so the only stream this client has
        // an opinion about is the one with the person in it.
        let mut video_muted = false;
        let mut audio_muted = false;

        for stream in metadata
            .values()
            .filter(|stream| stream.purpose == StreamPurpose::UserMedia)
        {
            video_muted |= stream.video_muted;
            audio_muted |= stream.audio_muted;
        }

        // The spec asks that a muted camera be muted locally too, so that
        // the other person sees an avatar rather than the last frame we
        // were sent or a black rectangle. It asks the opposite for audio,
        // because unmuting takes a round trip and the words in between
        // would be lost — so their microphone is shown and not acted on.
        self.inner.is_remote_camera_muted.set_if_not_eq(video_muted);
        self.inner
            .is_remote_microphone_muted
            .set_if_not_eq(audio_muted);
    }

    /// The other party left the room.
    ///
    /// The spec asks that this be treated as a hangup, which it effectively
    /// is: they cannot send one from outside the room.
    pub(crate) fn handle_remote_left(&self) {
        self.end(CallEndReason::HungUp);
    }

    /// Send a call event into the room.
    ///
    /// Straight to the homeserver rather than through the send queue. A
    /// queued invite is one that arrives after the person has stopped
    /// waiting, and a queued hangup is a call the other end thinks is still
    /// running.
    fn send(&self, content: AnyMessageLikeEventContent) {
        let matrix_room = self.inner.room.matrix_room().clone();
        let room_id = matrix_room.room_id().to_owned();
        let event_type = content.event_type().to_string();

        RUNTIME.spawn(async move {
            // The event ID, because a call that rings for nobody is a call
            // whose events have to be looked for in the room, and this is
            // the only place their IDs exist.
            match matrix_room.send(content).await {
                Ok(result) => debug!(
                    "Sent {event_type} into {room_id} as {}{}",
                    result.response.event_id,
                    if result.encryption_info.is_some() {
                        ", encrypted"
                    } else {
                        ""
                    }
                ),
                Err(error) => warn!("Could not send a call event: {error}"),
            }
        });
    }

    /// Set the state, unless the call is already over.
    fn set_state(&self, state: CallState) {
        let inner = &self.inner;

        if inner.state.get() == state || inner.state.get().is_ended() {
            return;
        }

        inner.state.set(state);
    }

    /// End the call locally, without telling anybody.
    fn end(&self, reason: CallEndReason) {
        let inner = &self.inner;

        if inner.state.get().is_ended() {
            return;
        }

        self.cancel_timeouts();
        inner
            .pending_offer
            .lock()
            .expect("mutex is not poisoned")
            .take();
        inner
            .pending_candidates
            .lock()
            .expect("mutex is not poisoned")
            .clear();
        inner
            .outgoing_candidates
            .lock()
            .expect("mutex is not poisoned")
            .clear();

        inner.end_reason.set_if_not_eq(reason);
        inner.state.set(CallState::Ended);
    }
}

/// How many media sections an SDP has, for the log.
fn media_section_count(sdp: &str) -> usize {
    sdp.matches("\r\nm=").count() + usize::from(sdp.starts_with("m="))
}

/// The ID of the first media stream in an SDP.
///
/// `sdp_stream_metadata` is keyed on the stream ID, which is the first
/// field of an `msid`. There are two places an SDP can carry one, and this
/// reads both because the one the spec's examples show is not the one
/// `webrtcbin` writes.
///
/// A browser puts it at media level:
///
/// ```text
/// a=msid:<stream id> <track id>
/// ```
///
/// `webrtcbin` writes no such line anywhere. What it writes is the
/// per-source form, once for each `ssrc`:
///
/// ```text
/// a=ssrc:2181993077 msid:user217149580@host-e5ab91bf webrtctransceiver0
/// ```
///
/// Reading only the first form is what left `local_stream_id` empty, and
/// an empty one means `stream_metadata()` returns nothing, and nothing
/// means muting was never announced to the other end at all — the
/// microphone and the camera stopped, and the far side was never told why.
///
/// Both media sections share one stream ID, which is what a single
/// `m.usermedia` stream should look like, so the first one found is the
/// one.
#[must_use]
pub fn first_stream_id(sdp: &str) -> Option<String> {
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
        // `ssrc:<id> msid:<stream id> <track id>`. The same line shape
        // also carries `cname:`, which is not an msid and must not be read
        // as one.
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
    /// lines that matter here. There is no media-level `a=msid:` anywhere
    /// in it; the msid is an attribute of the source.
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
        // This is the case that was broken: no media-level `a=msid:` line,
        // so the whole of `sdp_stream_metadata` was silently empty and
        // muting was never announced.
        assert_eq!(
            first_stream_id(WEBRTCBIN_OFFER),
            Some("user217149580@host-e5ab91bf".to_owned())
        );
    }

    #[test]
    fn both_sections_of_that_offer_name_one_stream() {
        // One `m.usermedia` stream carrying audio and video, which is what
        // the single entry in `sdp_stream_metadata` is supposed to
        // describe.
        let ids = WEBRTCBIN_OFFER
            .lines()
            .filter_map(stream_id_of_line)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(ids.len(), 1);
    }

    #[test]
    fn a_cname_on_the_same_ssrc_is_not_a_stream_id() {
        // `a=ssrc:N cname:…` has the shape of the line we read and is not
        // one.
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

    #[test]
    fn media_sections_are_counted() {
        assert_eq!(media_section_count(WEBRTCBIN_OFFER), 2);
        assert_eq!(media_section_count("m=audio 9 RTP/AVP 111\r\n"), 1);
        assert_eq!(media_section_count("v=0\r\n"), 0);
    }
}
