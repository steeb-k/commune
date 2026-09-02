//! One-to-one voice and video calls.
//!
//! This is the Voice over IP module of the Client-Server API: `m.call.*`
//! events carrying WebRTC signalling between exactly two devices. Group
//! calls are a different thing entirely — `MatrixRTC`, still a proposal —
//! and none of this is about them.
//!
//! The application's `Calls` object headless: the eight event handlers,
//! the one active call, the TURN credentials kept until they go stale, the
//! candidates that arrive before their invite, the outcomes the timeline's
//! rows are drawn from, and every rule about which invites ring. The
//! ringtone and the notification are the embedder's; so is the media.

use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use eyeball::{SharedObservable, Subscriber};
use matrix_sdk::{event_handler::EventHandlerDropGuard, room::Room as MatrixRoom};
use ruma::{
    Int, OwnedRoomId, OwnedUserId, OwnedVoipId, UInt, UserId,
    events::call::{
        answer::OriginalSyncCallAnswerEvent,
        candidates::OriginalSyncCallCandidatesEvent,
        hangup::{OriginalSyncCallHangupEvent, Reason},
        invite::OriginalSyncCallInviteEvent,
        negotiate::OriginalSyncCallNegotiateEvent,
        reject::OriginalSyncCallRejectEvent,
        sdp_stream_metadata_changed::OriginalSyncCallSdpStreamMetadataChangedEvent,
        select_answer::OriginalSyncCallSelectAnswerEvent,
    },
    serde::Raw,
};
use tokio::{
    sync::{broadcast, mpsc},
    task::AbortHandle,
};
use tracing::debug;

mod call;
mod state;
mod turn;

pub use self::{
    call::{Call, CallEvent, INVITE_LIFETIME, NEGOTIATE_LIFETIME, first_stream_id},
    state::{CallEndReason, CallOutcome, CallState},
    turn::{IceServers, RawTurnCredentials, TurnCredentials, TurnServer, load_turn_credentials},
};
use super::{JoinRuleValue, Membership, Room, WeakSession};
use crate::{RUNTIME, UserFacingError};

/// How long a candidate batch is kept for an invite that has not arrived.
///
/// One sync round trip is all it takes; this is generous so that a slow
/// one still lands the candidates rather than losing them.
const EARLY_CANDIDATE_LIFETIME: Duration = Duration::from_secs(30);

/// How many such batches are kept at once.
///
/// A bound rather than a number that matters: the batches are small, and
/// everything in here is either claimed by an invite within a sync or two,
/// or belongs to a call this session is not in.
const MAX_EARLY_CANDIDATE_BATCHES: usize = 8;

/// How many calls are remembered for the sake of the rows in the timeline.
///
/// What is remembered is one enum per call, so the number is a bound
/// rather than a budget: it is there so that a session left running for a
/// week does not keep every call the account ever saw.
const MAX_REMEMBERED_OUTCOMES: usize = 256;

/// An error encountered while placing or answering a call.
#[derive(Debug, thiserror::Error)]
pub enum CallError {
    /// There is already a call.
    #[error("another call is in progress")]
    AlreadyInCall,
    /// The room is not one a call can be placed in.
    #[error("a call cannot be placed in this room")]
    CannotCall,
    /// There is no call with the given ID.
    #[error("there is no such call")]
    NoSuchCall,
}

impl UserFacingError for CallError {
    fn to_user_facing(&self) -> String {
        match self {
            Self::AlreadyInCall => "Another call is in progress".to_owned(),
            Self::CannotCall => "A call cannot be placed in this room".to_owned(),
            Self::NoSuchCall => "There is no such call".to_owned(),
        }
    }
}

/// Everything that arrives about a call.
///
/// One enum rather than seven handlers on the far side, so that the hop
/// off the sync happens once and in one place.
#[derive(Debug)]
enum CallSignal {
    Invite(Box<OriginalSyncCallInviteEvent>),
    Answer(Box<OriginalSyncCallAnswerEvent>),
    Candidates(Box<OriginalSyncCallCandidatesEvent>),
    Hangup(Box<OriginalSyncCallHangupEvent>),
    Reject(Box<OriginalSyncCallRejectEvent>),
    SelectAnswer(Box<OriginalSyncCallSelectAnswerEvent>),
    StreamMetadata(Box<OriginalSyncCallSdpStreamMetadataChangedEvent>),
    Negotiate(Box<OriginalSyncCallNegotiateEvent>),
}

/// A candidate batch that arrived before the invite it belongs to.
///
/// The other end sends its candidates immediately after the invite, and
/// the two can reach us the other way round — the invite takes longer to
/// send when it is the first encrypted event of a session, and the sync
/// that carries the candidates gets here first. Dropping them costs the
/// call: for a peer that gathers before it dials, that batch is every
/// candidate it will ever send.
#[derive(Debug)]
struct EarlyCandidates {
    /// The room the batch arrived in.
    room_id: OwnedRoomId,
    /// The batch.
    event: OriginalSyncCallCandidatesEvent,
    /// When it arrived, so that it can be forgotten.
    received: Instant,
}

/// The calls of a session.
///
/// Cheap to clone; every clone shares the same state.
#[derive(Debug, Clone)]
pub struct Calls {
    inner: Arc<CallsInner>,
}

#[derive(Debug)]
struct CallsInner {
    /// The current session.
    session: WeakSession,
    /// The call that is happening, if there is one.
    ///
    /// There is at most one. Two calls at once means two microphones and
    /// two sets of speakers, and no way to say which one a hangup was for.
    active_call: SharedObservable<Option<Call>>,
    /// The TURN credentials, kept until they go stale.
    turn_credentials: tokio::sync::Mutex<TurnCredentials>,
    /// Candidates that arrived before the invite they belong to.
    early_candidates: Mutex<Vec<EarlyCandidates>>,
    /// What became of the calls this session has seen.
    ///
    /// Keyed by call ID, which is what the invite in the timeline carries.
    /// Nothing here is written to disk: a client that was not running when
    /// a call happened has no way to know what became of it, and saying so
    /// is better than guessing.
    outcomes: Mutex<HashMap<OwnedVoipId, CallOutcome>>,
    /// The order the outcomes were first noted in, so that the oldest can
    /// be forgotten.
    outcome_order: Mutex<VecDeque<OwnedVoipId>>,
    /// The call ID whose outcome changed, so that a row can tell whether
    /// the change is about the call it is showing.
    outcome_changed: broadcast::Sender<OwnedVoipId>,
    /// The guards of the event handlers.
    handler_guards: Mutex<Vec<EventHandlerDropGuard>>,
    /// The task acting on the signals.
    signal_task: Mutex<Option<AbortHandle>>,
    /// The task forgetting the active call when it ends.
    end_watch: Mutex<Option<AbortHandle>>,
    /// Whether the handlers are installed.
    initialized: AtomicBool,
}

impl Drop for CallsInner {
    fn drop(&mut self) {
        for slot in [&mut self.signal_task, &mut self.end_watch] {
            if let Ok(Some(handle)) = slot.get_mut().map(Option::take) {
                handle.abort();
            }
        }
    }
}

impl Calls {
    /// Construct the calls of the given session.
    pub(crate) fn new(session: WeakSession) -> Self {
        let (outcome_changed, _) = broadcast::channel(32);

        Self {
            inner: Arc::new(CallsInner {
                session,
                active_call: SharedObservable::new(None),
                turn_credentials: tokio::sync::Mutex::new(TurnCredentials::default()),
                early_candidates: Mutex::new(Vec::new()),
                outcomes: Mutex::new(HashMap::new()),
                outcome_order: Mutex::new(VecDeque::new()),
                outcome_changed,
                handler_guards: Mutex::new(Vec::new()),
                signal_task: Mutex::new(None),
                end_watch: Mutex::new(None),
                initialized: AtomicBool::new(false),
            }),
        }
    }

    /// Start listening for call signalling.
    pub(crate) fn init(&self) {
        let inner = &self.inner;

        if inner.initialized.swap(true, Ordering::SeqCst) {
            return;
        }

        let Some(session) = inner.session.upgrade() else {
            return;
        };

        let client = session.client();
        let (sender, mut receiver) = mpsc::unbounded_channel::<(OwnedRoomId, CallSignal)>();
        let mut guards = Vec::new();

        macro_rules! handle {
            ($event:ty, $variant:ident) => {{
                let sender = sender.clone();
                let handle = client.add_event_handler(move |event: $event, room: MatrixRoom| {
                    let sender = sender.clone();
                    async move {
                        let _ = sender.send((
                            room.room_id().to_owned(),
                            CallSignal::$variant(Box::new(event)),
                        ));
                    }
                });
                guards.push(client.event_handler_drop_guard(handle));
            }};
        }

        handle!(OriginalSyncCallInviteEvent, Invite);
        handle!(OriginalSyncCallAnswerEvent, Answer);
        handle!(OriginalSyncCallCandidatesEvent, Candidates);
        handle!(OriginalSyncCallHangupEvent, Hangup);
        handle!(OriginalSyncCallRejectEvent, Reject);
        handle!(OriginalSyncCallSelectAnswerEvent, SelectAnswer);
        handle!(
            OriginalSyncCallSdpStreamMetadataChangedEvent,
            StreamMetadata
        );
        handle!(OriginalSyncCallNegotiateEvent, Negotiate);

        // What the other end's invite and answer actually carried, when
        // what they carried did not include stream metadata. See
        // [`note_missing_stream_metadata`].
        let handle =
            client.add_event_handler(move |event: Raw<OriginalSyncCallInviteEvent>| async move {
                note_missing_stream_metadata("invite", &event);
            });
        guards.push(client.event_handler_drop_guard(handle));
        let handle =
            client.add_event_handler(move |event: Raw<OriginalSyncCallAnswerEvent>| async move {
                note_missing_stream_metadata("answer", &event);
            });
        guards.push(client.event_handler_drop_guard(handle));

        *inner.handler_guards.lock().expect("mutex is not poisoned") = guards;

        let weak = Arc::downgrade(inner);
        let task = RUNTIME
            .spawn(async move {
                while let Some((room_id, signal)) = receiver.recv().await {
                    let Some(inner) = weak.upgrade() else {
                        break;
                    };
                    Calls { inner }.handle_signal(&room_id, signal);
                }
            })
            .abort_handle();
        *inner.signal_task.lock().expect("mutex is not poisoned") = Some(task);
    }

    /// Place a call in the given room, with the offer the embedder's
    /// WebRTC produced.
    ///
    /// Refused when there is already a call, or when the room is not one a
    /// call can be placed in.
    pub async fn place(&self, room: &Room, offer_sdp: String) -> Result<Call, CallError> {
        if self.active_call().is_some() {
            debug!("Refusing to place a second call");
            return Err(CallError::AlreadyInCall);
        }

        room.permissions().ensure_loaded().await;
        if !can_call(room) {
            return Err(CallError::CannotCall);
        }

        let remote_user_id = other_member(room).await;
        let call = Call::place(room, offer_sdp, remote_user_id);
        self.set_active_call(Some(call.clone()));

        Ok(call)
    }

    /// Answer the call with the given ID, with the answer the embedder's
    /// WebRTC produced for its offer.
    pub fn accept(&self, call_id: &str, answer_sdp: String) -> Result<(), CallError> {
        let call = self.call_with_id(call_id).ok_or(CallError::NoSuchCall)?;
        call.accept(answer_sdp);
        Ok(())
    }

    /// The ICE servers to use, asking the homeserver if what we have is
    /// stale.
    pub async fn turn_servers(&self) -> IceServers {
        self.ensure_turn_credentials().await.servers().clone()
    }

    /// The homeserver's TURN answer as it gave it, asking again if what we
    /// have is stale.
    pub async fn turn_credentials_raw(&self) -> Option<RawTurnCredentials> {
        self.ensure_turn_credentials().await.raw().cloned()
    }

    /// The TURN credentials, refreshed if stale.
    async fn ensure_turn_credentials(&self) -> tokio::sync::MutexGuard<'_, TurnCredentials> {
        let mut credentials = self.inner.turn_credentials.lock().await;

        if !credentials.is_fresh()
            && let Some(session) = self.inner.session.upgrade()
        {
            *credentials = load_turn_credentials(&session.client()).await;
        }

        credentials
    }

    /// The call that is happening, if there is one.
    #[must_use]
    pub fn active_call(&self) -> Option<Call> {
        self.inner.active_call.get()
    }

    /// Subscribe to the call that is happening.
    pub fn subscribe_active_call(&self) -> Subscriber<Option<Call>> {
        self.inner.active_call.subscribe()
    }

    /// Set the call that is happening, and forget it when it ends.
    fn set_active_call(&self, call: Option<Call>) {
        let inner = &self.inner;
        inner.active_call.set(call.clone());

        if let Some(previous) = inner
            .end_watch
            .lock()
            .expect("mutex is not poisoned")
            .take()
        {
            previous.abort();
        }

        let Some(call) = call else {
            return;
        };

        // Candidates for it may already be here, an invite having taken
        // longer to arrive than the batch that followed it.
        self.replay_early_candidates(&call);

        let weak = Arc::downgrade(inner);
        let watched = call.clone();
        let handle = RUNTIME
            .spawn(async move {
                let mut states = watched.subscribe_state();
                while !watched.state().is_ended() {
                    if states.next().await.is_none() {
                        break;
                    }
                }

                let Some(inner) = weak.upgrade() else {
                    return;
                };
                let is_this_call = inner
                    .active_call
                    .get()
                    .is_some_and(|active| active.call_id() == watched.call_id());
                if is_this_call {
                    inner.active_call.set(None);
                }
            })
            .abort_handle();
        *inner.end_watch.lock().expect("mutex is not poisoned") = Some(handle);
    }

    /// Keep a candidate batch whose call has not appeared.
    ///
    /// Measured on 23 August 2026: an incoming call arrived with its whole
    /// batch of twenty-four candidates one sync ahead of the invite, all
    /// of them dropped here, and the call that followed had not one remote
    /// candidate to pair with. The ring worked; nothing after it could.
    fn hold_early_candidates(&self, room_id: &OwnedRoomId, event: OriginalSyncCallCandidatesEvent) {
        debug!(
            "Holding {} ICE candidate(s) for call {}, which has not been invited yet",
            event.content.candidates.len(),
            event.content.call_id
        );

        let now = Instant::now();
        let mut held = self
            .inner
            .early_candidates
            .lock()
            .expect("mutex is not poisoned");

        // A batch nothing claimed belongs to a call this session is not in.
        held.retain(|batch| now.duration_since(batch.received) < EARLY_CANDIDATE_LIFETIME);

        if held.len() >= MAX_EARLY_CANDIDATE_BATCHES {
            held.remove(0);
        }

        held.push(EarlyCandidates {
            room_id: room_id.clone(),
            event,
            received: now,
        });
    }

    /// Hand a call the candidates that arrived before it did.
    fn replay_early_candidates(&self, call: &Call) {
        let room_id = call.room().room_id();
        let call_id = call.call_id();

        let claimed = {
            let mut held = self
                .inner
                .early_candidates
                .lock()
                .expect("mutex is not poisoned");
            let mut claimed = Vec::new();
            let mut rest = Vec::new();

            for batch in held.drain(..) {
                if batch.room_id == *room_id && batch.event.content.call_id == *call_id {
                    claimed.push(batch.event);
                } else {
                    rest.push(batch);
                }
            }

            *held = rest;
            claimed
        };

        for event in claimed {
            debug!(
                "Replaying {} ICE candidate(s) that arrived before call {call_id}",
                event.content.candidates.len()
            );
            call.handle_candidates(
                &event.sender,
                event.content.party_id.as_ref(),
                &event.content.candidates,
            );
        }
    }

    /// The call with the given ID, if it is the one that is happening.
    #[must_use]
    pub fn call_with_id(&self, call_id: &str) -> Option<Call> {
        self.active_call()
            .filter(|call| call.call_id().as_str() == call_id)
    }

    /// What became of the call with the given ID, if this session saw it.
    #[must_use]
    pub fn outcome(&self, call_id: &str) -> Option<CallOutcome> {
        self.inner
            .outcomes
            .lock()
            .expect("mutex is not poisoned")
            .get(&OwnedVoipId::from(call_id.to_owned()))
            .copied()
    }

    /// What became of every call this session saw.
    #[must_use]
    pub fn outcomes_snapshot(&self) -> HashMap<OwnedVoipId, CallOutcome> {
        self.inner
            .outcomes
            .lock()
            .expect("mutex is not poisoned")
            .clone()
    }

    /// Subscribe to the outcome of a call changing, by call ID.
    #[must_use]
    pub fn subscribe_outcome_changed(&self) -> broadcast::Receiver<OwnedVoipId> {
        self.inner.outcome_changed.subscribe()
    }

    /// Note what has happened to a call.
    ///
    /// Every call the room shows, not only the one this client is in: a
    /// call answered on another device is one the timeline still has to
    /// describe, and the events that say so arrive here either way.
    fn note_outcome(&self, call_id: &OwnedVoipId, outcome: CallOutcome) {
        let inner = &self.inner;
        let mut outcomes = inner.outcomes.lock().expect("mutex is not poisoned");
        let previous = outcomes.get(call_id).copied();

        let outcome = merge_outcome(previous, outcome);

        if previous == Some(outcome) {
            return;
        }

        if previous.is_none() {
            let mut order = inner.outcome_order.lock().expect("mutex is not poisoned");
            order.push_back(call_id.clone());

            while order.len() > MAX_REMEMBERED_OUTCOMES {
                if let Some(forgotten) = order.pop_front() {
                    outcomes.remove(&forgotten);
                }
            }
        }

        outcomes.insert(call_id.clone(), outcome);
        drop(outcomes);

        let _ = inner.outcome_changed.send(call_id.clone());
    }

    /// Act on a call event.
    fn handle_signal(&self, room_id: &OwnedRoomId, signal: CallSignal) {
        // What the timeline says about a call afterwards is built here,
        // from the same events the call itself is made of.
        match &signal {
            CallSignal::Invite(event) => {
                self.note_outcome(&event.content.call_id, CallOutcome::Ringing);
            }
            CallSignal::Answer(event) => {
                self.note_outcome(&event.content.call_id, CallOutcome::Answered);
            }
            CallSignal::SelectAnswer(event) => {
                self.note_outcome(&event.content.call_id, CallOutcome::Answered);
            }
            CallSignal::Reject(event) => {
                self.note_outcome(&event.content.call_id, CallOutcome::Declined);
            }
            CallSignal::Hangup(event) => {
                // A hangup is only ever the end of the call; whether anybody
                // answered it first is what [`Self::note_outcome`] keeps.
                // The one reason that says something on its own is the busy
                // signal, which is a refusal spelled as a hangup.
                let outcome = if event.content.reason == Reason::UserBusy {
                    CallOutcome::Declined
                } else {
                    CallOutcome::Missed
                };
                self.note_outcome(&event.content.call_id, outcome);
            }
            CallSignal::Candidates(_)
            | CallSignal::StreamMetadata(_)
            | CallSignal::Negotiate(_) => {}
        }

        match signal {
            CallSignal::Invite(event) => self.handle_invite(room_id, &event),
            CallSignal::Answer(event) => {
                if let Some(call) = self.call_with_id(event.content.call_id.as_str()) {
                    call.handle_answer(
                        &event.sender,
                        event.content.party_id.as_ref(),
                        &event.content,
                    );
                }
            }
            CallSignal::Candidates(event) => {
                if let Some(call) = self.call_with_id(event.content.call_id.as_str()) {
                    call.handle_candidates(
                        &event.sender,
                        event.content.party_id.as_ref(),
                        &event.content.candidates,
                    );
                } else {
                    // Not a call this session is in — or not one it is in
                    // *yet*, the invite being still on its way. The two
                    // look identical here, so the batch is kept for the
                    // invite that may be about to claim it.
                    self.hold_early_candidates(room_id, *event);
                }
            }
            CallSignal::Hangup(event) => {
                if let Some(call) = self.call_with_id(event.content.call_id.as_str()) {
                    call.handle_hangup(&event.sender, event.content.party_id.as_ref());
                }
            }
            CallSignal::Reject(event) => {
                if let Some(call) = self.call_with_id(event.content.call_id.as_str()) {
                    call.handle_reject(&event.sender, &event.content.party_id);
                }
            }
            CallSignal::SelectAnswer(event) => {
                if let Some(call) = self.call_with_id(event.content.call_id.as_str()) {
                    call.handle_select_answer(&event.sender, &event.content.selected_party_id);
                }
            }
            CallSignal::StreamMetadata(event) => {
                if let Some(call) = self.call_with_id(event.content.call_id.as_str()) {
                    call.handle_stream_metadata(
                        &event.sender,
                        &event.content.party_id,
                        &event.content.sdp_stream_metadata,
                    );
                }
            }
            CallSignal::Negotiate(event) => {
                if let Some(call) = self.call_with_id(event.content.call_id.as_str()) {
                    if is_stale(event.unsigned.age, event.content.lifetime) {
                        // A description that expired on the way here
                        // describes a call as it was, and applying it would
                        // take the call back to a state neither end is in.
                        debug!("Ignoring a renegotiation that is no longer valid");
                        return;
                    }

                    call.handle_negotiate(&event.sender, &event.content.party_id, &event.content);
                }
            }
        }
    }

    /// Act on an invite, which is the only signal that can start something.
    fn handle_invite(&self, room_id: &OwnedRoomId, event: &OriginalSyncCallInviteEvent) {
        let Some(session) = self.inner.session.upgrade() else {
            return;
        };
        let Some(room) = session.room_list().get(room_id) else {
            return;
        };
        let own_user_id = session.user_id().clone();

        // An invite of our own, echoed back through the sync. There is no
        // ignoring events from our own user in general — a person can call
        // themselves from another device — so this is about the room, not
        // the sender.
        if event.sender == own_user_id
            && self
                .active_call()
                .is_some_and(|call| call.call_id() == &event.content.call_id)
        {
            return;
        }

        // "The invite should be ignored if the invitee is set and doesn't
        // match the user's ID."
        if let Some(invitee) = &event.content.invitee
            && *invitee != own_user_id
        {
            debug!("Ignoring a call invite addressed to somebody else");
            return;
        }

        if event.sender == own_user_id {
            // We placed this from another device. Our own other device is
            // the one ringing, not this one.
            return;
        }

        if is_expired(event) {
            debug!("Ignoring a call invite that has already expired");
            return;
        }

        // "As a starting point, it is RECOMMENDED that clients ignore call
        // invites in rooms with a join rule of public." Anybody can walk
        // into one of those, and a ringing phone is a thing a stranger
        // should not be able to cause.
        if room.join_rule().state().value == JoinRuleValue::Public {
            debug!("Ignoring a call invite in a public room");
            return;
        }

        if let Some(active) = self.active_call() {
            self.handle_glare(&active, &room, event);
            return;
        }

        let call = Call::receive(
            &room,
            event.content.call_id.clone(),
            event.content.party_id.clone(),
            &event.sender,
            &event.content,
        );
        self.set_active_call(Some(call));
    }

    /// Decide what to do about an invite that arrived during another call.
    ///
    /// Two people calling each other at the same moment is "glare", and the
    /// spec settles it with a rule both ends can run without talking: of
    /// the two call IDs, the lesser one wins. Both ends reach the same
    /// answer, so the two people end up in one call rather than two
    /// half-calls.
    fn handle_glare(&self, active: &Call, room: &Room, event: &OriginalSyncCallInviteEvent) {
        let same_room = active.room().room_id() == room.room_id();

        if !(same_room && active.is_outgoing() && active.state() == CallState::Dialing) {
            // Not glare, just a second call. Busy is the honest answer.
            debug!("Refusing a call because another one is in progress");
            let busy = Call::receive(
                room,
                event.content.call_id.clone(),
                event.content.party_id.clone(),
                &event.sender,
                &event.content,
            );
            busy.decline_as_busy();
            return;
        }

        if event.content.call_id < *active.call_id() {
            // Theirs wins: drop ours and take theirs, which should look to
            // the user as though the person they called simply picked up.
            // The application answers on the spot; an embedder whose
            // WebRTC lives outside the core is told the call wants
            // answering at once.
            active.hangup();

            let call = Call::receive(
                room,
                event.content.call_id.clone(),
                event.content.party_id.clone(),
                &event.sender,
                &event.content,
            );
            call.mark_wants_immediate_answer();
            self.set_active_call(Some(call));
        }
        // Ours wins: say nothing. Their client runs the same comparison and
        // drops its own call.
    }

    /// Note that a user left a room, which ends any call with them in it.
    pub fn handle_member_left(&self, room: &Room, user_id: &UserId) {
        let Some(call) = self.active_call() else {
            return;
        };

        if call.room().room_id() != room.room_id() {
            return;
        }

        let is_remote = call
            .remote_user_id()
            .is_some_and(|remote| remote == *user_id);

        if is_remote {
            call.handle_remote_left();
        }
    }
}

/// Whether a call can be placed in the given room.
///
/// "Calls should only be placed to rooms with one other user in them. If
/// they are placed to group chat rooms it is possible that another user
/// will intercept and answer the call." That is not a hint; the invite
/// goes to the room, and anybody in it can answer.
///
/// The count is the test, not whether the room is a direct chat. Two
/// people who both happen to be in a room that nobody marked as a DM can
/// still call each other, and a direct chat that grew a third member
/// cannot. The permissions must be loaded for the last check to answer.
#[must_use]
pub fn can_call(room: &Room) -> bool {
    if room.joined_members_count() != 2 || !room.is_joined() {
        return false;
    }

    // A call is a stream of message-like events into the room, so a room
    // we cannot send a message to is a room we cannot call in. The server
    // notices room is exactly that — the recipient sits at power level -10
    // — and offering the buttons there produced a run of `M_FORBIDDEN` and
    // a call that rang for nobody.
    room.permissions().state().can_send_message
}

/// The one other person in a two-person room.
///
/// The direct member answers for a room in `m.direct`; otherwise the
/// joined members are what gets looked at. Returns `None` unless there is
/// exactly one other joined member, which is the same condition as
/// [`can_call`].
pub async fn other_member(room: &Room) -> Option<OwnedUserId> {
    if let Some(user_id) = room.direct_member_user_id() {
        return Some(user_id);
    }

    let own_user_id = room.matrix_room().own_user_id().to_owned();
    let member_list = room.member_list();
    member_list.loaded().await;

    let members = member_list.snapshot();
    let mut others = members
        .iter()
        .filter(|member| member.membership == Membership::Join && member.user_id != own_user_id);

    let first = others.next()?;

    if others.next().is_some() {
        return None;
    }

    Some(first.user_id.clone())
}

/// Say what a call event carried, when it did not carry stream metadata.
///
/// "For backwards compatibility, if `sdp_stream_metadata` is not present
/// in the initial `m.call.invite` or `m.call.answer` event sent by the
/// other party, the client should assume that this property is not
/// supported by the other party." That is the rule and it is followed —
/// but *not supported* and *sent under the name it had while it was still
/// a proposal* look identical from here, and only one of those two is
/// something this client could read.
///
/// So the names of the fields that did arrive are logged, and nothing else
/// about them: an SDP is already logged in full elsewhere and there is no
/// reason to write one twice.
fn note_missing_stream_metadata(kind: &str, event: &Raw<impl Sized>) {
    let Ok(Some(content)) =
        event.get_field::<serde_json::Map<String, serde_json::Value>>("content")
    else {
        return;
    };

    if content.contains_key("sdp_stream_metadata") {
        return;
    }

    debug!(
        "The other party's {kind} carries no stream metadata; its content is {:?}",
        content.keys().collect::<Vec<_>>()
    );
}

/// Whether a session description carries video.
///
/// There is no field for it: an `m.call.invite` says what it offers in its
/// SDP and nowhere else, so what "a video call" means is a media section
/// for video in the offer.
#[must_use]
pub fn sdp_has_video(sdp: &str) -> bool {
    sdp.contains("\r\nm=video ") || sdp.contains("\nm=video ") || sdp.starts_with("m=video ")
}

/// What one call outcome becomes when another is learned.
///
/// Split out from [`Calls::note_outcome`] because the ordering is the
/// whole of the logic and the rest is bookkeeping.
fn merge_outcome(previous: Option<CallOutcome>, next: CallOutcome) -> CallOutcome {
    match (previous, next) {
        // A call that was answered and then hung up ended; it was not
        // missed. Every call ends with a hangup, so without this every call
        // in the timeline would end up saying that nobody answered.
        (Some(CallOutcome::Answered), CallOutcome::Missed) => CallOutcome::Answered,
        // The invite arrives once, and its echo says nothing new.
        (Some(previous), CallOutcome::Ringing) => previous,
        (_, next) => next,
    }
}

/// Whether an invite is too old to ring for.
fn is_expired(event: &OriginalSyncCallInviteEvent) -> bool {
    is_stale(event.unsigned.age, event.content.lifetime)
}

/// Whether an event with a lifetime is past it.
///
/// The `age` the homeserver put on the event is used rather than its
/// timestamp, which is the spec's reason as well: a client with a wrong
/// clock would otherwise discard every event of this kind, or none of
/// them.
fn is_stale(age: Option<Int>, lifetime: UInt) -> bool {
    let Some(age) = age else {
        // No age means we cannot tell. Acting is the recoverable mistake of
        // the two — an invite that should not have rung is hung up by the
        // other end — where not acting loses the call in silence.
        return false;
    };

    let Ok(age) = u64::try_from(i64::from(age)) else {
        return false;
    };

    age >= u64::from(lifetime)
}

#[cfg(test)]
mod tests {
    use ruma::{UInt, events::call::invite::CallInviteEventContent};

    use super::*;

    fn invite_with_age(age: Option<i64>) -> OriginalSyncCallInviteEvent {
        let content = CallInviteEventContent::version_1(
            OwnedVoipId::from("call".to_owned()),
            OwnedVoipId::from("party".to_owned()),
            UInt::from(90_000u32),
            ruma::events::call::SessionDescription::new("offer".to_owned(), String::new()),
        );

        let json = serde_json::json!({
            "content": serde_json::to_value(&content).unwrap(),
            "event_id": "$1:localhost",
            "origin_server_ts": 0,
            "sender": "@alice:localhost",
            "type": "m.call.invite",
            "unsigned": age.map_or_else(|| serde_json::json!({}), |age| serde_json::json!({ "age": age })),
        });

        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn an_invite_older_than_its_lifetime_is_expired() {
        assert!(is_expired(&invite_with_age(Some(90_001))));
        assert!(is_expired(&invite_with_age(Some(120_000))));
    }

    #[test]
    fn a_fresh_invite_is_not() {
        assert!(!is_expired(&invite_with_age(Some(1_000))));
    }

    #[test]
    fn an_invite_with_no_age_is_rung_for() {
        // Not ringing is the mistake that loses a call without saying so.
        assert!(!is_expired(&invite_with_age(None)));
    }

    #[test]
    fn an_offer_with_a_video_section_is_a_video_call() {
        assert!(sdp_has_video(
            "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\nm=video 9 UDP/TLS/RTP/SAVPF 96\r\n"
        ));
        assert!(sdp_has_video("v=0\nm=video 9 UDP/TLS/RTP/SAVPF 96\n"));
    }

    #[test]
    fn an_offer_with_only_audio_is_not() {
        assert!(!sdp_has_video("v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\n"));
    }

    #[test]
    fn a_rejected_video_section_still_counts_as_one() {
        // `m=video 0` is a section the other end refused, and it is still a
        // call that was placed with video in it.
        assert!(sdp_has_video("v=0\r\nm=video 0 UDP/TLS/RTP/SAVPF 96\r\n"));
    }

    #[test]
    fn the_word_video_on_its_own_is_not_a_media_section() {
        assert!(!sdp_has_video("v=0\r\na=rtpmap:96 VP8/90000 video\r\n"));
    }

    #[test]
    fn a_call_that_was_answered_and_hung_up_is_not_a_missed_call() {
        assert_eq!(
            merge_outcome(Some(CallOutcome::Answered), CallOutcome::Missed),
            CallOutcome::Answered
        );
    }

    #[test]
    fn a_call_that_was_never_answered_is() {
        assert_eq!(
            merge_outcome(Some(CallOutcome::Ringing), CallOutcome::Missed),
            CallOutcome::Missed
        );
    }

    #[test]
    fn the_echo_of_an_invite_does_not_undo_what_followed_it() {
        assert_eq!(
            merge_outcome(Some(CallOutcome::Declined), CallOutcome::Ringing),
            CallOutcome::Declined
        );
    }

    #[test]
    fn an_invite_for_a_call_nothing_is_known_about_is_ringing() {
        assert_eq!(
            merge_outcome(None, CallOutcome::Ringing),
            CallOutcome::Ringing
        );
    }
}
