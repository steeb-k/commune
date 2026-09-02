//! One-to-one voice and video calls.
//!
//! This is the Voice over IP module of the Client-Server API: `m.call.*`
//! events carrying WebRTC signalling between exactly two devices. Group calls
//! are a different thing entirely — `MatrixRTC`, still a proposal — and none of
//! this is about them.
//!
//! The signalling, the one active call, the TURN credentials, the early
//! candidates and the outcomes are the core's
//! ([`commune_core::session::Calls`]); this presents them, and keeps what
//! is the desktop's: the `webrtcbin` pipeline, the ringtone and the
//! notification.

use commune_core::session::{Call as CoreCall, Calls as CoreCalls};
pub(crate) use commune_core::session::{IceServers, sdp_has_video};
use gtk::{
    glib,
    glib::{clone, closure_local},
    prelude::*,
    subclass::prelude::*,
};
use ruma::UserId;
use tokio::{sync::broadcast::error::RecvError, task::AbortHandle};
use tracing::{debug, warn};

mod call;
mod pipeline;
mod ringtone;
mod state;

use self::ringtone::Ringtone;
pub(crate) use self::{
    call::Call,
    state::{CallEndReason, CallOutcome, CallState},
};
use super::{Member, MembershipListKind, Room, Session, UserExt};
use crate::{
    Application,
    core_bridge::ObjectWatcher,
    session_list::{SessionInfo, SessionInfoExt},
    spawn, spawn_tokio,
};

mod imp {
    use std::{cell::RefCell, sync::LazyLock};

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
        /// The sound the call that is happening is making, if it is making
        /// one.
        pub(super) ringtone: RefCell<Option<Ringtone>>,
        /// The task following the core's active call.
        pub(super) watch_handle: RefCell<Option<AbortHandle>>,
        /// The task carrying the core's outcome changes to the signal.
        pub(super) outcomes_handle: RefCell<Option<AbortHandle>>,
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

        fn dispose(&self) {
            if let Some(handle) = self.watch_handle.take() {
                handle.abort();
            }
            if let Some(handle) = self.outcomes_handle.take() {
                handle.abort();
            }
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

    /// The core's calls, while the session is there.
    fn core(&self) -> Option<CoreCalls> {
        self.session().map(|session| session.core().calls().clone())
    }

    /// Start following the core's calls.
    ///
    /// The core listens for the signalling; this presents the call it has,
    /// and raises the outcome signal a timeline row waits for.
    pub(crate) fn init(&self) {
        let imp = self.imp();
        let Some(core) = self.core() else {
            return;
        };

        let handle = ObjectWatcher::new(self)
            .follow(core.subscribe_active_call(), |obj: &Calls, call| {
                obj.present_active_call(call.as_ref());
            })
            .spawn();
        imp.watch_handle.replace(Some(handle));

        // What the core already has, after subscribing so that nothing
        // between the two is lost.
        self.present_active_call(core.active_call().as_ref());

        let obj_weak = glib::SendWeakRef::from(self.downgrade());
        let mut receiver = core.subscribe_outcome_changed();
        let outcomes_handle = spawn_tokio!(async move {
            loop {
                let call_id = match receiver.recv().await {
                    Ok(call_id) => call_id,
                    Err(RecvError::Lagged(missed)) => {
                        warn!("Missed {missed} call outcome change(s) from the core");
                        continue;
                    }
                    Err(RecvError::Closed) => break,
                };

                let obj_weak = obj_weak.clone();
                let ctx = glib::MainContext::default();
                ctx.spawn(async move {
                    if let Some(obj) = obj_weak.upgrade() {
                        obj.emit_by_name::<()>(
                            "call-outcome-changed",
                            &[&call_id.as_str().to_owned()],
                        );
                    }
                });
            }
        })
        .abort_handle();
        imp.outcomes_handle.replace(Some(outcomes_handle));
    }

    /// Present the call the core has, if it is not the one presented.
    ///
    /// A call we placed is presented before the core has it, so the one
    /// the core reports for it is the one already on screen. A call that
    /// took over from ours in a glare wants answering at once, as the
    /// application answered it.
    fn present_active_call(&self, core_call: Option<&CoreCall>) {
        // The core drops a call once it ended; the presented call heard the
        // end first and is on its way out.
        let Some(core_call) = core_call else {
            return;
        };

        if self
            .active_call()
            .is_some_and(|active| active.presents(core_call))
        {
            return;
        }

        let Some(session) = self.session() else {
            return;
        };
        let Some(room) = session.room_list().get(core_call.room().room_id()) else {
            warn!("The core has a call in a room this session does not");
            return;
        };

        let call = Call::from_core(&room, core_call);
        self.set_active_call(Some(call.clone()));

        if core_call.wants_immediate_answer() {
            // Theirs won the glare: it should look to the user as though
            // the person they called simply picked up.
            spawn!(clone!(
                #[weak(rename_to = obj)]
                self,
                async move {
                    let servers = obj.turn_servers().await;
                    call.accept(&servers);
                }
            ));
        }
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
        let Some(core) = self.core() else {
            return IceServers::default();
        };

        spawn_tokio!(async move { core.turn_servers().await })
            .await
            .expect("task was not aborted")
    }

    /// Set the call that is happening, and forget it when it ends.
    fn set_active_call(&self, call: Option<Call>) {
        self.imp().active_call.replace(call.clone());
        self.notify_active_call();

        let Some(call) = call else {
            return;
        };

        self.update_ringing(&call);

        call.connect_state_notify(clone!(
            #[weak(rename_to = obj)]
            self,
            move |call| {
                obj.update_ringing(call);

                if !call.state().is_ended() {
                    return;
                }

                if obj.active_call().is_some_and(|active| active == *call) {
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

        // A call from an account that is open in this same window is one this
        // window just placed. This client holds several accounts at once, and
        // a call from one of them to another is a real call — the window for
        // it still opens — but ringing and notifying at the person who pressed
        // the button is not telling them anything they do not know. Measured
        // on 23 August 2026, where it was indistinguishable from a ringback:
        // two accounts in one window, and the second one rang.
        let is_from_this_window = call
            .remote_member()
            .is_some_and(|member| is_logged_in_here(member.user_id()));

        if is_from_this_window && state == CallState::Ringing {
            debug!("Not ringing: the call came from another account in this window");
        }

        let should_ring = state == CallState::Ringing && !is_from_this_window;

        // Only a call coming in. A ringback on the way out was tried and taken
        // out again: a telephone plays one because the caller has nothing to
        // look at, and here the window says `Calling…` in front of them, so all
        // the sound adds is a noise in their own room.
        let ringtone = should_ring.then(Ringtone::incoming).flatten();

        // Replaced rather than stopped and started, so that a call that goes
        // from ringing to connected stops making a noise at the instant it
        // does.
        imp.ringtone.replace(ringtone);

        let Some(session) = self.session() else {
            return;
        };
        let notifications = session.notifications();

        if should_ring {
            let call = call.clone();
            spawn!(async move {
                notifications.show_incoming_call(&call).await;
            });
        } else {
            notifications.withdraw_incoming_call(call);
        }
    }

    /// What became of the call with the given ID, if this session saw it.
    pub(crate) fn outcome(&self, call_id: &str) -> Option<CallOutcome> {
        self.core()?.outcome(call_id).map(Into::into)
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
}

/// Whether a call can be placed in the given room.
///
/// The rule is the core's — two people, joined, allowed to send a message
/// — read off the core room, whose permissions the room's page has loaded.
pub(crate) fn can_call(room: &Room) -> bool {
    commune_core::session::can_call(room.core())
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

/// Whether the given user is logged in to this application.
///
/// Not "is this our own user": every account open in this window, since the
/// point of asking is whether the person being called is the person who placed
/// the call.
fn is_logged_in_here(user_id: &UserId) -> bool {
    let application = Application::default();
    let sessions = application.session_list();

    (0..sessions.n_items())
        .filter_map(|position| sessions.item(position).and_downcast::<SessionInfo>())
        // Named, because `UserExt` is in the prelude and has a `user_id()` of
        // its own that a `SessionInfo` cannot answer.
        .any(|session| *SessionInfoExt::user_id(&session) == *user_id)
}
