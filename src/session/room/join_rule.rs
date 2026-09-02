use commune_core::session::JoinRuleState;
use gettextrs::gettext;
use gtk::{
    glib,
    glib::{clone, closure_local},
    prelude::*,
    subclass::prelude::*,
};
use ruma::events::room::join_rules::JoinRule as MatrixJoinRule;
use tokio::task::AbortHandle;
use tracing::error;

use super::Room;
use crate::{
    components::PillSource, core_bridge::ObjectWatcher, gettext_f, spawn_tokio, utils::BoundObject,
};

/// Simplified join rules.
#[derive(Debug, Default, Hash, Eq, PartialEq, Clone, Copy, glib::Enum)]
#[enum_type(name = "JoinRuleValue")]
pub enum JoinRuleValue {
    /// Only invited users can join.
    #[default]
    Invite,
    /// Anyone can join.
    Public,
    /// Members of a room can join.
    RoomMembership,
    /// The rule is unsupported.
    Unsupported,
}

impl JoinRuleValue {
    /// Whether we support editing this join rule.
    pub(crate) fn can_be_edited(self) -> bool {
        matches!(self, Self::Invite | Self::Public | Self::RoomMembership)
    }
}

impl From<commune_core::session::JoinRuleValue> for JoinRuleValue {
    fn from(value: commune_core::session::JoinRuleValue) -> Self {
        use commune_core::session::JoinRuleValue as Core;

        match value {
            Core::Invite => Self::Invite,
            Core::Public => Self::Public,
            Core::RoomMembership => Self::RoomMembership,
            Core::Unsupported => Self::Unsupported,
        }
    }
}

impl From<JoinRuleValue> for commune_core::session::JoinRuleValue {
    fn from(value: JoinRuleValue) -> Self {
        match value {
            JoinRuleValue::Invite => Self::Invite,
            JoinRuleValue::Public => Self::Public,
            JoinRuleValue::RoomMembership => Self::RoomMembership,
            JoinRuleValue::Unsupported => Self::Unsupported,
        }
    }
}

impl From<&MatrixJoinRule> for JoinRuleValue {
    fn from(value: &MatrixJoinRule) -> Self {
        commune_core::session::JoinRuleValue::from(value).into()
    }
}

mod imp {
    use std::{
        cell::{Cell, RefCell},
        sync::LazyLock,
    };

    use glib::subclass::Signal;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::JoinRule)]
    pub struct JoinRule {
        /// The room where this join rule apply.
        #[property(get)]
        room: glib::WeakRef<Room>,
        /// The value of the join rule.
        #[property(get, builder(JoinRuleValue::default()))]
        value: Cell<JoinRuleValue>,
        /// Whether users can knock.
        #[property(get)]
        can_knock: Cell<bool>,
        /// The string to use to display this join rule.
        ///
        /// This string can contain markup.
        #[property(get)]
        display_name: RefCell<String>,
        /// The room we need to be a member of to match this join rule, if any.
        ///
        /// This can be a `Room` or a `RemoteRoom`.
        // TODO: Support multiple rooms.
        #[property(get)]
        membership_room: BoundObject<PillSource>,
        /// Whether our own user can join this room on their own.
        #[property(get)]
        we_can_join: Cell<bool>,
        /// Whether anyone can join this room on their own.
        #[property(get)]
        anyone_can_join: Cell<bool>,
        /// The task following the core's state.
        watch_handle: RefCell<Option<AbortHandle>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for JoinRule {
        const NAME: &'static str = "RoomJoinRule";
        type Type = super::JoinRule;
    }

    #[glib::derived_properties]
    impl ObjectImpl for JoinRule {
        fn signals() -> &'static [Signal] {
            static SIGNALS: LazyLock<Vec<Signal>> =
                LazyLock::new(|| vec![Signal::builder("changed").build()]);
            SIGNALS.as_ref()
        }

        fn dispose(&self) {
            if let Some(handle) = self.watch_handle.take() {
                handle.abort();
            }
        }
    }

    impl JoinRule {
        /// Set the room where this join rule applies, and follow its core
        /// rule.
        pub(super) fn set_room(&self, room: &Room) {
            self.room.set(Some(room));

            let core = room.core().join_rule();

            let handle = ObjectWatcher::new(&*self.obj())
                .follow(core.subscribe(), |obj: &super::JoinRule, state| {
                    obj.imp().set_state(&state);
                })
                .spawn();
            self.watch_handle.replace(Some(handle));

            // What the core already knows, after subscribing so that
            // nothing between the two is lost.
            self.set_state(&core.state());
        }

        /// Mirror the core's state into the properties.
        fn set_state(&self, state: &JoinRuleState) {
            self.set_value(state.value.into());
            self.set_can_knock(state.can_knock);
            self.update_membership_room(state);
            self.update_display_name();
            self.set_we_can_join(state.we_can_join);
            self.set_anyone_can_join(state.anyone_can_join);

            self.obj().emit_by_name::<()>("changed", &[]);
        }

        /// Set the value of the join rule.
        fn set_value(&self, value: JoinRuleValue) {
            if self.value.get() == value {
                return;
            }

            self.value.set(value);
            self.obj().notify_value();
        }

        /// Set whether users can knock.
        fn set_can_knock(&self, can_knock: bool) {
            if self.can_knock.get() == can_knock {
                return;
            }

            self.can_knock.set(can_knock);
            self.obj().notify_can_knock();
        }

        /// Set the room we need to be a member of to match this join rule.
        fn update_membership_room(&self, state: &JoinRuleState) {
            let room_id = state.membership_room_id.clone();

            if self
                .membership_room
                .obj()
                .map(|d| d.identifier())
                .as_deref()
                == room_id.as_ref().map(|id| id.as_str())
            {
                return;
            }

            self.membership_room.disconnect_signals();

            if let Some(room_id) = room_id {
                let Some(session) = self.room.upgrade().and_then(|r| r.session()) else {
                    return;
                };

                let room: PillSource = if let Some(room) = session.room_list().get(&room_id) {
                    room.upcast()
                } else {
                    session.remote_cache().room(room_id.into()).upcast()
                };

                let display_name_handler = room.connect_display_name_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_display_name();
                    }
                ));

                self.membership_room.set(room, vec![display_name_handler]);
            }

            self.obj().notify_membership_room();
        }

        /// Update the display name of the join rule.
        fn update_display_name(&self) {
            let value = self.value.get();
            let can_knock = self.can_knock.get();

            let name = match value {
                JoinRuleValue::Invite => {
                    if can_knock {
                        gettext("Only invited users, and users can request an invite")
                    } else {
                        gettext("Only invited users")
                    }
                }
                JoinRuleValue::RoomMembership => {
                    let room_name = self
                        .membership_room
                        .obj()
                        .map(|r| r.display_name())
                        .unwrap_or_default();

                    if can_knock {
                        gettext_f(
                            // Translators: Do NOT translate the content between '{' and '}',
                            // this is a variable name.
                            "Members of {room}, and users can request an invite",
                            &[("room", &format!("<b>{room_name}</b>"))],
                        )
                    } else {
                        gettext_f(
                            // Translators: Do NOT translate the content between '{' and '}',
                            // this is a variable name.
                            "Members of {room}",
                            &[("room", &format!("<b>{room_name}</b>"))],
                        )
                    }
                }
                JoinRuleValue::Public => gettext("Any registered user"),
                JoinRuleValue::Unsupported => gettext("Unsupported rule"),
            };

            if *self.display_name.borrow() == name {
                return;
            }

            self.display_name.replace(name);
            self.obj().notify_display_name();
        }

        /// Set whether our own user can join this room on their own.
        fn set_we_can_join(&self, we_can_join: bool) {
            if self.we_can_join.get() == we_can_join {
                return;
            }

            self.we_can_join.set(we_can_join);
            self.obj().notify_we_can_join();
        }

        /// Set whether anyone can join this room on their own.
        fn set_anyone_can_join(&self, anyone_can_join: bool) {
            if self.anyone_can_join.get() == anyone_can_join {
                return;
            }

            self.anyone_can_join.set(anyone_can_join);
            self.obj().notify_anyone_can_join();
        }
    }
}

glib::wrapper! {
    /// The join rule of a room.
    ///
    /// The rule itself is the core's; this presents it, with the sentence
    /// the interface shows for it.
    pub struct JoinRule(ObjectSubclass<imp::JoinRule>);
}

impl JoinRule {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Initialize the join rule with the room where it applies.
    pub(super) fn init(&self, room: &Room) {
        self.imp().set_room(room);
    }

    /// Get the current join rule from the SDK.
    pub(crate) fn matrix_join_rule(&self) -> Option<MatrixJoinRule> {
        self.room()
            .and_then(|room| room.core().join_rule().matrix_join_rule())
    }

    /// Change the join rule.
    pub(crate) async fn set_matrix_join_rule(&self, rule: MatrixJoinRule) -> Result<(), ()> {
        let Some(room) = self.room() else {
            return Err(());
        };

        let core = room.core().clone();
        let handle = spawn_tokio!(async move { core.join_rule().set_matrix_join_rule(rule).await });

        handle
            .await
            .expect("task was not aborted")
            .map_err(|error| {
                error!("Could not change join rule: {error}");
            })
    }

    /// Connect to the signal emitted when the join rule changed.
    pub(crate) fn connect_changed<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "changed",
            true,
            closure_local!(move |obj: Self| {
                f(&obj);
            }),
        )
    }
}

impl Default for JoinRule {
    fn default() -> Self {
        Self::new()
    }
}
