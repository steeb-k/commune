use std::cell::RefCell;

use commune_core::session::{
    Room as CoreRoom, RoomDisplayName as CoreRoomDisplayName, ServerNotice,
};
use gettextrs::gettext;
use gtk::{
    glib,
    glib::{clone, closure_local},
    prelude::*,
    subclass::prelude::*,
};
use matrix_sdk::{
    Result as MatrixResult, RoomState, deserialized_responses::RawSyncOrStrippedState,
    event_handler::EventHandlerDropGuard, room::Room as MatrixRoom,
};
use ruma::{
    EventId, MatrixToUri, OwnedEventId, OwnedRoomId, OwnedUserId, RoomId, UserId,
    api::{client::receipt::create_receipt::v3::ReceiptType as ApiReceiptType, error::ErrorKind},
    events::{
        SyncStateEvent,
        room::{
            history_visibility::HistoryVisibility,
            member::{MembershipState, SyncRoomMemberEvent},
            server_acl::RoomServerAclEventContent,
        },
    },
    room_version_rules::RoomVersionRules,
};
use tokio::task::AbortHandle;
use tracing::{debug, error, warn};

mod aliases;
mod category;
mod highlight_flags;
mod join_rule;
mod member;
mod member_list;
mod permissions;
mod search;
mod spaces;
mod thread_list;
mod timeline;
mod typing_list;

pub(crate) use self::{
    aliases::{AddAltAliasError, RegisterLocalAliasError, RoomAliases},
    category::{RoomCategory, TargetRoomCategory},
    highlight_flags::HighlightFlags,
    join_rule::{JoinRule, JoinRuleValue},
    member::{Member, Membership},
    member_list::*,
    permissions::*,
    search::{RoomSearch, RoomSearchResult},
    spaces::{add_room_to_space, parent_spaces, remove_room_from_space},
    thread_list::{ThreadList, ThreadListEntry},
    timeline::*,
    typing_list::TypingList,
};
use super::{IdentityVerification, Presence, Session, notifications::NotificationsRoomSetting};
use crate::{
    components::{AtRoom, AvatarImage, AvatarUriSource, PillSource},
    core_bridge::ObjectWatcher,
    gettext_f,
    prelude::*,
    spawn, spawn_tokio,
    utils::{BoundObjectWeakRef, string::linkify},
};

/// The tag order of a room that has none.
///
/// The specification keeps real orders in `[0, 1]` and asks that ordered
/// rooms come first, so anything past 1 sorts a room after all of them.
const NO_TAG_ORDER: f64 = 2.0;

/// Whether the given error is the homeserver refusing to let our user out of
/// the server notices room.
///
/// The Server Notices module makes this its own error code because it is not a
/// failure the user can do anything about: the client "must not expect to be
/// able to reject an invite to join the server notices room", and a server that
/// also prevents leaving after joining answers with the same code.
pub(crate) fn is_cannot_leave_server_notice_room(error: &matrix_sdk::Error) -> bool {
    matches!(
        error.client_api_error_kind(),
        Some(ErrorKind::CannotLeaveServerNoticeRoom)
    )
}

mod imp {
    use std::{
        cell::{Cell, OnceCell},
        marker::PhantomData,
        sync::LazyLock,
    };

    use glib::subclass::Signal;

    use super::*;

    #[derive(Default, glib::Properties)]
    #[properties(wrapper_type = super::Room)]
    pub struct Room {
        /// The room API of the SDK.
        matrix_room: OnceCell<MatrixRoom>,
        /// The core's room, which this presents.
        core: OnceCell<CoreRoom>,
        /// The task following the core's room.
        core_watch_handle: RefCell<Option<AbortHandle>>,
        /// The current session.
        #[property(get, set = Self::set_session, construct_only)]
        session: glib::WeakRef<Session>,
        /// The ID of this room, as a string.
        #[property(get = Self::room_id_string)]
        room_id_string: PhantomData<String>,
        /// The aliases of this room.
        #[property(get)]
        aliases: RoomAliases,
        /// The name that is set for this room.
        ///
        /// This can be empty, the display name should be used instead in the
        /// interface.
        #[property(get)]
        name: RefCell<Option<String>>,
        /// Whether this room has an avatar explicitly set.
        ///
        /// This is `false` if there is no avatar or if the avatar is the one
        /// from the other member.
        #[property(get)]
        has_avatar: Cell<bool>,
        /// The topic of this room.
        #[property(get)]
        topic: RefCell<Option<String>>,
        /// The linkified topic of this room.
        ///
        /// This is the string that should be used in the interface when markup
        /// is allowed.
        #[property(get)]
        topic_linkified: RefCell<Option<String>>,
        /// The category of this room.
        #[property(get, builder(RoomCategory::default()))]
        category: Cell<RoomCategory>,
        /// The order of this room inside its tag, from the `m.tag` account
        /// data.
        ///
        /// The specification orders it in `[0, 1]`, smaller first, and asks
        /// that rooms carrying an order come before rooms without one — so a
        /// room without one reports [`NO_TAG_ORDER`], which sorts after
        /// every real value.
        #[property(get)]
        tag_order: Cell<f64>,
        /// Whether this room is a direct chat.
        #[property(get)]
        is_direct: Cell<bool>,
        /// Whether this room has been upgraded.
        #[property(get)]
        is_tombstoned: Cell<bool>,
        /// The ID of the room that was upgraded and that this one replaces, as
        /// a string.
        #[property(get = Self::predecessor_id_string)]
        predecessor_id_string: PhantomData<Option<String>>,
        /// The ID of the successor of this Room, if this room was upgraded.
        pub(super) successor_id: RefCell<Option<OwnedRoomId>>,
        /// The ID of the successor of this Room, if this room was upgraded, as
        /// a string.
        #[property(get = Self::successor_id_string)]
        successor_id_string: PhantomData<Option<String>>,
        /// The successor of this Room, if this room was upgraded and the
        /// successor was joined.
        #[property(get)]
        successor: glib::WeakRef<super::Room>,
        /// The members of this room.
        #[property(get)]
        pub(super) members: glib::WeakRef<MemberList>,
        members_drop_guard: OnceCell<EventHandlerDropGuard>,
        /// The number of joined members in the room, according to the
        /// homeserver.
        #[property(get)]
        joined_members_count: Cell<u64>,
        /// The member corresponding to our own user.
        #[property(get)]
        own_member: OnceCell<Member>,
        /// Whether this room is a current invite or an invite that was declined
        /// or retracted.
        #[property(get)]
        is_invite: Cell<bool>,
        /// The user who sent the invite to this room.
        ///
        /// This is only set when this room is an invitation.
        #[property(get)]
        inviter: RefCell<Option<Member>>,
        /// The other member of the room, if this room is a direct chat and
        /// there is only one other member.
        #[property(get)]
        direct_member: RefCell<Option<Member>>,
        /// The member whose presence this room's avatar is carrying, and the
        /// handler watching it.
        direct_member_watched: RefCell<Option<Member>>,
        direct_member_presence_handler: RefCell<Option<glib::SignalHandlerId>>,
        /// The live timeline of this room.
        #[property(get)]
        live_timeline: OnceCell<Timeline>,
        /// The timestamp of the room's latest activity.
        ///
        /// This is the timestamp of the latest event that counts as possibly
        /// unread.
        ///
        /// If it is not known, it will return `0`.
        #[property(get)]
        latest_activity: Cell<u64>,
        /// Whether this room is marked as unread.
        #[property(get)]
        is_marked_unread: Cell<bool>,
        /// Whether all messages of this room are read.
        #[property(get)]
        is_read: Cell<bool>,
        /// The number of unread notifications of this room.
        #[property(get)]
        notification_count: Cell<u64>,
        /// whether this room has unread notifications.
        #[property(get)]
        has_notifications: Cell<bool>,
        /// The highlight state of the room.
        #[property(get)]
        highlight: Cell<HighlightFlags>,
        /// Whether this room is encrypted.
        #[property(get)]
        is_encrypted: Cell<bool>,
        /// The join rule of this room.
        #[property(get)]
        join_rule: JoinRule,
        /// Whether guests are allowed.
        #[property(get)]
        guests_allowed: Cell<bool>,
        /// The visibility of the history.
        #[property(get, builder(HistoryVisibilityValue::default()))]
        history_visibility: Cell<HistoryVisibilityValue>,
        /// The version of this room.
        #[property(get = Self::version)]
        version: PhantomData<String>,
        /// Whether this room is federated.
        #[property(get = Self::federated)]
        federated: PhantomData<bool>,
        /// The list of members currently typing in this room.
        #[property(get)]
        typing_list: TypingList,
        /// The notifications settings for this room.
        #[property(get, builder(NotificationsRoomSetting::default()))]
        notifications_setting: Cell<NotificationsRoomSetting>,
        /// The permissions of our own user in this room
        #[property(get)]
        permissions: Permissions,
        /// An ongoing identity verification in this room.
        #[property(get, set = Self::set_verification, nullable, explicit_notify)]
        verification: BoundObjectWeakRef<IdentityVerification>,
        /// Whether the room info is initialized.
        ///
        /// Used to silence logs during initialization.
        #[property(get)]
        is_room_info_initialized: Cell<bool>,
        /// Whether we already attempted an auto-join.
        #[property(get = Self::attempted_auto_join)]
        attempted_auto_join: PhantomData<bool>,
        /// Whether this is a call room as defined by [MSC3417](https://github.com/matrix-org/matrix-spec-proposals/pull/3417)
        #[property(get = Self::is_call)]
        is_call: PhantomData<bool>,
        /// The body of the active server notice of this room, if any.
        ///
        /// A notice is active while it is pinned in the server notices room.
        /// This is always `None` outside of that room.
        #[property(get)]
        active_server_notice: RefCell<Option<String>>,
        /// The contact method for the administrator of the homeserver, given
        /// by the active server notice.
        #[property(get)]
        server_notice_admin_contact: RefCell<Option<String>>,
        /// The number of events pinned in this room.
        #[property(get)]
        pinned_count: Cell<u32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Room {
        const NAME: &'static str = "Room";
        type Type = super::Room;
        type ParentType = PillSource;
    }

    #[glib::derived_properties]
    impl ObjectImpl for Room {
        fn constructed(&self) {
            self.parent_constructed();

            // A `Cell<f64>` starts at zero, which would read as the highest
            // possible order; a room starts unordered instead.
            self.tag_order.set(NO_TAG_ORDER);
        }

        fn dispose(&self) {
            if let Some(handle) = self.core_watch_handle.take() {
                handle.abort();
            }
        }

        fn signals() -> &'static [Signal] {
            static SIGNALS: LazyLock<Vec<Signal>> = LazyLock::new(|| {
                vec![
                    Signal::builder("room-forgotten").build(),
                    Signal::builder("pinned-events-changed").build(),
                ]
            });
            SIGNALS.as_ref()
        }
    }

    impl PillSourceImpl for Room {
        fn identifier(&self) -> String {
            self.aliases
                .alias_string()
                .unwrap_or_else(|| self.room_id_string())
        }
    }

    impl Room {
        /// Initialize this room.
        pub(super) fn init(&self, matrix_room: MatrixRoom) {
            let obj = self.obj();

            self.matrix_room
                .set(matrix_room)
                .expect("matrix room is uninitialized");

            self.init_live_timeline();
            self.aliases.init(&obj);
            #[cfg(not(target_os = "android"))]
            self.watch_members();
            self.join_rule.init(&obj);

            spawn!(
                glib::Priority::DEFAULT_IDLE,
                clone!(
                    #[weak(rename_to = imp)]
                    self,
                    async move {
                        imp.load_own_member().await;
                    }
                )
            );

            self.watch_core();
        }

        /// The room API of the SDK.
        pub(super) fn matrix_room(&self) -> &MatrixRoom {
            self.matrix_room.get().expect("matrix room was initialized")
        }

        /// Set the core's room.
        pub(super) fn set_core(&self, core: CoreRoom) {
            self.core.set(core).expect("core room was uninitialized");
        }

        /// The core's room.
        pub(super) fn core(&self) -> &CoreRoom {
            self.core.get().expect("core room was initialized")
        }

        /// Follow the core's room into this object's properties.
        ///
        /// One task for the lot; every closure runs on the main thread and
        /// ends in a `notify_*()`.
        #[allow(
            clippy::too_many_lines,
            reason = "one follow per property, and the initial read of each"
        )]
        fn watch_core(&self) {
            type R = super::Room;

            let obj = self.obj();
            let core = self.core();

            let handle = ObjectWatcher::new(&*obj)
                .follow(core.subscribe_name(), |obj: &R, name| {
                    obj.imp().set_name(name);
                })
                .follow(core.subscribe_display_name(), |obj: &R, name| {
                    obj.imp().set_display_name_from_core(&name);
                })
                .follow(core.subscribe_has_avatar(), |obj: &R, has_avatar| {
                    obj.imp().set_has_avatar(has_avatar);
                    obj.imp().update_avatar();
                })
                .follow(core.subscribe_avatar_url(), |obj: &R, _| {
                    obj.imp().update_avatar();
                })
                .follow(core.subscribe_topic(), |obj: &R, topic| {
                    obj.imp().set_topic(topic);
                })
                .follow(core.subscribe_category(), |obj: &R, category| {
                    obj.imp().set_category(category.into());
                })
                .follow(core.subscribe_tag_order(), |obj: &R, tag_order| {
                    obj.imp().set_tag_order(tag_order);
                })
                .follow(core.subscribe_is_direct(), |obj: &R, is_direct| {
                    obj.imp().set_is_direct(is_direct);
                })
                .follow(
                    core.subscribe_notifications_setting(),
                    |obj: &R, setting| {
                        obj.imp().set_notifications_setting(setting.into());
                    },
                )
                .follow(core.subscribe_direct_member_user_id(), |obj: &R, _| {
                    obj.imp().spawn_update_direct_member();
                })
                .follow(core.subscribe_is_tombstoned(), |obj: &R, is_tombstoned| {
                    obj.imp().set_is_tombstoned(is_tombstoned);
                })
                .follow(core.subscribe_successor_id(), |obj: &R, successor_id| {
                    obj.imp().set_successor_id(successor_id);
                })
                .follow(core.subscribe_joined_members_count(), |obj: &R, count| {
                    obj.imp().set_joined_members_count(count);
                })
                .follow(core.subscribe_is_invite(), |obj: &R, is_invite| {
                    obj.imp().set_is_invite(is_invite);
                })
                .follow(core.subscribe_inviter_user_id(), |obj: &R, inviter| {
                    obj.imp().spawn_update_inviter(inviter);
                })
                .follow(
                    core.subscribe_latest_activity(),
                    |obj: &R, latest_activity| {
                        obj.imp().set_latest_activity(latest_activity);
                    },
                )
                .follow(
                    core.subscribe_is_marked_unread(),
                    |obj: &R, is_marked_unread| {
                        obj.imp().set_is_marked_unread(is_marked_unread);
                    },
                )
                .follow(core.subscribe_is_read(), |obj: &R, is_read| {
                    obj.imp().set_is_read(is_read);
                })
                .follow(core.subscribe_notification_count(), |obj: &R, count| {
                    obj.imp().set_notification_count(count);
                })
                .follow(core.subscribe_highlight(), |obj: &R, highlight| {
                    obj.imp().set_highlight(highlight.into());
                })
                .follow(core.subscribe_is_encrypted(), |obj: &R, is_encrypted| {
                    obj.imp().set_is_encrypted(is_encrypted);
                })
                .follow(
                    core.subscribe_guests_allowed(),
                    |obj: &R, guests_allowed| {
                        obj.imp().set_guests_allowed(guests_allowed);
                    },
                )
                .follow(
                    core.subscribe_history_visibility(),
                    |obj: &R, visibility| {
                        obj.imp().set_history_visibility(visibility.into());
                    },
                )
                .follow(core.subscribe_typing(), |obj: &R, user_ids| {
                    obj.imp().update_typing_list(user_ids);
                })
                .follow(
                    core.subscribe_is_room_info_initialized(),
                    |obj: &R, is_initialized| {
                        obj.imp().set_is_room_info_initialized(is_initialized);
                    },
                )
                .follow(core.subscribe_active_server_notice(), |obj: &R, notice| {
                    obj.imp().set_active_server_notice(notice);
                })
                .follow(core.subscribe_pinned_event_ids(), |obj: &R, event_ids| {
                    obj.imp().set_pinned_event_ids(&event_ids);
                })
                .spawn();
            self.core_watch_handle.replace(Some(handle));

            // What the core already knows, after subscribing so that nothing
            // between the two is lost.
            self.set_name(core.name());
            self.set_display_name_from_core(&core.display_name());
            self.set_has_avatar(core.has_avatar());
            self.update_avatar();
            self.set_topic(core.topic());
            self.set_category(core.category().into());
            self.set_tag_order(core.tag_order());
            self.set_is_direct(core.is_direct());
            self.set_notifications_setting(core.notifications_setting().into());
            self.spawn_update_direct_member();
            self.set_is_tombstoned(core.is_tombstoned());
            self.set_successor_id(core.successor_id());
            self.set_joined_members_count(core.joined_members_count());
            self.set_is_invite(core.is_invite());
            self.spawn_update_inviter(core.inviter_user_id());
            self.set_latest_activity(core.latest_activity());
            self.set_is_marked_unread(core.is_marked_unread());
            self.set_is_read(core.is_read());
            self.set_notification_count(core.notification_count());
            self.set_highlight(core.highlight().into());
            self.set_is_encrypted(core.is_encrypted());
            self.set_guests_allowed(core.guests_allowed());
            self.set_history_visibility(core.history_visibility().into());
            self.update_typing_list(core.typing_users());
            self.set_is_room_info_initialized(core.is_room_info_initialized());
            self.set_active_server_notice(core.active_server_notice());
            self.set_pinned_event_ids(&core.pinned_event_ids());
        }

        /// Set the name of this room.
        fn set_name(&self, name: Option<String>) {
            if *self.name.borrow() == name {
                return;
            }

            self.name.replace(name);
            self.obj().notify_name();
        }

        /// Set the display name from the core's, rendering the cases that
        /// take a sentence.
        fn set_display_name_from_core(&self, name: &CoreRoomDisplayName) {
            let display_name = match name {
                CoreRoomDisplayName::Named(s) => s.clone(),
                CoreRoomDisplayName::EmptyWas(s) => {
                    // Translators: This is the name of a room that is empty but had another
                    // user before. Do NOT translate the content between
                    // '{' and '}', this is a variable name.
                    gettext_f("Empty Room (was {user})", &[("user", s)])
                }
                // Translators: This is the name of a room without other users.
                CoreRoomDisplayName::Empty => gettext("Empty Room"),
                // Translators: This is displayed when the room name is unknown yet.
                CoreRoomDisplayName::Unknown => gettext("Unknown"),
            };

            self.obj().set_display_name(display_name);
        }

        /// Set the topic of this room.
        fn set_topic(&self, topic: Option<String>) {
            if *self.topic.borrow() == topic {
                return;
            }

            let topic_linkified = topic.as_ref().map(|t| {
                // Detect links.
                let mut s = linkify(t);
                // Remove trailing spaces.
                s.truncate_end_whitespaces();
                s
            });

            self.topic.replace(topic);
            self.topic_linkified.replace(topic_linkified);

            let obj = self.obj();
            obj.notify_topic();
            obj.notify_topic_linkified();
        }

        /// Set whether this room is a direct chat.
        fn set_is_direct(&self, is_direct: bool) {
            if self.is_direct.get() == is_direct {
                return;
            }

            self.is_direct.set(is_direct);
            self.obj().notify_is_direct();
        }

        /// Update the direct member, on the main context.
        fn spawn_update_direct_member(&self) {
            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    imp.update_direct_member().await;
                }
            ));
        }

        /// Set whether this room has been upgraded.
        fn set_is_tombstoned(&self, is_tombstoned: bool) {
            if self.is_tombstoned.get() == is_tombstoned {
                return;
            }

            self.is_tombstoned.set(is_tombstoned);
            self.obj().notify_is_tombstoned();
        }

        /// Set the ID of the successor of this room.
        fn set_successor_id(&self, successor_id: Option<OwnedRoomId>) {
            if *self.successor_id.borrow() == successor_id {
                return;
            }

            self.successor_id.replace(successor_id);
            self.obj().notify_successor_id_string();
            self.update_successor();
        }

        /// Set whether this room is a current invite or an invite that was
        /// declined or retracted.
        fn set_is_invite(&self, is_invite: bool) {
            if self.is_invite.get() == is_invite {
                return;
            }

            self.is_invite.set(is_invite);
            self.obj().notify_is_invite();
        }

        /// Update the inviter, on the main context.
        fn spawn_update_inviter(&self, inviter_user_id: Option<OwnedUserId>) {
            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    imp.update_inviter(inviter_user_id).await;
                }
            ));
        }

        /// Update the member that invited us to this room, from the user the
        /// core says it is.
        async fn update_inviter(&self, inviter_user_id: Option<OwnedUserId>) {
            let Some(user_id) = inviter_user_id else {
                if self.inviter.take().is_some() {
                    self.obj().notify_inviter();
                }
                return;
            };

            let existing = self.inviter.borrow().clone();
            let inviter = match existing.filter(|inviter| *inviter.user_id() == user_id) {
                Some(inviter) => inviter,
                None => Member::new(&self.obj(), user_id.clone()),
            };

            let matrix_room = self.matrix_room().clone();
            let handle =
                spawn_tokio!(async move { matrix_room.get_member_no_sync(&user_id).await });

            match handle.await.expect("task was not aborted") {
                Ok(Some(matrix_member)) => inviter.update_from_room_member(&matrix_member),
                Ok(None) => {}
                Err(error) => {
                    error!("Could not get inviter: {error}");
                }
            }

            if self.inviter.borrow().as_ref() != Some(&inviter) {
                self.inviter.replace(Some(inviter));
                self.obj().notify_inviter();
            }
        }

        /// Set whether this room is marked as unread.
        fn set_is_marked_unread(&self, is_marked_unread: bool) {
            if self.is_marked_unread.get() == is_marked_unread {
                return;
            }

            self.is_marked_unread.set(is_marked_unread);
            self.obj().notify_is_marked_unread();
        }

        /// Set whether this room is encrypted.
        fn set_is_encrypted(&self, is_encrypted: bool) {
            if self.is_encrypted.get() == is_encrypted {
                return;
            }

            self.is_encrypted.set(is_encrypted);
            self.obj().notify_is_encrypted();
        }

        /// Set whether guests are allowed.
        fn set_guests_allowed(&self, guests_allowed: bool) {
            if self.guests_allowed.get() == guests_allowed {
                return;
            }

            self.guests_allowed.set(guests_allowed);
            self.obj().notify_guests_allowed();
        }

        /// Set the visibility of the history.
        fn set_history_visibility(&self, visibility: HistoryVisibilityValue) {
            if self.history_visibility.get() == visibility {
                return;
            }

            self.history_visibility.set(visibility);
            self.obj().notify_history_visibility();
        }

        /// Set whether the room info is initialized.
        fn set_is_room_info_initialized(&self, is_initialized: bool) {
            if self.is_room_info_initialized.get() == is_initialized {
                return;
            }

            self.is_room_info_initialized.set(is_initialized);
            self.obj().notify_is_room_info_initialized();

            if is_initialized {
                self.on_room_info_initialized();
            }
        }

        /// What waits for the category to be known, since it is only done for
        /// some categories.
        fn on_room_info_initialized(&self) {
            // The core's category rather than the mirror: the two streams
            // are merged, and the mirror may be a step behind.
            let category: RoomCategory = self.core().category().into();

            // Preload the timeline of rooms that the user is likely to visit
            // and for which we offer to show the timeline.
            let preload = matches!(
                category,
                RoomCategory::Favorite
                    | RoomCategory::Normal
                    | RoomCategory::LowPriority
                    | RoomCategory::ServerNotice
            );
            self.live_timeline().set_preload(preload);

            spawn!(
                glib::Priority::DEFAULT_IDLE,
                clone!(
                    #[weak(rename_to = imp)]
                    self,
                    async move {
                        imp.permissions.init(&imp.obj()).await;
                    }
                )
            );
        }

        /// Whether we already attempted an auto-join.
        fn attempted_auto_join(&self) -> bool {
            self.core().attempted_auto_join()
        }

        /// Set the active server notice of this room.
        fn set_active_server_notice(&self, notice: Option<ServerNotice>) {
            let (body, admin_contact) = match notice {
                Some(notice) => (Some(notice.body), notice.admin_contact),
                None => (None, None),
            };

            if *self.active_server_notice.borrow() != body {
                self.active_server_notice.replace(body);
                self.obj().notify_active_server_notice();
            }

            if *self.server_notice_admin_contact.borrow() != admin_contact {
                self.server_notice_admin_contact.replace(admin_contact);
                self.obj().notify_server_notice_admin_contact();
            }
        }

        /// Set the events pinned in this room.
        fn set_pinned_event_ids(&self, event_ids: &[OwnedEventId]) {
            let count = event_ids.len().try_into().unwrap_or(u32::MAX);

            if self.pinned_count.get() != count {
                self.pinned_count.set(count);
                self.obj().notify_pinned_count();
            }

            self.obj().emit_by_name::<()>("pinned-events-changed", &[]);
        }

        /// Set the current session
        fn set_session(&self, session: &Session) {
            self.session.set(Some(session));

            let own_member = Member::new(&self.obj(), session.user_id().clone());
            self.own_member
                .set(own_member)
                .expect("own member was uninitialized");
        }

        /// The ID of this room.
        pub(super) fn room_id(&self) -> &RoomId {
            self.matrix_room().room_id()
        }

        /// The ID of this room, as a string.
        fn room_id_string(&self) -> String {
            self.matrix_room().room_id().to_string()
        }

        /// Set whether this room has an avatar explicitly set.
        fn set_has_avatar(&self, has_avatar: bool) {
            if self.has_avatar.get() == has_avatar {
                return;
            }

            self.has_avatar.set(has_avatar);
            self.obj().notify_has_avatar();
        }

        /// Update the avatar of the room.
        fn update_avatar(&self) {
            let Some(session) = self.session.upgrade() else {
                return;
            };

            let obj = self.obj();
            let avatar_data = obj.avatar_data();
            let matrix_room = self.matrix_room();

            let prev_avatar_url = avatar_data.image().and_then(|i| i.uri());
            let room_avatar_url = matrix_room.avatar_url();

            if prev_avatar_url.is_some() && prev_avatar_url == room_avatar_url {
                // The avatar did not change.
                return;
            }

            if let Some(avatar_url) = room_avatar_url {
                // The avatar has changed, update it.
                let avatar_info = matrix_room.avatar_info();

                if let Some(avatar_image) = avatar_data
                    .image()
                    .filter(|i| i.uri_source() == AvatarUriSource::Room)
                {
                    avatar_image.set_uri_and_info(Some(avatar_url), avatar_info);
                } else {
                    let avatar_image = AvatarImage::new(
                        &session,
                        AvatarUriSource::Room,
                        Some(avatar_url),
                        avatar_info,
                    );

                    avatar_data.set_image(Some(avatar_image.clone()));
                }

                self.set_has_avatar(true);
                return;
            }

            self.set_has_avatar(false);

            // If we have a direct member, use their avatar.
            if let Some(direct_member) = self.direct_member.borrow().as_ref() {
                avatar_data.set_image(direct_member.avatar_data().image());
            }

            let avatar_image = avatar_data.image();

            if let Some(avatar_image) = avatar_image
                .as_ref()
                .filter(|i| i.uri_source() == AvatarUriSource::Room)
            {
                // The room has no avatar, make sure we remove it.
                avatar_image.set_uri_and_info(None, None);
            } else if avatar_image.is_none() {
                // We always need an avatar image, even if it is empty.
                avatar_data.set_image(Some(AvatarImage::new(
                    &session,
                    AvatarUriSource::Room,
                    None,
                    None,
                )));
            }
        }

        /// Set the category of this room, from the core.
        fn set_category(&self, category: RoomCategory) {
            let old_category = self.category.get();

            if old_category == category {
                return;
            }

            self.category.set(category);
            self.obj().notify_category();

            // Check if the previous state was different.
            let room_state = self.matrix_room().state();
            if !old_category.is_state(room_state) {
                if self.is_room_info_initialized.get() {
                    debug!(room_id = %self.room_id(), ?room_state, "The state of the room changed");
                }

                match room_state {
                    RoomState::Joined => {
                        if let Some(members) = self.members.upgrade() {
                            // If we where invited or left before, the list was likely not completed
                            // or might have changed.
                            members.reload();
                        }
                    }
                    RoomState::Left
                    | RoomState::Knocked
                    | RoomState::Banned
                    | RoomState::Invited => {}
                }
            }

            if category == RoomCategory::Outdated {
                self.update_successor();
            }
        }

        /// Set the order of this room inside its tag.
        fn set_tag_order(&self, tag_order: f64) {
            if (self.tag_order.get() - tag_order).abs() < f64::EPSILON {
                return;
            }

            self.tag_order.set(tag_order);
            self.obj().notify_tag_order();
        }

        /// Update the successor of this room.
        pub(super) fn update_successor(&self) {
            let Some(session) = self.session.upgrade() else {
                return;
            };
            let room_list = session.room_list();

            let successor_id = self.successor_id.borrow().clone();
            if let Some(successor) =
                successor_id.and_then(|successor_id| room_list.get(&successor_id))
            {
                // The Matrix spec says that we should use the "predecessor" field of the
                // m.room.create event of the successor, not the "successor" field of the
                // m.room.tombstone event, so check it just to be sure.
                if successor
                    .predecessor_id()
                    .is_some_and(|predecessor_id| predecessor_id == self.room_id())
                {
                    self.set_successor(&successor);
                    return;
                }
            }

            // The tombstone event can be redacted and we lose the successor, so search in
            // the room predecessors of other rooms.
            for room in room_list.iter::<super::Room>() {
                let Ok(room) = room else {
                    break;
                };

                if room
                    .predecessor_id()
                    .is_some_and(|predecessor_id| predecessor_id == self.room_id())
                {
                    self.set_successor(&room);
                    return;
                }
            }
        }

        /// The ID of the room that was upgraded and that this one replaces, as
        /// a string.
        fn predecessor_id_string(&self) -> Option<String> {
            self.core().predecessor_id().map(ToString::to_string)
        }

        /// The ID of the successor of this room, if this room was upgraded.
        fn successor_id_string(&self) -> Option<String> {
            self.successor_id.borrow().as_ref().map(ToString::to_string)
        }

        /// Set the successor of this room.
        fn set_successor(&self, successor: &super::Room) {
            if self.successor.upgrade().as_ref() == Some(successor) {
                return;
            }

            self.successor.set(Some(successor));
            self.obj().notify_successor();
        }

        /// Watch the room's member events for the one thing the
        /// application's calls still need from them.
        ///
        /// The core's room handles member events for the member list and
        /// the direct member; the desktop's calls are the application's
        /// own until their module, and a call has to hear that the other
        /// party left. Goes with module 11.
        #[cfg(not(target_os = "android"))]
        fn watch_members(&self) {
            let matrix_room = self.matrix_room();

            let obj_weak = glib::SendWeakRef::from(self.obj().downgrade());
            let handle = matrix_room.add_event_handler(move |event: SyncRoomMemberEvent| {
                let obj_weak = obj_weak.clone();
                async move {
                    // "If the client sees the user it is in a call with leave
                    // the room, the client should treat this as a hangup
                    // event for any calls that are in progress."
                    if !matches!(
                        event.membership(),
                        MembershipState::Leave | MembershipState::Ban
                    ) {
                        return;
                    }

                    let ctx = glib::MainContext::default();
                    ctx.spawn(async move {
                        spawn!(async move {
                            if let Some(obj) = obj_weak.upgrade()
                                && let Some(session) = obj.session()
                            {
                                session.calls().handle_member_left(&obj, event.state_key());
                            }
                        });
                    });
                }
            });

            let drop_guard = matrix_room.client().event_handler_drop_guard(handle);
            self.members_drop_guard
                .set(drop_guard)
                .expect("members drop guard is uninitialized");
        }

        /// Set the number of joined members in the room, according to the
        /// homeserver.
        fn set_joined_members_count(&self, count: u64) {
            if self.joined_members_count.get() == count {
                return;
            }

            self.joined_members_count.set(count);
            self.obj().notify_joined_members_count();
        }

        /// The member corresponding to our own user.
        pub(super) fn own_member(&self) -> &Member {
            self.own_member.get().expect("Own member was initialized")
        }

        /// Load our own member from the store.
        async fn load_own_member(&self) {
            let own_member = self.own_member();
            let user_id = own_member.user_id().clone();
            let matrix_room = self.matrix_room().clone();

            let handle =
                spawn_tokio!(async move { matrix_room.get_member_no_sync(&user_id).await });

            match handle.await.expect("task was not aborted") {
                Ok(Some(matrix_member)) => own_member.update_from_room_member(&matrix_member),
                Ok(None) => {}
                Err(error) => error!(
                    "Could not load own member for room {}: {error}",
                    self.room_id()
                ),
            }
        }

        /// Set the other member of the room, if this room is a direct chat and
        /// there is only one other member.
        fn set_direct_member(&self, member: Option<Member>) {
            if *self.direct_member.borrow() == member {
                return;
            }

            self.direct_member.replace(member);
            self.obj().notify_direct_member();
            self.update_avatar();
            self.update_direct_member_presence();
        }

        /// Carry the presence of the other person onto this room's avatar, if
        /// this is a direct chat.
        ///
        /// A direct chat is the one room where the room *is* a person, so its
        /// avatar answers the same question a member's does. Any other room
        /// stays at `Presence::Unknown` and so draws no badge.
        fn update_direct_member_presence(&self) {
            let obj = self.obj();
            let avatar_data = obj.avatar_data();

            if let Some(handler) = self.direct_member_presence_handler.take() {
                avatar_data.set_presence(Presence::default());

                if let Some(member) = self.direct_member_watched.take() {
                    member.disconnect(handler);
                }
            }

            let direct_member = self.direct_member.borrow().clone();
            let Some(direct_member) = direct_member else {
                return;
            };

            let handler = direct_member.connect_presence_notify(clone!(
                #[weak]
                avatar_data,
                move |member| {
                    avatar_data.set_presence(member.presence());
                }
            ));

            avatar_data.set_presence(direct_member.presence());
            self.direct_member_presence_handler.replace(Some(handler));
            self.direct_member_watched.replace(Some(direct_member));
        }

        /// Update the other member of the room, from the user the core says
        /// this is a direct chat with.
        async fn update_direct_member(&self) {
            let Some(direct_user_id) = self.core().direct_member_user_id() else {
                self.set_direct_member(None);
                return;
            };

            if self
                .direct_member
                .borrow()
                .as_ref()
                .is_some_and(|m| *m.user_id() == direct_user_id)
            {
                // Already up-to-date.
                return;
            }

            let direct_member = if let Some(members) = self.members.upgrade() {
                members.get_or_create(direct_user_id.clone())
            } else {
                Member::new(&self.obj(), direct_user_id.clone())
            };

            let matrix_room = self.matrix_room().clone();
            let handle =
                spawn_tokio!(async move { matrix_room.get_member_no_sync(&direct_user_id).await });

            match handle.await.expect("task was not aborted") {
                Ok(Some(matrix_member)) => {
                    direct_member.update_from_room_member(&matrix_member);
                }
                Ok(None) => {}
                Err(error) => {
                    error!("Could not get direct member: {error}");
                }
            }

            self.set_direct_member(Some(direct_member));
        }

        /// Initialize the live timeline of this room.
        ///
        /// Creating it creates the core's, whose read-state watcher keeps
        /// `is_read` and the latest activity current from then on.
        fn init_live_timeline(&self) {
            self.live_timeline
                .get_or_init(|| Timeline::new(&self.obj()));
        }

        /// The live timeline of this room.
        fn live_timeline(&self) -> &Timeline {
            self.live_timeline
                .get()
                .expect("live timeline is initialized")
        }

        /// Set the timestamp of the room's latest possibly unread event.
        pub(super) fn set_latest_activity(&self, latest_activity: u64) {
            if self.latest_activity.get() == latest_activity {
                return;
            }

            self.latest_activity.set(latest_activity);
            self.obj().notify_latest_activity();
        }

        /// Set whether all messages of this room are read.
        fn set_is_read(&self, is_read: bool) {
            if self.is_read.get() == is_read {
                return;
            }

            self.is_read.set(is_read);
            self.obj().notify_is_read();
        }

        /// Set how this room is highlighted.
        fn set_highlight(&self, highlight: HighlightFlags) {
            if self.highlight.get() == highlight {
                return;
            }

            self.highlight.set(highlight);
            self.obj().notify_highlight();
        }

        /// Set the number of unread notifications of this room.
        fn set_notification_count(&self, count: u64) {
            if self.notification_count.get() == count {
                return;
            }

            self.notification_count.set(count);
            self.set_has_notifications(count > 0);
            self.obj().notify_notification_count();
        }

        /// Set whether this room has unread notifications.
        fn set_has_notifications(&self, has_notifications: bool) {
            if self.has_notifications.get() == has_notifications {
                return;
            }

            self.has_notifications.set(has_notifications);
            self.obj().notify_has_notifications();
        }

        /// The version of this room.
        fn version(&self) -> String {
            self.matrix_room()
                .create_content()
                .map(|c| c.room_version.to_string())
                .unwrap_or_default()
        }

        /// If this is a Call room as defined by [MSC3417].
        ///
        /// [MSC3417]: <https://github.com/matrix-org/matrix-spec-proposals/pull/3417>
        fn is_call(&self) -> bool {
            self.matrix_room().is_call()
        }

        /// The rules for the version of this room.
        pub(super) fn rules(&self) -> RoomVersionRules {
            self.matrix_room()
                .clone_info()
                .room_version_rules_or_default()
        }

        /// Whether this room is federated.
        fn federated(&self) -> bool {
            self.matrix_room()
                .create_content()
                .is_some_and(|c| c.federate)
        }

        /// Update the typing list with the given user IDs.
        fn update_typing_list(&self, typing_user_ids: Vec<OwnedUserId>) {
            let Some(session) = self.session.upgrade() else {
                return;
            };

            let Some(members) = self.members.upgrade() else {
                // If we don't have a members list, the room is not shown so we don't need to
                // update the typing list.
                self.typing_list.update(vec![]);
                return;
            };

            let own_user_id = session.user_id();

            let members = typing_user_ids
                .into_iter()
                .filter(|user_id| user_id != own_user_id)
                .map(|user_id| members.get_or_create(user_id))
                .collect();

            self.typing_list.update(members);
        }

        /// Set the notifications setting for this room.
        fn set_notifications_setting(&self, setting: NotificationsRoomSetting) {
            if self.notifications_setting.get() == setting {
                return;
            }

            self.notifications_setting.set(setting);
            self.obj().notify_notifications_setting();
        }

        /// Set an ongoing verification in this room.
        fn set_verification(&self, verification: Option<IdentityVerification>) {
            if self.verification.obj().is_some() && verification.is_some() {
                // Just keep the same verification until it is dropped. Then we will look if
                // there is an ongoing verification in the room.
                return;
            }

            self.verification.disconnect_signals();

            let verification = verification.or_else(|| {
                // Look if there is an ongoing verification to replace it with.
                let room_id = self.matrix_room().room_id();
                self.session
                    .upgrade()
                    .map(|s| s.verification_list())
                    .and_then(|list| list.ongoing_room_verification(room_id))
            });

            if let Some(verification) = &verification {
                let state_handler = verification.connect_is_finished_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.set_verification(None);
                    }
                ));

                let dismiss_handler = verification.connect_dismiss(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.set_verification(None);
                    }
                ));

                self.verification
                    .set(verification, vec![state_handler, dismiss_handler]);
            }

            self.obj().notify_verification();
        }

        /// Change the category of this room.
        ///
        /// This makes the necessary to propagate the category to the
        /// homeserver.
        ///
        /// This can be used to trigger actions like join or leave, as well as
        /// changing the category in the sidebar.
        ///
        /// Note that rooms cannot change category once they are upgraded.
        pub(super) async fn change_category(
            &self,
            category: TargetRoomCategory,
        ) -> MatrixResult<()> {
            // The core sets the category on the spot and puts it back if the
            // homeserver refuses; the mirror follows both.
            let core = self.core().clone();
            spawn_tokio!(async move { core.change_category(category.into()).await })
                .await
                .expect("task was not aborted")
        }
    }
}

glib::wrapper! {
    /// GObject representation of a Matrix room.
    ///
    /// Handles populating the Timeline.
    pub struct Room(ObjectSubclass<imp::Room>) @extends PillSource;
}

impl Room {
    /// Create a new `Room` for the given session, presenting the given core
    /// room.
    pub fn new(session: &Session, core: CoreRoom) -> Self {
        let this = glib::Object::builder::<Self>()
            .property("session", session)
            .build();

        let matrix_room = core.matrix_room().clone();

        this.imp().set_core(core);
        this.imp().init(matrix_room);
        this
    }

    /// The room API of the SDK.
    pub(crate) fn matrix_room(&self) -> &MatrixRoom {
        self.imp().matrix_room()
    }

    /// The core's room, which this presents.
    pub(crate) fn core(&self) -> &CoreRoom {
        self.imp().core()
    }

    /// The ID of this room.
    pub(crate) fn room_id(&self) -> &RoomId {
        self.imp().room_id()
    }

    /// Get a human-readable ID for this `Room`.
    ///
    /// This shows the display name and room ID to identify the room easily in
    /// logs.
    pub fn human_readable_id(&self) -> String {
        format!("{} ({})", self.display_name(), self.room_id())
    }

    /// The rules for the version of this room.
    pub(crate) fn rules(&self) -> RoomVersionRules {
        self.imp().rules()
    }

    /// Whether this room is joined.
    pub(crate) fn is_joined(&self) -> bool {
        self.own_member().membership() == Membership::Join
    }

    /// The ID of the predecessor of this room, if this room is an upgrade to a
    /// previous room.
    pub(crate) fn predecessor_id(&self) -> Option<&OwnedRoomId> {
        self.core().predecessor_id()
    }

    /// The ID of the successor of this Room, if this room was upgraded.
    pub(crate) fn successor_id(&self) -> Option<OwnedRoomId> {
        self.imp().successor_id.borrow().clone()
    }

    /// The `matrix.to` URI representation for this room.
    pub(crate) async fn matrix_to_uri(&self) -> MatrixToUri {
        let matrix_room = self.matrix_room().clone();

        let handle = spawn_tokio!(async move { matrix_room.matrix_to_permalink().await });
        match handle.await.expect("task was not aborted") {
            Ok(permalink) => {
                return permalink;
            }
            Err(error) => {
                error!("Could not get room event permalink: {error}");
            }
        }

        // Fallback to using just the room ID, without routing.
        self.room_id().matrix_to_uri()
    }

    /// The `matrix.to` URI representation for the given event in this room.
    pub(crate) async fn matrix_to_event_uri(&self, event_id: OwnedEventId) -> MatrixToUri {
        let matrix_room = self.matrix_room().clone();

        let event_id_clone = event_id.clone();
        let handle =
            spawn_tokio!(
                async move { matrix_room.matrix_to_event_permalink(event_id_clone).await }
            );
        match handle.await.expect("task was not aborted") {
            Ok(permalink) => {
                return permalink;
            }
            Err(error) => {
                error!("Could not get room event permalink: {error}");
            }
        }

        // Fallback to using just the room ID, without routing.
        self.room_id().matrix_to_event_uri(event_id)
    }

    /// Constructs an `AtRoom` for this room.
    pub(crate) fn at_room(&self) -> AtRoom {
        AtRoom::new(self)
    }

    /// Get or create the list of members of this room.
    ///
    /// This creates the [`MemberList`] if no strong reference to it exists.
    pub(crate) fn get_or_create_members(&self) -> MemberList {
        let members = &self.imp().members;
        if let Some(list) = members.upgrade() {
            list
        } else {
            let list = MemberList::new(self);
            members.set(Some(&list));
            self.notify_members();
            list
        }
    }

    /// Change the category of this room.
    ///
    /// This makes the necessary to propagate the category to the homeserver.
    ///
    /// This can be used to trigger actions like join or leave, as well as
    /// changing the category in the sidebar.
    ///
    /// Note that rooms cannot change category once they are upgraded.
    pub(crate) async fn change_category(&self, category: TargetRoomCategory) -> MatrixResult<()> {
        self.imp().change_category(category).await
    }

    /// Toggle the `key` reaction on the given related event in this room.
    pub(crate) async fn toggle_reaction(&self, key: String, event: &Event) -> Result<(), ()> {
        // Whether this adds the reaction rather than taking it back. Taking one
        // back is not a use of the emoji, and has to be read before the toggle.
        let is_adding = !event
            .reactions()
            .reaction_group_by_key(&key)
            .is_some_and(|group| group.has_own_user());

        // Use the timeline of the event: it might be a focused timeline rather than
        // the live one, and the SDK can only react to an event it knows about.
        let matrix_timeline = event.timeline().matrix_timeline();
        let identifier = event.identifier();
        let key_clone = key.clone();

        let handle = spawn_tokio!(async move {
            matrix_timeline
                .toggle_reaction(&identifier, &key_clone)
                .await
        });

        if let Err(error) = handle.await.expect("task was not aborted") {
            error!("Could not toggle reaction: {error}");
            return Err(());
        }

        if is_adding && let Some(session) = self.session() {
            session.global_account_data().record_emoji_use(&key).await;
        }

        Ok(())
    }

    /// Send the given receipt.
    ///
    /// This will also unmark the room as unread.
    pub(crate) async fn send_receipt(
        &self,
        receipt_type: ApiReceiptType,
        position: ReceiptPosition,
    ) {
        self.live_timeline()
            .send_receipt(receipt_type, position)
            .await;
    }

    /// Mark the room as unread.
    pub(crate) async fn mark_as_unread(&self) {
        let core = self.core().clone();
        spawn_tokio!(async move { core.mark_as_unread().await })
            .await
            .expect("task was not aborted");
    }

    /// Send a typing notification for this room, with the given typing state.
    pub(crate) fn send_typing_notification(&self, is_typing: bool) {
        self.core().send_typing_notification(is_typing);
    }

    /// Redact the given events in this room because of the given reason.
    ///
    /// Returns `Ok(())` if all the redactions are successful, otherwise
    /// returns the list of events that could not be redacted.
    pub(crate) async fn redact<'a>(
        &self,
        events: &'a [OwnedEventId],
        reason: Option<String>,
    ) -> Result<(), Vec<&'a EventId>> {
        let matrix_room = self.matrix_room();
        if matrix_room.state() != RoomState::Joined {
            return Ok(());
        }

        let events_clone = events.to_owned();
        let matrix_room = matrix_room.clone();
        let handle = spawn_tokio!(async move {
            let mut failed_redactions = Vec::new();

            for (i, event_id) in events_clone.iter().enumerate() {
                match matrix_room.redact(event_id, reason.as_deref(), None).await {
                    Ok(_) => {}
                    Err(error) => {
                        error!("Could not redact event with ID {event_id}: {error}");
                        failed_redactions.push(i);
                    }
                }
            }

            failed_redactions
        });

        let failed_redactions = handle.await.expect("task was not aborted");
        let failed_redactions = failed_redactions
            .into_iter()
            .map(|i| &*events[i])
            .collect::<Vec<_>>();

        if failed_redactions.is_empty() {
            Ok(())
        } else {
            Err(failed_redactions)
        }
    }

    /// Report the given events in this room.
    ///
    /// The events are a list of `(event_id, reason)` tuples.
    ///
    /// Returns `Ok(())` if all the reports are sent successfully, otherwise
    /// returns the list of event IDs that could not be reported.
    pub(crate) async fn report_events<'a>(
        &self,
        events: &'a [(OwnedEventId, Option<String>)],
    ) -> Result<(), Vec<&'a EventId>> {
        let events_clone = events.to_owned();
        let matrix_room = self.matrix_room().clone();
        let handle = spawn_tokio!(async move {
            let futures = events_clone
                .into_iter()
                .map(|(event_id, reason)| matrix_room.report_content(event_id, reason));
            futures_util::future::join_all(futures).await
        });

        let mut failed = Vec::new();
        for (index, result) in handle
            .await
            .expect("task was not aborted")
            .iter()
            .enumerate()
        {
            match result {
                Ok(_) => {}
                Err(error) => {
                    error!(
                        "Could not report content with event ID {}: {error}",
                        events[index].0,
                    );
                    failed.push(&*events[index].0);
                }
            }
        }

        if failed.is_empty() {
            Ok(())
        } else {
            Err(failed)
        }
    }

    /// Report this room to the administrator of our homeserver.
    ///
    /// The reason may be empty. We do not have to be joined to report a room.
    pub(crate) async fn report(&self, reason: String) -> Result<(), ()> {
        let matrix_room = self.matrix_room().clone();
        let handle = spawn_tokio!(async move { matrix_room.report_room(reason).await });

        match handle.await.expect("task was not aborted") {
            Ok(_) => Ok(()),
            Err(error) => {
                error!("Could not report room {}: {error}", self.room_id());
                Err(())
            }
        }
    }

    /// The server ACL of this room, if it has one.
    ///
    /// A room without an ACL is not restricted at all, which is different from
    /// an ACL we could not load, hence the nested result.
    pub(crate) async fn server_acl(&self) -> Result<Option<RoomServerAclEventContent>, ()> {
        let matrix_room = self.matrix_room().clone();
        let handle = spawn_tokio!(async move {
            matrix_room
                .get_state_event_static::<RoomServerAclEventContent>()
                .await
        });

        let raw_event = match handle.await.expect("task was not aborted") {
            Ok(Some(RawSyncOrStrippedState::Sync(raw_event))) => raw_event,
            // A room we were never in does not hand us its ACL.
            Ok(_) => return Ok(None),
            Err(error) => {
                error!("Could not get server ACL event: {error}");
                return Err(());
            }
        };

        match raw_event.deserialize() {
            Ok(SyncStateEvent::Original(event)) => Ok(Some(event.content)),
            // A redacted event has no content. An ACL should never be redacted,
            // it is in `NON_REDACTABLE_EVENTS`, but a remote one might be.
            Ok(_) => Ok(None),
            Err(error) => {
                error!("Could not deserialize server ACL event: {error}");
                Err(())
            }
        }
    }

    /// Set the server ACL of this room.
    pub(crate) async fn set_server_acl(
        &self,
        content: RoomServerAclEventContent,
    ) -> Result<(), ()> {
        let matrix_room = self.matrix_room().clone();
        let handle = spawn_tokio!(async move { matrix_room.send_state_event(content).await });

        match handle.await.expect("task was not aborted") {
            Ok(_) => Ok(()),
            Err(error) => {
                error!("Could not change server ACL: {error}");
                Err(())
            }
        }
    }

    /// Invite the given users to this room.
    ///
    /// Returns `Ok(())` if all the invites are sent successfully, otherwise
    /// returns the list of users who could not be invited.
    pub(crate) async fn invite<'a>(
        &self,
        user_ids: &'a [OwnedUserId],
    ) -> Result<(), Vec<&'a UserId>> {
        let matrix_room = self.matrix_room();
        if matrix_room.state() != RoomState::Joined {
            error!("Can’t invite users, because this room isn’t a joined room");
            return Ok(());
        }

        let user_ids_clone = user_ids.to_owned();
        let matrix_room = matrix_room.clone();
        let handle = spawn_tokio!(async move {
            let invitations = user_ids_clone
                .iter()
                .map(|user_id| matrix_room.invite_user_by_id(user_id));
            futures_util::future::join_all(invitations).await
        });

        let mut failed_invites = Vec::new();
        for (index, result) in handle
            .await
            .expect("task was not aborted")
            .iter()
            .enumerate()
        {
            match result {
                Ok(()) => {}
                Err(error) => {
                    error!("Could not invite user with ID {}: {error}", user_ids[index],);
                    failed_invites.push(&*user_ids[index]);
                }
            }
        }

        if failed_invites.is_empty() {
            Ok(())
        } else {
            Err(failed_invites)
        }
    }

    /// Kick the given users from this room.
    ///
    /// The users are a list of `(user_id, reason)` tuples.
    ///
    /// Returns `Ok(())` if all the kicks are sent successfully, otherwise
    /// returns the list of users who could not be kicked.
    pub(crate) async fn kick<'a>(
        &self,
        users: &'a [(OwnedUserId, Option<String>)],
    ) -> Result<(), Vec<&'a UserId>> {
        let users_clone = users.to_owned();
        let matrix_room = self.matrix_room().clone();
        let handle = spawn_tokio!(async move {
            let futures = users_clone
                .iter()
                .map(|(user_id, reason)| matrix_room.kick_user(user_id, reason.as_deref()));
            futures_util::future::join_all(futures).await
        });

        let mut failed_kicks = Vec::new();
        for (index, result) in handle
            .await
            .expect("task was not aborted")
            .iter()
            .enumerate()
        {
            match result {
                Ok(()) => {}
                Err(error) => {
                    error!("Could not kick user with ID {}: {error}", users[index].0);
                    failed_kicks.push(&*users[index].0);
                }
            }
        }

        if failed_kicks.is_empty() {
            Ok(())
        } else {
            Err(failed_kicks)
        }
    }

    /// Ban the given users from this room.
    ///
    /// The users are a list of `(user_id, reason)` tuples.
    ///
    /// Returns `Ok(())` if all the bans are sent successfully, otherwise
    /// returns the list of users who could not be banned.
    pub(crate) async fn ban<'a>(
        &self,
        users: &'a [(OwnedUserId, Option<String>)],
    ) -> Result<(), Vec<&'a UserId>> {
        let users_clone = users.to_owned();
        let matrix_room = self.matrix_room().clone();
        let handle = spawn_tokio!(async move {
            let futures = users_clone
                .iter()
                .map(|(user_id, reason)| matrix_room.ban_user(user_id, reason.as_deref()));
            futures_util::future::join_all(futures).await
        });

        let mut failed_bans = Vec::new();
        for (index, result) in handle
            .await
            .expect("task was not aborted")
            .iter()
            .enumerate()
        {
            match result {
                Ok(()) => {}
                Err(error) => {
                    error!("Could not ban user with ID {}: {error}", users[index].0);
                    failed_bans.push(&*users[index].0);
                }
            }
        }

        if failed_bans.is_empty() {
            Ok(())
        } else {
            Err(failed_bans)
        }
    }

    /// Unban the given users from this room.
    ///
    /// The users are a list of `(user_id, reason)` tuples.
    ///
    /// Returns `Ok(())` if all the unbans are sent successfully, otherwise
    /// returns the list of users who could not be unbanned.
    pub(crate) async fn unban<'a>(
        &self,
        users: &'a [(OwnedUserId, Option<String>)],
    ) -> Result<(), Vec<&'a UserId>> {
        let users_clone = users.to_owned();
        let matrix_room = self.matrix_room().clone();
        let handle = spawn_tokio!(async move {
            let futures = users_clone
                .iter()
                .map(|(user_id, reason)| matrix_room.unban_user(user_id, reason.as_deref()));
            futures_util::future::join_all(futures).await
        });

        let mut failed_unbans = Vec::new();
        for (index, result) in handle
            .await
            .expect("task was not aborted")
            .iter()
            .enumerate()
        {
            match result {
                Ok(()) => {}
                Err(error) => {
                    error!("Could not unban user with ID {}: {error}", users[index].0);
                    failed_unbans.push(&*users[index].0);
                }
            }
        }

        if failed_unbans.is_empty() {
            Ok(())
        } else {
            Err(failed_unbans)
        }
    }

    /// Enable encryption for this room.
    pub(crate) async fn enable_encryption(&self) -> Result<(), ()> {
        if self.is_encrypted() {
            // Nothing to do.
            return Ok(());
        }

        let matrix_room = self.matrix_room().clone();
        let handle = spawn_tokio!(async move { matrix_room.enable_encryption().await });

        match handle.await.expect("task was not aborted") {
            Ok(()) => Ok(()),
            Err(error) => {
                error!("Could not enable room encryption: {error}");
                Err(())
            }
        }
    }

    /// Forget a room that is left.
    pub(crate) async fn forget(&self) -> MatrixResult<()> {
        if self.category() != RoomCategory::Left {
            warn!("Cannot forget a room that is not left");
            return Ok(());
        }

        // Through the core, whose list is the one the sidebar presents: it
        // drops the room when the room says it is forgotten.
        let core = self.core().clone();
        let handle = spawn_tokio!(async move { core.forget().await });

        match handle.await.expect("task was not aborted") {
            Ok(()) => {
                self.emit_by_name::<()>("room-forgotten", &[]);
                Ok(())
            }
            Err(error) => {
                error!("Could not forget the room: {error}");
                Err(error)
            }
        }
    }

    /// Connect to the signal emitted when the room was forgotten.
    /// Whether the event with the given ID is pinned in this room.
    pub(crate) fn is_pinned(&self, event_id: &EventId) -> bool {
        self.core().is_pinned(event_id)
    }

    /// Pin the event with the given ID in this room.
    pub(crate) async fn pin_event(&self, event_id: OwnedEventId) -> Result<(), ()> {
        let matrix_room = self.matrix_room().clone();
        let handle = spawn_tokio!(async move { matrix_room.pin_event(&event_id).await });

        match handle.await.expect("task was not aborted") {
            Ok(_) => Ok(()),
            Err(error) => {
                error!("Could not pin event: {error}");
                Err(())
            }
        }
    }

    /// Unpin the event with the given ID in this room.
    pub(crate) async fn unpin_event(&self, event_id: OwnedEventId) -> Result<(), ()> {
        let matrix_room = self.matrix_room().clone();
        let handle = spawn_tokio!(async move { matrix_room.unpin_event(&event_id).await });

        match handle.await.expect("task was not aborted") {
            Ok(_) => Ok(()),
            Err(error) => {
                error!("Could not unpin event: {error}");
                Err(())
            }
        }
    }

    /// Connect to the signal emitted when the pinned events of this room
    /// change.
    pub(crate) fn connect_pinned_events_changed<F: Fn(&Self) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "pinned-events-changed",
            true,
            closure_local!(move |obj: Self| {
                f(&obj);
            }),
        )
    }
}

/// Supported values for the history visibility.
#[derive(Debug, Default, Hash, Eq, PartialEq, Clone, Copy, glib::Enum)]
#[enum_type(name = "HistoryVisibilityValue")]
pub enum HistoryVisibilityValue {
    /// Anyone can read.
    WorldReadable,
    /// Members, since this was selected.
    #[default]
    Shared,
    /// Members, since they were invited.
    Invited,
    /// Members, since they joined.
    Joined,
    /// Unsupported value.
    Unsupported,
}

impl From<commune_core::session::HistoryVisibilityValue> for HistoryVisibilityValue {
    fn from(value: commune_core::session::HistoryVisibilityValue) -> Self {
        use commune_core::session::HistoryVisibilityValue as Core;

        match value {
            Core::WorldReadable => Self::WorldReadable,
            Core::Shared => Self::Shared,
            Core::Invited => Self::Invited,
            Core::Joined => Self::Joined,
            Core::Unsupported => Self::Unsupported,
        }
    }
}

impl From<HistoryVisibility> for HistoryVisibilityValue {
    fn from(value: HistoryVisibility) -> Self {
        match value {
            HistoryVisibility::Invited => Self::Invited,
            HistoryVisibility::Joined => Self::Joined,
            HistoryVisibility::Shared => Self::Shared,
            HistoryVisibility::WorldReadable => Self::WorldReadable,
            _ => Self::Unsupported,
        }
    }
}

impl From<HistoryVisibilityValue> for HistoryVisibility {
    fn from(value: HistoryVisibilityValue) -> Self {
        match value {
            HistoryVisibilityValue::Invited => Self::Invited,
            HistoryVisibilityValue::Joined => Self::Joined,
            HistoryVisibilityValue::Shared => Self::Shared,
            HistoryVisibilityValue::WorldReadable => Self::WorldReadable,
            HistoryVisibilityValue::Unsupported => unimplemented!(),
        }
    }
}

/// The position of the receipt to send.
#[derive(Debug, Clone)]
pub(crate) enum ReceiptPosition {
    /// We are at the end of the timeline (bottom of the view).
    End,
    /// We are at the event with the given ID.
    Event(OwnedEventId),
}

impl From<ReceiptPosition> for commune_core::session::ReceiptPosition {
    fn from(value: ReceiptPosition) -> Self {
        match value {
            ReceiptPosition::End => Self::End,
            ReceiptPosition::Event(event_id) => Self::Event(event_id),
        }
    }
}
