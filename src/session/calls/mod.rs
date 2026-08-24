//! One-to-one voice and video calls.
//!
//! This is the Voice over IP module of the Client-Server API: `m.call.*`
//! events carrying WebRTC signalling between exactly two devices. Group calls
//! are a different thing entirely — `MatrixRTC`, still a proposal — and none of
//! this is about them.

use std::time::{Duration, Instant};

use gtk::{
    glib,
    glib::{clone, closure_local},
    prelude::*,
    subclass::prelude::*,
};
use matrix_sdk::room::Room as MatrixRoom;
use ruma::{
    Int, OwnedRoomId, OwnedVoipId, UInt, UserId,
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
};
use tracing::debug;

mod call;
mod pipeline;
mod ringtone;
mod state;
mod turn;

use self::ringtone::Ringtone;
pub(crate) use self::{
    call::Call,
    state::{CallEndReason, CallOutcome, CallState},
    turn::{IceServers, TurnCredentials, load_turn_credentials},
};
use super::{JoinRuleValue, Member, Membership, MembershipListKind, Room, Session, UserExt};
use crate::spawn;

/// How long a candidate batch is kept for an invite that has not arrived.
///
/// One sync round trip is all it takes; this is generous so that a slow one
/// still lands the candidates rather than losing them.
const EARLY_CANDIDATE_LIFETIME: Duration = Duration::from_secs(30);

/// How many such batches are kept at once.
///
/// A bound rather than a number that matters: the batches are small, and
/// everything in here is either claimed by an invite within a sync or two, or
/// belongs to a call this session is not in.
const MAX_EARLY_CANDIDATE_BATCHES: usize = 8;

/// How many calls are remembered for the sake of the rows in the timeline.
///
/// What is remembered is one enum per call, so the number is a bound rather
/// than a budget: it is there so that a session left running for a week does
/// not keep every call the account ever saw.
const MAX_REMEMBERED_OUTCOMES: usize = 256;

/// Everything that arrives about a call.
///
/// One enum rather than seven handlers on the far side, so that the hop onto
/// the main thread happens once and in one place.
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
/// The other end sends its candidates immediately after the invite, and the
/// two can reach us the other way round — the invite takes longer to send when
/// it is the first encrypted event of a session, and the sync that carries the
/// candidates gets here first. Dropping them costs the call: for a peer that
/// gathers before it dials, that batch is every candidate it will ever send.
#[derive(Debug)]
struct EarlyCandidates {
    /// The room the batch arrived in.
    room_id: OwnedRoomId,
    /// The batch.
    event: OriginalSyncCallCandidatesEvent,
    /// When it arrived, so that it can be forgotten.
    received: Instant,
}

mod imp {
    use std::{
        cell::RefCell,
        collections::{HashMap, VecDeque},
        sync::LazyLock,
    };

    use glib::subclass::Signal;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::Calls)]
    pub struct Calls {
        /// The current session.
        #[property(get, set = Self::set_session, construct_only)]
        pub(super) session: glib::WeakRef<Session>,
        /// The call that is happening, if there is one.
        ///
        /// There is at most one. Two calls at once means two microphones and
        /// two sets of speakers, and no way to say which one a hangup was for.
        #[property(get)]
        pub(super) active_call: RefCell<Option<Call>>,
        /// The TURN credentials, kept until they go stale.
        pub(super) turn_credentials: RefCell<TurnCredentials>,
        /// Candidates that arrived before the invite they belong to.
        pub(super) early_candidates: RefCell<Vec<EarlyCandidates>>,
        /// The sound the call that is happening is making, if it is making
        /// one.
        pub(super) ringtone: RefCell<Option<Ringtone>>,
        /// What became of the calls this session has seen.
        ///
        /// Keyed by call ID, which is what the invite in the timeline carries.
        /// Nothing here is written to disk: a client that was not running when
        /// a call happened has no way to know what became of it, and saying so
        /// is better than guessing.
        pub(super) outcomes: RefCell<HashMap<OwnedVoipId, CallOutcome>>,
        /// The order the outcomes were first noted in, so that the oldest can
        /// be forgotten.
        pub(super) outcome_order: RefCell<VecDeque<OwnedVoipId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Calls {
        const NAME: &'static str = "Calls";
        type Type = super::Calls;
    }

    #[glib::derived_properties]
    impl ObjectImpl for Calls {
        fn signals() -> &'static [Signal] {
            static SIGNALS: LazyLock<Vec<Signal>> = LazyLock::new(|| {
                vec![
                    // The call ID, so that a row can tell whether the change
                    // is about the call it is showing.
                    Signal::builder("call-outcome-changed")
                        .param_types([String::static_type()])
                        .build(),
                ]
            });

            SIGNALS.as_ref()
        }
    }

    impl Calls {
        fn set_session(&self, session: &Session) {
            self.session.set(Some(session));
        }
    }
}

glib::wrapper! {
    /// The calls of a session.
    pub struct Calls(ObjectSubclass<imp::Calls>);
}

impl Calls {
    /// Construct a new `Calls` for the given session.
    pub(crate) fn new(session: &Session) -> Self {
        glib::Object::builder().property("session", session).build()
    }

    /// Start listening for call signalling.
    pub(crate) fn init(&self) {
        let Some(session) = self.session() else {
            return;
        };

        let client = session.client();
        let (sender, mut receiver) = futures_channel::mpsc::unbounded();

        macro_rules! handle {
            ($event:ty, $variant:ident) => {
                let sender = sender.clone();
                client.add_event_handler(move |event: $event, room: MatrixRoom| {
                    let sender = sender.clone();
                    async move {
                        let _ = sender.unbounded_send((
                            room.room_id().to_owned(),
                            CallSignal::$variant(Box::new(event)),
                        ));
                    }
                });
            };
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

        spawn!(clone!(
            #[weak(rename_to = obj)]
            self,
            async move {
                use futures_util::StreamExt as _;

                while let Some((room_id, signal)) = receiver.next().await {
                    obj.handle_signal(&room_id, signal);
                }
            }
        ));
    }

    /// Place a call in the given room.
    ///
    /// Returns `None` when there is already a call, or when the room is not one
    /// call can be placed in.
    pub(crate) async fn place(&self, room: &Room, with_video: bool) -> Option<Call> {
        if self.active_call().is_some() {
            debug!("Refusing to place a second call");
            return None;
        }
        if !can_call(room) {
            return None;
        }

        let servers = self.turn_servers().await;
        let call = Call::place(room, with_video, &servers);
        self.set_active_call(Some(call.clone()));

        Some(call)
    }

    /// Answer the call that is ringing.
    pub(crate) async fn accept_active_call(&self) {
        let Some(call) = self.active_call() else {
            return;
        };

        let servers = self.turn_servers().await;
        call.accept(&servers);
    }

    /// The ICE servers to use, asking the homeserver if what we have is stale.
    async fn turn_servers(&self) -> IceServers {
        let imp = self.imp();

        if !imp.turn_credentials.borrow().is_fresh() {
            let Some(session) = self.session() else {
                return IceServers::default();
            };

            let credentials = load_turn_credentials(&session.client()).await;
            imp.turn_credentials.replace(credentials);
        }

        imp.turn_credentials.borrow().servers().clone()
    }

    /// Set the call that is happening, and forget it when it ends.
    fn set_active_call(&self, call: Option<Call>) {
        self.imp().active_call.replace(call.clone());
        self.notify_active_call();

        let Some(call) = call else {
            return;
        };

        // Candidates for it may already be here, an invite having taken longer
        // to arrive than the batch that followed it.
        self.replay_early_candidates(&call);

        self.update_ringing(&call);

        call.connect_state_notify(clone!(
            #[weak(rename_to = obj)]
            self,
            move |call| {
                obj.update_ringing(call);

                if !call.state().is_ended() {
                    return;
                }

                if obj
                    .active_call()
                    .is_some_and(|active| active.call_id() == call.call_id())
                {
                    obj.imp().active_call.take();
                    obj.notify_active_call();
                }
            }
        ));
    }

    /// Make the noise that goes with the state of the call, and stop making it.
    ///
    /// A window is not enough on its own: it opens behind whatever is on
    /// screen, on whichever workspace the client happens to be on, and a call
    /// that nobody is looking at rings for ninety seconds and is gone.
    fn update_ringing(&self, call: &Call) {
        let imp = self.imp();
        let state = call.state();

        // Only a call coming in. A ringback on the way out was tried and taken
        // out again: a telephone plays one because the caller has nothing to
        // look at, and here the window says `Calling…` in front of them, so all
        // the sound adds is a noise in their own room.
        let ringtone = (state == CallState::Ringing)
            .then(Ringtone::incoming)
            .flatten();

        // Replaced rather than stopped and started, so that a call that goes
        // from ringing to connected stops making a noise at the instant it
        // does.
        imp.ringtone.replace(ringtone);

        let Some(session) = self.session() else {
            return;
        };
        let notifications = session.notifications();

        if state == CallState::Ringing {
            let call = call.clone();
            spawn!(async move {
                notifications.show_incoming_call(&call).await;
            });
        } else {
            notifications.withdraw_incoming_call(call);
        }
    }

    /// Keep a candidate batch whose call has not appeared.
    ///
    /// Measured on 23 August 2026: an incoming call arrived with its whole
    /// batch of twenty-four candidates one sync ahead of the invite, all of
    /// them dropped here, and the call that followed had not one remote
    /// candidate to pair with. The ring worked; nothing after it could.
    fn hold_early_candidates(&self, room_id: &OwnedRoomId, event: OriginalSyncCallCandidatesEvent) {
        debug!(
            "Holding {} ICE candidate(s) for call {}, which has not been invited yet",
            event.content.candidates.len(),
            event.content.call_id
        );

        let now = Instant::now();
        let mut held = self.imp().early_candidates.borrow_mut();

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
        let room = call.room();
        let room_id = room.room_id();
        let call_id = call.call_id();

        let claimed = {
            let mut held = self.imp().early_candidates.borrow_mut();
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
    fn call_with_id(&self, call_id: &OwnedVoipId) -> Option<Call> {
        self.active_call().filter(|call| call.call_id() == call_id)
    }

    /// What became of the call with the given ID, if this session saw it.
    pub(crate) fn outcome(&self, call_id: &str) -> Option<CallOutcome> {
        self.imp()
            .outcomes
            .borrow()
            .get(&OwnedVoipId::from(call_id.to_owned()))
            .copied()
    }

    /// Note what has happened to a call.
    ///
    /// Every call the room shows, not only the one this client is in: a call
    /// answered on another device is one the timeline still has to describe,
    /// and the events that say so arrive here either way.
    fn note_outcome(&self, call_id: &OwnedVoipId, outcome: CallOutcome) {
        let imp = self.imp();
        let mut outcomes = imp.outcomes.borrow_mut();
        let previous = outcomes.get(call_id).copied();

        let outcome = merge_outcome(previous, outcome);

        if previous == Some(outcome) {
            return;
        }

        if previous.is_none() {
            let mut order = imp.outcome_order.borrow_mut();
            order.push_back(call_id.clone());

            while order.len() > MAX_REMEMBERED_OUTCOMES {
                if let Some(forgotten) = order.pop_front() {
                    outcomes.remove(&forgotten);
                }
            }
        }

        outcomes.insert(call_id.clone(), outcome);
        drop(outcomes);

        self.emit_by_name::<()>("call-outcome-changed", &[&call_id.as_str().to_owned()]);
    }

    /// Connect to the outcome of a call changing.
    pub(crate) fn connect_call_outcome_changed<F: Fn(&Self, String) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "call-outcome-changed",
            true,
            closure_local!(move |obj: Self, call_id: String| {
                f(&obj, call_id);
            }),
        )
    }

    /// Act on a call event.
    fn handle_signal(&self, room_id: &OwnedRoomId, signal: CallSignal) {
        // What the timeline says about a call afterwards is built here, from
        // the same events the call itself is made of.
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
                // answered it first is what [`Self::note_outcome`] keeps. The
                // one reason that says something on its own is the busy
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
                if let Some(call) = self.call_with_id(&event.content.call_id) {
                    call.handle_answer(
                        &event.sender,
                        event.content.party_id.as_ref(),
                        &event.content,
                    );
                }
            }
            CallSignal::Candidates(event) => {
                if let Some(call) = self.call_with_id(&event.content.call_id) {
                    call.handle_candidates(
                        &event.sender,
                        event.content.party_id.as_ref(),
                        &event.content.candidates,
                    );
                } else {
                    // Not a call this session is in — or not one it is in
                    // *yet*, the invite being still on its way. The two look
                    // identical here, so the batch is kept for the invite that
                    // may be about to claim it.
                    self.hold_early_candidates(room_id, *event);
                }
            }
            CallSignal::Hangup(event) => {
                if let Some(call) = self.call_with_id(&event.content.call_id) {
                    call.handle_hangup(&event.sender, event.content.party_id.as_ref());
                }
            }
            CallSignal::Reject(event) => {
                if let Some(call) = self.call_with_id(&event.content.call_id) {
                    call.handle_reject(&event.sender, &event.content.party_id);
                }
            }
            CallSignal::SelectAnswer(event) => {
                if let Some(call) = self.call_with_id(&event.content.call_id) {
                    call.handle_select_answer(&event.sender, &event.content.selected_party_id);
                }
            }
            CallSignal::StreamMetadata(event) => {
                if let Some(call) = self.call_with_id(&event.content.call_id) {
                    call.handle_stream_metadata(
                        &event.sender,
                        &event.content.party_id,
                        &event.content.sdp_stream_metadata,
                    );
                }
            }
            CallSignal::Negotiate(event) => {
                if let Some(call) = self.call_with_id(&event.content.call_id) {
                    if is_stale(event.unsigned.age, event.content.lifetime) {
                        // A description that expired on the way here describes
                        // a call as it was, and applying it would take the
                        // call back to a state neither end is in.
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
        let Some(session) = self.session() else {
            return;
        };
        let Some(room) = session.room_list().get(room_id) else {
            return;
        };
        let own_user_id = room.own_member().user_id().clone();

        // An invite of our own, echoed back through the sync. There is no
        // ignoring events from our own user in general — a person can call
        // themselves from another device — so this is about the room, not the
        // sender.
        if event.sender == own_user_id
            && self
                .active_call()
                .is_some_and(|call| call.call_id() == &event.content.call_id)
        {
            return;
        }

        // "The invite should be ignored if the invitee is set and doesn't match
        // the user's ID."
        if let Some(invitee) = &event.content.invitee
            && *invitee != own_user_id
        {
            debug!("Ignoring a call invite addressed to somebody else");
            return;
        }

        if event.sender == own_user_id {
            // We placed this from another device. Our own other device is the
            // one ringing, not this one.
            return;
        }

        if is_expired(event) {
            debug!("Ignoring a call invite that has already expired");
            return;
        }

        // "As a starting point, it is RECOMMENDED that clients ignore call
        // invites in rooms with a join rule of public." Anybody can walk into
        // one of those, and a ringing phone is a thing a stranger should not be
        // able to cause.
        if room.join_rule().value() == JoinRuleValue::Public {
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
    /// spec settles it with a rule both ends can run without talking: of the
    /// two call IDs, the lesser one wins. Both ends reach the same answer, so
    /// the two people end up in one call rather than two half-calls.
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
            // Theirs wins: drop ours and answer theirs, which should look to
            // the user as though the person they called simply picked up.
            active.hangup();

            let call = Call::receive(
                room,
                event.content.call_id.clone(),
                event.content.party_id.clone(),
                &event.sender,
                &event.content,
            );
            self.set_active_call(Some(call.clone()));

            spawn!(clone!(
                #[weak(rename_to = obj)]
                self,
                async move {
                    let servers = obj.turn_servers().await;
                    call.accept(&servers);
                }
            ));
        }
        // Ours wins: say nothing. Their client runs the same comparison and
        // drops its own call.
    }

    /// Note that a user left a room, which ends any call with them in it.
    pub(crate) fn handle_member_left(&self, room: &Room, user_id: &UserId) {
        let Some(call) = self.active_call() else {
            return;
        };

        if call.room().room_id() != room.room_id() {
            return;
        }

        let is_remote = call
            .remote_member()
            .is_some_and(|member| *member.user_id() == *user_id);

        if is_remote {
            call.handle_remote_left();
        }
    }
}

/// Whether a call can be placed in the given room.
///
/// "Calls should only be placed to rooms with one other user in them. If they
/// are placed to group chat rooms it is possible that another user will
/// intercept and answer the call." That is not a hint; the invite goes to the
/// room, and anybody in it can answer.
///
/// The count is the test, not whether the room is a direct chat. Two people who
/// both happen to be in a room that nobody marked as a DM can still call each
/// other, and a direct chat that grew a third member cannot.
pub(crate) fn can_call(room: &Room) -> bool {
    if room.joined_members_count() != 2 || room.own_member().membership() != Membership::Join {
        return false;
    }

    // A call is a stream of message-like events into the room, so a room we
    // cannot send a message to is a room we cannot call in. The server notices
    // room is exactly that — the recipient sits at power level -10 — and
    // offering the buttons there produced a run of `M_FORBIDDEN` and a call
    // that rang for nobody.
    room.permissions().can_send_message()
}

/// The one other person in a two-person room.
///
/// `Room::direct_member()` only answers for a room in `m.direct`, so the joined
/// members are what gets looked at. Returns `None` unless there is exactly one
/// other joined member, which is the same condition as [`can_call`].
pub(crate) fn other_member(room: &Room) -> Option<Member> {
    if let Some(member) = room.direct_member() {
        return Some(member);
    }

    let own_user_id = room.own_member().user_id().clone();
    let joined = room
        .get_or_create_members()
        .membership_list(MembershipListKind::Join);

    let mut others = (0..joined.n_items())
        .filter_map(|position| joined.item(position).and_downcast::<Member>())
        .filter(|member| *member.user_id() != own_user_id);

    let first = others.next()?;

    if others.next().is_some() {
        return None;
    }

    Some(first)
}

/// Whether a session description carries video.
///
/// There is no field for it: an `m.call.invite` says what it offers in its SDP
/// and nowhere else, so what "a video call" means is a media section for video
/// in the offer.
pub(crate) fn sdp_has_video(sdp: &str) -> bool {
    sdp.contains("\r\nm=video ") || sdp.contains("\nm=video ") || sdp.starts_with("m=video ")
}

/// What one call outcome becomes when another is learned.
///
/// Split out from [`Calls::note_outcome`] because the ordering is the whole of
/// the logic and the rest is bookkeeping.
fn merge_outcome(previous: Option<CallOutcome>, next: CallOutcome) -> CallOutcome {
    match (previous, next) {
        // A call that was answered and then hung up ended; it was not missed.
        // Every call ends with a hangup, so without this every call in the
        // timeline would end up saying that nobody answered.
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
/// The `age` the homeserver put on the event is used rather than its timestamp,
/// which is the spec's reason as well: a client with a wrong clock would
/// otherwise discard every event of this kind, or none of them.
fn is_stale(age: Option<Int>, lifetime: UInt) -> bool {
    let Some(age) = age else {
        // No age means we cannot tell. Acting is the recoverable mistake of the
        // two — an invite that should not have rung is hung up by the other
        // end — where not acting loses the call in silence.
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
