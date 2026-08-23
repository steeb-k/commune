//! One-to-one voice and video calls.
//!
//! This is the Voice over IP module of the Client-Server API: `m.call.*`
//! events carrying WebRTC signalling between exactly two devices. Group calls
//! are a different thing entirely — `MatrixRTC`, still a proposal — and none of
//! this is about them.

use gtk::{glib, glib::clone, prelude::*, subclass::prelude::*};
use matrix_sdk::room::Room as MatrixRoom;
use ruma::{
    OwnedRoomId, OwnedVoipId, UserId,
    events::call::{
        answer::OriginalSyncCallAnswerEvent, candidates::OriginalSyncCallCandidatesEvent,
        hangup::OriginalSyncCallHangupEvent, invite::OriginalSyncCallInviteEvent,
        reject::OriginalSyncCallRejectEvent,
        sdp_stream_metadata_changed::OriginalSyncCallSdpStreamMetadataChangedEvent,
        select_answer::OriginalSyncCallSelectAnswerEvent,
    },
};
use tracing::debug;

mod call;
mod pipeline;
mod state;
mod turn;

pub(crate) use self::{
    call::Call,
    state::{CallEndReason, CallState},
    turn::{TurnCredentials, TurnServer, load_turn_credentials},
};
use super::{JoinRuleValue, Member, Membership, MembershipListKind, Room, Session, UserExt};
use crate::spawn;

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
}

mod imp {
    use std::cell::RefCell;

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
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Calls {
        const NAME: &'static str = "Calls";
        type Type = super::Calls;
    }

    #[glib::derived_properties]
    impl ObjectImpl for Calls {}

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

    /// The TURN servers to use, asking the homeserver if what we have is stale.
    async fn turn_servers(&self) -> Vec<TurnServer> {
        let imp = self.imp();

        if !imp.turn_credentials.borrow().is_fresh() {
            let Some(session) = self.session() else {
                return Vec::new();
            };

            let credentials = load_turn_credentials(&session.client()).await;
            imp.turn_credentials.replace(credentials);
        }

        imp.turn_credentials.borrow().servers().to_vec()
    }

    /// Set the call that is happening, and forget it when it ends.
    fn set_active_call(&self, call: Option<Call>) {
        self.imp().active_call.replace(call.clone());
        self.notify_active_call();

        let Some(call) = call else {
            return;
        };

        call.connect_state_notify(clone!(
            #[weak(rename_to = obj)]
            self,
            move |call| {
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

    /// The call with the given ID, if it is the one that is happening.
    fn call_with_id(&self, call_id: &OwnedVoipId) -> Option<Call> {
        self.active_call().filter(|call| call.call_id() == call_id)
    }

    /// Act on a call event.
    fn handle_signal(&self, room_id: &OwnedRoomId, signal: CallSignal) {
        match signal {
            CallSignal::Invite(event) => self.handle_invite(room_id, &event),
            CallSignal::Answer(event) => {
                if let Some(call) = self.call_with_id(&event.content.call_id) {
                    call.handle_answer(
                        &event.sender,
                        event.content.party_id.as_ref(),
                        &event.content.answer,
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
                    // The last silent path. Candidates for a call this session
                    // is not in look exactly like candidates that never
                    // arrived, and the two want opposite fixes.
                    debug!(
                        "{} ICE candidate(s) arrived for call {}, which is not the call in progress",
                        event.content.candidates.len(),
                        event.content.call_id
                    );
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

/// Whether an invite is too old to ring for.
///
/// The `age` the homeserver put on the event is used rather than its timestamp,
/// which is the spec's reason as well: a client with a wrong clock would
/// otherwise discard every invite, or none of them.
fn is_expired(event: &OriginalSyncCallInviteEvent) -> bool {
    let Some(age) = event.unsigned.age else {
        // No age means we cannot tell. Ringing is the recoverable mistake of
        // the two — the other end hangs up and the ringing stops — where not
        // ringing loses the call in silence.
        return false;
    };

    let Ok(age) = u64::try_from(i64::from(age)) else {
        return false;
    };

    age >= u64::from(event.content.lifetime)
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
}
