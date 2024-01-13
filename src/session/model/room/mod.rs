mod event;
mod highlight_flags;
mod member;
mod member_list;
mod member_role;
mod permissions;
mod room_type;
mod timeline;
mod typing_list;

use std::{cell::RefCell, io::Cursor};

use futures_util::StreamExt;
use gettextrs::gettext;
use gtk::{
    glib,
    glib::{clone, closure_local},
    prelude::*,
    subclass::prelude::*,
};
use matrix_sdk::{
    attachment::{generate_image_thumbnail, AttachmentConfig, AttachmentInfo, Thumbnail},
    deserialized_responses::{MemberEvent, SyncTimelineEvent},
    event_handler::EventHandlerDropGuard,
    room::Room as MatrixRoom,
    sync::{JoinedRoom, LeftRoom},
    DisplayName, HttpError, Result as MatrixResult, RoomInfo, RoomMemberships, RoomState,
};
use ruma::{
    events::{
        reaction::ReactionEventContent,
        receipt::{ReceiptEventContent, ReceiptType},
        relation::Annotation,
        room::{
            encryption::SyncRoomEncryptionEvent,
            join_rules::{AllowRule, JoinRule},
        },
        tag::{TagInfo, TagName},
        typing::TypingEventContent,
        AnyMessageLikeEventContent, AnyRoomAccountDataEvent, AnySyncStateEvent,
        AnySyncTimelineEvent, SyncEphemeralRoomEvent, SyncStateEvent,
    },
    MatrixToUri, MatrixUri, OwnedEventId, OwnedRoomAliasId, OwnedRoomId, OwnedUserId, RoomId,
    UserId,
};
use tracing::{debug, error, warn};

pub use self::{
    event::*,
    highlight_flags::HighlightFlags,
    member::{Member, Membership},
    member_list::MemberList,
    member_role::MemberRole,
    permissions::{
        Permissions, PowerLevel, PowerLevelUserAction, POWER_LEVEL_MAX, POWER_LEVEL_MIN,
    },
    room_type::RoomType,
    timeline::*,
    typing_list::TypingList,
};
use super::{
    notifications::NotificationsRoomSetting, room_list::RoomMetainfo, AvatarData, AvatarImage,
    AvatarUriSource, IdentityVerification, Session, SidebarItem, SidebarItemImpl, User,
};
use crate::{components::Pill, gettext_f, prelude::*, spawn, spawn_tokio};

mod imp {
    use std::{
        cell::{Cell, OnceCell},
        marker::PhantomData,
    };

    use glib::subclass::Signal;
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Default, glib::Properties)]
    #[properties(wrapper_type = super::Room)]
    pub struct Room {
        /// The room API of the SDK.
        pub matrix_room: OnceCell<MatrixRoom>,
        /// The current session.
        #[property(get, set = Self::set_session, construct_only)]
        pub session: glib::WeakRef<Session>,
        /// The ID of this room, as a string.
        #[property(get = Self::room_id_string)]
        pub room_id_string: PhantomData<String>,
        /// The alias of this room, as a string.
        #[property(get = Self::alias_string)]
        pub alias_string: PhantomData<Option<String>>,
        /// The unique identifier to display for this room, as a string.
        ///
        /// Prefers the alias over the room ID.
        #[property(get = Self::identifier_string)]
        pub identifier_string: PhantomData<String>,
        /// The version of this room.
        #[property(get = Self::version)]
        pub version: PhantomData<String>,
        /// Whether this room is federated.
        #[property(get = Self::federated)]
        pub federated: PhantomData<bool>,
        /// The name that is set for this room.
        ///
        /// This can be empty, the display name should be used instead in the
        /// interface.
        #[property(get = Self::name)]
        pub name: PhantomData<Option<String>>,
        /// The display name of this room.
        #[property(get = Self::display_name, type = String)]
        pub display_name: RefCell<Option<String>>,
        /// Whether this room has an avatar explicitly set.
        ///
        /// This is `false` if there is no avatar or if the avatar is the one
        /// from the other member.
        #[property(get)]
        pub has_avatar: Cell<bool>,
        /// The Avatar data of this room.
        #[property(get)]
        pub avatar_data: AvatarData,
        /// The category of this room.
        #[property(get, builder(RoomType::default()))]
        pub category: Cell<RoomType>,
        /// The timeline of this room.
        #[property(get)]
        pub timeline: OnceCell<Timeline>,
        /// The member corresponding to our own user.
        #[property(get)]
        pub own_member: OnceCell<Member>,
        /// The members of this room.
        #[property(get)]
        pub members: glib::WeakRef<MemberList>,
        /// The number of joined members in the room, according to the
        /// homeserver.
        #[property(get)]
        pub joined_members_count: Cell<u64>,
        /// The user who sent the invite to this room.
        ///
        /// This is only set when this room is an invitation.
        #[property(get)]
        pub inviter: RefCell<Option<Member>>,
        /// The permissions of our own user in this room
        #[property(get)]
        pub permissions: Permissions,
        /// The timestamp of the room's latest activity.
        ///
        /// This is the timestamp of the latest event that counts as possibly
        /// unread.
        ///
        /// If it is not known, it will return `0`.
        #[property(get)]
        pub latest_activity: Cell<u64>,
        /// Whether all messages of this room are read.
        #[property(get)]
        pub is_read: Cell<bool>,
        /// The highlight state of the room.
        #[property(get)]
        pub highlight: Cell<HighlightFlags>,
        /// The ID of the room that was upgraded and that this one replaces.
        pub predecessor_id: OnceCell<OwnedRoomId>,
        /// The ID of the room that was upgraded and that this one replaces, as
        /// a string.
        #[property(get = Self::predecessor_id_string)]
        pub predecessor_id_string: PhantomData<Option<String>>,
        /// The ID of the successor of this Room, if this room was upgraded.
        pub successor_id: OnceCell<OwnedRoomId>,
        /// The ID of the successor of this Room, if this room was upgraded, as
        /// a string.
        #[property(get = Self::successor_id_string)]
        pub successor_id_string: PhantomData<Option<String>>,
        /// The successor of this Room, if this room was upgraded and the
        /// successor was joined.
        #[property(get)]
        pub successor: glib::WeakRef<super::Room>,
        /// The most recent verification request event.
        #[property(get, set)]
        pub verification: RefCell<Option<IdentityVerification>>,
        /// Whether this room is encrypted.
        #[property(get)]
        pub encrypted: Cell<bool>,
        /// The list of members currently typing in this room.
        #[property(get)]
        pub typing_list: TypingList,
        /// Whether this room is a direct chat.
        #[property(get)]
        pub is_direct: Cell<bool>,
        /// The other member of the room, if this room is a direct chat and
        /// there is only one other member.
        #[property(get)]
        pub direct_member: RefCell<Option<Member>>,
        /// The number of unread notifications of this room.
        #[property(get = Self::notification_count)]
        pub notification_count: PhantomData<u64>,
        /// The topic of this room.
        #[property(get = Self::topic)]
        pub topic: PhantomData<Option<String>>,
        /// Whether this room has been upgraded.
        #[property(get = Self::is_tombstoned)]
        pub is_tombstoned: PhantomData<bool>,
        /// The notifications settings for this room.
        #[property(get, set = Self::set_notifications_setting, explicit_notify, builder(NotificationsRoomSetting::default()))]
        pub notifications_setting: Cell<NotificationsRoomSetting>,
        /// Whether anyone can join this room.
        #[property(get = Self::anyone_can_join)]
        pub anyone_can_join: PhantomData<bool>,
        pub typing_drop_guard: OnceCell<EventHandlerDropGuard>,
        pub receipts_drop_guard: OnceCell<EventHandlerDropGuard>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Room {
        const NAME: &'static str = "Room";
        type Type = super::Room;
        type ParentType = SidebarItem;
    }

    #[glib::derived_properties]
    impl ObjectImpl for Room {
        fn signals() -> &'static [Signal] {
            static SIGNALS: Lazy<Vec<Signal>> = Lazy::new(|| {
                vec![
                    Signal::builder("room-forgotten").build(),
                    Signal::builder("join-rule-changed").build(),
                ]
            });
            SIGNALS.as_ref()
        }

        fn constructed(&self) {
            self.parent_constructed();

            self.obj()
                .bind_property("display-name", &self.avatar_data, "display-name")
                .sync_create()
                .build();
        }
    }

    impl SidebarItemImpl for Room {}

    impl Room {
        /// Set the current session
        fn set_session(&self, session: Session) {
            self.session.set(Some(&session));

            let own_member = Member::new(&self.obj(), session.user_id().clone());
            self.own_member.set(own_member).unwrap();
        }

        /// The room API of the SDK.
        pub fn matrix_room(&self) -> &MatrixRoom {
            self.matrix_room.get().unwrap()
        }

        /// The room ID of this room, as a string.
        fn room_id_string(&self) -> String {
            self.matrix_room().room_id().to_string()
        }

        /// The alias of this room.
        pub(super) fn alias(&self) -> Option<OwnedRoomAliasId> {
            let matrix_room = self.matrix_room();
            matrix_room
                .canonical_alias()
                .or_else(|| matrix_room.alt_aliases().into_iter().next())
        }

        /// The alias of this room, as a string.
        fn alias_string(&self) -> Option<String> {
            self.alias().map(Into::into)
        }

        /// The unique identifier to display for this room, as a string.
        fn identifier_string(&self) -> String {
            self.alias_string().unwrap_or_else(|| self.room_id_string())
        }

        /// The version of this room.
        fn version(&self) -> String {
            self.matrix_room()
                .create_content()
                .map(|c| c.room_version.to_string())
                .unwrap_or_default()
        }

        /// Whether this room is federated.
        fn federated(&self) -> bool {
            self.matrix_room()
                .create_content()
                .map(|c| c.federate)
                .unwrap_or_default()
        }

        /// The name of this room.
        ///
        /// This can be empty, the display name should be used instead in the
        /// interface.
        fn name(&self) -> Option<String> {
            self.matrix_room().name()
        }

        /// The display name of this room.
        fn display_name(&self) -> String {
            let display_name = self.display_name.borrow().clone();
            // Translators: This is displayed when the room name is unknown yet.
            display_name.unwrap_or_else(|| gettext("Unknown"))
        }

        /// Set whether this room has an avatar explicitly set.
        pub fn set_has_avatar(&self, has_avatar: bool) {
            if self.has_avatar.get() == has_avatar {
                return;
            }

            self.has_avatar.set(has_avatar);
            self.obj().notify_has_avatar();
        }

        /// The number of unread notifications of this room.
        fn notification_count(&self) -> u64 {
            self.matrix_room()
                .unread_notification_counts()
                .notification_count
        }

        /// The topic of this room.
        fn topic(&self) -> Option<String> {
            self.matrix_room().topic().filter(|topic| {
                !topic.is_empty() && topic.find(|c: char| !c.is_whitespace()).is_some()
            })
        }

        /// Whether this room was tombstoned.
        fn is_tombstoned(&self) -> bool {
            self.matrix_room().is_tombstoned()
        }

        /// The ID of the room that was upgraded and that this one replaces, as
        /// a string.
        fn predecessor_id_string(&self) -> Option<String> {
            self.predecessor_id.get().map(ToString::to_string)
        }

        /// The ID of the successor of this Room, if this room was upgraded.
        fn successor_id_string(&self) -> Option<String> {
            self.successor_id.get().map(ToString::to_string)
        }

        /// Set the notifications setting for this room.
        fn set_notifications_setting(&self, setting: NotificationsRoomSetting) {
            if self.notifications_setting.get() == setting {
                return;
            }

            self.notifications_setting.set(setting);
            self.obj().notify_notifications_setting();
        }

        /// Whether anyone can join this room.
        fn anyone_can_join(&self) -> bool {
            self.matrix_room().join_rule() == JoinRule::Public
        }
    }
}

glib::wrapper! {
    /// GObject representation of a Matrix room.
    ///
    /// Handles populating the Timeline.
    pub struct Room(ObjectSubclass<imp::Room>) @extends SidebarItem;
}

impl Room {
    pub fn new(session: &Session, matrix_room: MatrixRoom, metainfo: Option<RoomMetainfo>) -> Self {
        let this = glib::Object::builder::<Self>()
            .property("session", session)
            .build();

        this.set_matrix_room(matrix_room);

        if let Some(RoomMetainfo {
            latest_activity,
            is_read,
        }) = metainfo
        {
            this.set_latest_activity(latest_activity);
            this.set_is_read(is_read);

            this.update_highlight();
        }

        this
    }

    /// The room API of the SDK.
    pub fn matrix_room(&self) -> &MatrixRoom {
        self.imp().matrix_room()
    }

    /// Set the room API of the SDK.
    fn set_matrix_room(&self, matrix_room: MatrixRoom) {
        let imp = self.imp();

        self.set_joined_members_count(matrix_room.joined_members_count());

        imp.matrix_room.set(matrix_room).unwrap();

        self.update_avatar();
        self.load_predecessor();
        self.load_tombstone();
        self.load_category();
        self.set_up_receipts();
        self.set_up_typing();
        self.init_timeline();
        self.set_up_is_encrypted();

        spawn!(
            glib::Priority::DEFAULT_IDLE,
            clone!(@weak self as obj => async move {
                obj.load_display_name().await;
            })
        );

        spawn!(
            glib::Priority::DEFAULT_IDLE,
            clone!(@weak self as obj => async move {
                obj.load_own_member().await;
            })
        );

        spawn!(
            glib::Priority::DEFAULT_IDLE,
            clone!(@weak self as obj => async move {
                obj.load_is_direct().await;
            })
        );

        spawn!(
            glib::Priority::DEFAULT_IDLE,
            clone!(@weak self as obj => async move {
                obj.watch_room_info().await;
            })
        );

        spawn!(
            glib::Priority::DEFAULT_IDLE,
            clone!(@weak self as obj => async move {
                obj.load_inviter().await;
            })
        );

        spawn!(
            glib::Priority::DEFAULT_IDLE,
            clone!(@weak self as obj => async move {
                obj.imp().permissions.init(&obj).await;
            })
        );
    }

    fn init_timeline(&self) {
        let timeline = Timeline::new(self);
        self.imp().timeline.set(timeline.clone()).unwrap();

        timeline
            .sdk_items()
            .connect_items_changed(clone!(@weak self as obj => move |_, _, _, _| {
                spawn!(clone!(@weak obj => async move {
                    obj.update_is_read().await;
                }));
            }));

        if !matches!(self.category(), RoomType::Left | RoomType::Outdated) {
            // Load the room history when idle.
            spawn!(
                glib::source::Priority::LOW,
                clone!(@weak self as obj => async move {
                    obj.timeline().load().await;
                })
            );
        }
    }

    /// The ID of this room.
    pub fn room_id(&self) -> &RoomId {
        self.matrix_room().room_id()
    }

    /// The alias of this room.
    pub fn alias(&self) -> Option<OwnedRoomAliasId> {
        self.imp().alias()
    }

    /// The state of the room.
    pub fn state(&self) -> RoomState {
        self.matrix_room().state()
    }

    /// Set whether this room is a direct chat.
    fn set_is_direct(&self, is_direct: bool) {
        if self.is_direct() == is_direct {
            return;
        }

        self.imp().is_direct.set(is_direct);
        self.notify_is_direct();

        spawn!(clone!(@weak self as obj => async move {
            obj.load_direct_member().await;
        }));
    }

    /// Load whether the room is direct or not.
    pub async fn load_is_direct(&self) {
        let matrix_room = self.matrix_room().clone();
        let handle = spawn_tokio!(async move { matrix_room.is_direct().await });

        match handle.await.unwrap() {
            Ok(is_direct) => self.set_is_direct(is_direct),
            Err(error) => {
                error!(room_id = %self.room_id(), "Failed to load whether room is direct: {error}");
            }
        }
    }

    /// The ID of the other user, if this is a direct chat and there is only one
    /// other user.
    async fn direct_user_id(&self) -> Option<OwnedUserId> {
        let matrix_room = self.matrix_room();

        // Check if the room direct and if there only one target.
        let direct_targets = matrix_room.direct_targets();
        if direct_targets.len() != 1 {
            // It was a direct chat with several users.
            return None;
        }

        let direct_target_user_id = direct_targets.into_iter().next().unwrap();

        // Check that there are still at most 2 members.
        let members_count = matrix_room.active_members_count();

        if members_count > 2 {
            // We only want a 1-to-1 room. The count might be 1 if the other user left, but
            // we can reinvite them.
            return None;
        }

        // Check that the members count is correct. It might not be correct if the room
        // was just joined, or if it is in an invited state.
        let matrix_room_clone = matrix_room.clone();
        let handle =
            spawn_tokio!(async move { matrix_room_clone.members(RoomMemberships::ACTIVE).await });

        let members = match handle.await.unwrap() {
            Ok(m) => m,
            Err(error) => {
                error!("Failed to load room members: {error}");
                vec![]
            }
        };

        let members_count = members_count.max(members.len() as u64);
        if members_count > 2 {
            // Same as before.
            return None;
        }

        let own_user_id = matrix_room.own_user_id();
        // Get the other member from the list.
        for member in members {
            let user_id = member.user_id();

            if user_id != direct_target_user_id && user_id != own_user_id {
                // There is a non-direct member.
                return None;
            }
        }

        Some(direct_target_user_id)
    }

    /// Set the other member of the room, if this room is a direct chat and
    /// there is only one other member..
    fn set_direct_member(&self, member: Option<Member>) {
        if self.direct_member() == member {
            return;
        }

        self.imp().direct_member.replace(member);
        self.notify_direct_member();
        self.update_avatar();
    }

    /// Load the other member of the room, if this room is a direct chat and
    /// there is only one other member.
    async fn load_direct_member(&self) {
        let Some(direct_user_id) = self.direct_user_id().await else {
            self.set_direct_member(None);
            return;
        };

        if self
            .direct_member()
            .is_some_and(|m| *m.user_id() == direct_user_id)
        {
            // Already up-to-date.
            return;
        }

        let direct_member = if let Some(members) = self.members() {
            members.get_or_create(direct_user_id.clone())
        } else {
            Member::new(self, direct_user_id.clone())
        };

        let matrix_room = self.matrix_room().clone();
        let handle =
            spawn_tokio!(async move { matrix_room.get_member_no_sync(&direct_user_id).await });

        match handle.await.unwrap() {
            Ok(Some(matrix_member)) => {
                direct_member.update_from_room_member(&matrix_member);
            }
            Ok(None) => {}
            Err(error) => {
                error!("Failed to get direct member: {error}");
            }
        }

        self.set_direct_member(Some(direct_member));
    }

    /// Ensure the direct user of this room is an active member.
    ///
    /// If there is supposed to be a direct user in this room but they have left
    /// it, re-invite them.
    ///
    /// This is a noop if there is no supposed direct user or if the user is
    /// already an active member.
    pub async fn ensure_direct_user(&self) -> Result<(), ()> {
        let Some(member) = self.direct_member() else {
            warn!("Cannot ensure direct user in a room without direct target");
            return Ok(());
        };

        if self.matrix_room().active_members_count() == 2 {
            return Ok(());
        }

        self.invite(&[member.user_id().clone()])
            .await
            .map_err(|_| ())
    }

    /// Forget a room that is left.
    pub async fn forget(&self) -> MatrixResult<()> {
        if self.category() != RoomType::Left {
            warn!("Cannot forget a room that is not left");
            return Ok(());
        }

        let matrix_room = self.matrix_room().clone();
        let handle = spawn_tokio!(async move { matrix_room.forget().await });

        match handle.await.unwrap() {
            Ok(_) => {
                self.emit_by_name::<()>("room-forgotten", &[]);
                Ok(())
            }
            Err(error) => {
                error!("Couldn’t forget the room: {error}");

                // Load the previous category
                self.load_category();

                Err(error)
            }
        }
    }

    /// Whether this room is joined.
    pub fn is_joined(&self) -> bool {
        self.own_member().membership() == Membership::Join
    }

    fn set_category_internal(&self, category: RoomType) {
        let old_category = self.category();

        if old_category == RoomType::Outdated || old_category == category {
            return;
        }

        self.imp().category.set(category);
        self.notify_category();
    }

    /// Set the category of this room.
    ///
    /// This makes the necessary to propagate the category to the homeserver.
    ///
    /// Note: Rooms can't be moved to the invite category and they can't be
    /// moved once they are upgraded.
    pub async fn set_category(&self, category: RoomType) -> MatrixResult<()> {
        let previous_category = self.category();

        if previous_category == category {
            return Ok(());
        }

        if previous_category == RoomType::Outdated {
            warn!("Can't set the category of an upgraded room");
            return Ok(());
        }

        match category {
            RoomType::Invited => {
                warn!("Rooms can’t be moved to the invite Category");
                return Ok(());
            }
            RoomType::Outdated => {
                // Outdated rooms don't need to propagate anything to the server
                self.set_category_internal(category);
                return Ok(());
            }
            _ => {}
        }

        self.set_category_internal(category);

        let matrix_room = self.matrix_room().clone();
        let handle = spawn_tokio!(async move {
            match matrix_room.state() {
                RoomState::Invited => match category {
                    RoomType::Invited => {}
                    RoomType::Favorite => {
                        if let Some(tags) = matrix_room.tags().await? {
                            if !tags.contains_key(&TagName::Favorite) {
                                matrix_room
                                    .set_tag(TagName::Favorite, TagInfo::new())
                                    .await?;
                            }
                            if tags.contains_key(&TagName::LowPriority) {
                                matrix_room.remove_tag(TagName::LowPriority).await?;
                            }
                        }
                        matrix_room.join().await?;
                    }
                    RoomType::Normal => {
                        if let Some(tags) = matrix_room.tags().await? {
                            if tags.contains_key(&TagName::Favorite) {
                                matrix_room.remove_tag(TagName::Favorite).await?;
                            }
                            if tags.contains_key(&TagName::LowPriority) {
                                matrix_room.remove_tag(TagName::LowPriority).await?;
                            }
                        }

                        if matrix_room.is_direct().await.unwrap_or_default() {
                            matrix_room.set_is_direct(false).await?;
                        }

                        matrix_room.join().await?;
                    }
                    RoomType::LowPriority => {
                        if let Some(tags) = matrix_room.tags().await? {
                            if tags.contains_key(&TagName::Favorite) {
                                matrix_room.remove_tag(TagName::Favorite).await?;
                            }
                            if !tags.contains_key(&TagName::LowPriority) {
                                matrix_room
                                    .set_tag(TagName::LowPriority, TagInfo::new())
                                    .await?;
                            }
                        }
                        matrix_room.join().await?;
                    }
                    RoomType::Left => {
                        matrix_room.leave().await?;
                    }
                    RoomType::Outdated | RoomType::Space | RoomType::Ignored => unimplemented!(),
                },
                RoomState::Joined => match category {
                    RoomType::Invited => {}
                    RoomType::Favorite => {
                        matrix_room
                            .set_tag(TagName::Favorite, TagInfo::new())
                            .await?;
                        if previous_category == RoomType::LowPriority {
                            matrix_room.remove_tag(TagName::LowPriority).await?;
                        }
                    }
                    RoomType::Normal => match previous_category {
                        RoomType::Favorite => {
                            matrix_room.remove_tag(TagName::Favorite).await?;
                        }
                        RoomType::LowPriority => {
                            matrix_room.remove_tag(TagName::LowPriority).await?;
                        }
                        _ => {}
                    },
                    RoomType::LowPriority => {
                        matrix_room
                            .set_tag(TagName::LowPriority, TagInfo::new())
                            .await?;
                        if previous_category == RoomType::Favorite {
                            matrix_room.remove_tag(TagName::Favorite).await?;
                        }
                    }
                    RoomType::Left => {
                        matrix_room.leave().await?;
                    }
                    RoomType::Outdated | RoomType::Space | RoomType::Ignored => unimplemented!(),
                },
                RoomState::Left => match category {
                    RoomType::Invited => {}
                    RoomType::Favorite => {
                        if let Some(tags) = matrix_room.tags().await? {
                            if !tags.contains_key(&TagName::Favorite) {
                                matrix_room
                                    .set_tag(TagName::Favorite, TagInfo::new())
                                    .await?;
                            }
                            if tags.contains_key(&TagName::LowPriority) {
                                matrix_room.remove_tag(TagName::LowPriority).await?;
                            }
                        }
                        matrix_room.join().await?;
                    }
                    RoomType::Normal => {
                        if let Some(tags) = matrix_room.tags().await? {
                            if tags.contains_key(&TagName::Favorite) {
                                matrix_room.remove_tag(TagName::Favorite).await?;
                            }
                            if tags.contains_key(&TagName::LowPriority) {
                                matrix_room.remove_tag(TagName::LowPriority).await?;
                            }
                        }
                        matrix_room.join().await?;
                    }
                    RoomType::LowPriority => {
                        if let Some(tags) = matrix_room.tags().await? {
                            if tags.contains_key(&TagName::Favorite) {
                                matrix_room.remove_tag(TagName::Favorite).await?;
                            }
                            if !tags.contains_key(&TagName::LowPriority) {
                                matrix_room
                                    .set_tag(TagName::LowPriority, TagInfo::new())
                                    .await?;
                            }
                        }
                        matrix_room.join().await?;
                    }
                    RoomType::Left => {}
                    RoomType::Outdated | RoomType::Space | RoomType::Ignored => unimplemented!(),
                },
            }

            Result::<_, matrix_sdk::Error>::Ok(())
        });

        match handle.await.unwrap() {
            Ok(_) => Ok(()),
            Err(error) => {
                error!("Could not set the room category: {error}");

                // Load the previous category
                self.load_category();

                Err(error)
            }
        }
    }

    /// Load the category from the SDK.
    fn load_category(&self) {
        // Don't load the category if this room was upgraded
        if self.category() == RoomType::Outdated {
            return;
        }

        if self.inviter().is_some_and(|i| i.is_ignored()) {
            self.set_category_internal(RoomType::Ignored);
        }

        let matrix_room = self.matrix_room();
        match matrix_room.state() {
            RoomState::Joined => {
                if matrix_room.is_space() {
                    self.set_category_internal(RoomType::Space);
                } else {
                    let matrix_room = matrix_room.clone();
                    let tags = spawn_tokio!(async move { matrix_room.tags().await });

                    spawn!(
                        glib::Priority::DEFAULT_IDLE,
                        clone!(@weak self as obj => async move {
                            let mut category = RoomType::Normal;

                            if let Ok(Some(tags)) = tags.await.unwrap() {
                                if tags.get(&TagName::Favorite).is_some() {
                                    category = RoomType::Favorite;
                                } else if tags.get(&TagName::LowPriority).is_some() {
                                    category = RoomType::LowPriority;
                                }
                            }

                            obj.set_category_internal(category);
                        })
                    );
                }
            }
            RoomState::Invited => self.set_category_internal(RoomType::Invited),
            RoomState::Left => self.set_category_internal(RoomType::Left),
        };
    }

    async fn watch_room_info(&self) {
        let matrix_room = self.matrix_room();
        let subscriber = matrix_room.subscribe_info();

        let room_weak = glib::SendWeakRef::from(self.downgrade());
        subscriber
            .for_each(move |room_info| {
                let room_weak = room_weak.clone();
                async move {
                    let ctx = glib::MainContext::default();
                    ctx.spawn(async move {
                        spawn!(async move {
                            if let Some(obj) = room_weak.upgrade() {
                                obj.update_room_info(room_info)
                            }
                        });
                    });
                }
            })
            .await;
    }

    fn update_room_info(&self, room_info: RoomInfo) {
        self.set_joined_members_count(room_info.joined_members_count());
    }

    /// Start listening to typing events.
    fn set_up_typing(&self) {
        let imp = self.imp();
        if imp.typing_drop_guard.get().is_some() {
            // The event handler is already set up.
            return;
        }

        let matrix_room = self.matrix_room();
        if matrix_room.state() != RoomState::Joined {
            return;
        };

        let room_weak = glib::SendWeakRef::from(self.downgrade());
        let handle = matrix_room.add_event_handler(
            move |event: SyncEphemeralRoomEvent<TypingEventContent>| {
                let room_weak = room_weak.clone();
                async move {
                    let ctx = glib::MainContext::default();
                    ctx.spawn(async move {
                        spawn!(async move {
                            if let Some(obj) = room_weak.upgrade() {
                                obj.handle_typing_event(event.content)
                            }
                        });
                    });
                }
            },
        );

        let drop_guard = matrix_room.client().event_handler_drop_guard(handle);
        imp.typing_drop_guard.set(drop_guard).unwrap();
    }

    /// Start listening to read receipts events.
    fn set_up_receipts(&self) {
        let imp = self.imp();
        if imp.receipts_drop_guard.get().is_some() {
            // The event handler is already set up.
            return;
        }

        // Listen to changes in the read receipts.
        let matrix_room = self.matrix_room();
        let room_weak = glib::SendWeakRef::from(self.downgrade());
        let handle = matrix_room.add_event_handler(
            move |event: SyncEphemeralRoomEvent<ReceiptEventContent>| {
                let room_weak = room_weak.clone();
                async move {
                    let ctx = glib::MainContext::default();
                    ctx.spawn(async move {
                        spawn!(async move {
                            if let Some(obj) = room_weak.upgrade() {
                                obj.handle_receipt_event(event.content)
                            }
                        });
                    });
                }
            },
        );

        let drop_guard = matrix_room.client().event_handler_drop_guard(handle);
        imp.receipts_drop_guard.set(drop_guard).unwrap();
    }

    fn handle_receipt_event(&self, content: ReceiptEventContent) {
        let Some(session) = self.session() else {
            return;
        };
        let own_user_id = session.user_id();

        for (_event_id, receipts) in content.iter() {
            if let Some(users) = receipts.get(&ReceiptType::Read) {
                if users.contains_key(own_user_id) {
                    spawn!(clone!(@weak self as obj => async move {
                        obj.update_is_read().await;
                    }));
                }
            }
        }
    }

    fn handle_typing_event(&self, content: TypingEventContent) {
        let Some(session) = self.session() else {
            return;
        };
        let typing_list = &self.imp().typing_list;

        let Some(members) = self.members() else {
            // If we don't have a members list, the room is not shown so we don't need to
            // update the typing list.
            typing_list.update(vec![]);
            return;
        };

        let own_user_id = session.user_id();

        let members = content
            .user_ids
            .into_iter()
            .filter_map(|user_id| (user_id != *own_user_id).then(|| members.get_or_create(user_id)))
            .collect();

        typing_list.update(members);
    }

    /// Create and load our own member from the store.
    async fn load_own_member(&self) {
        let own_member = self.own_member();
        let user_id = own_member.user_id().clone();
        let matrix_room = self.matrix_room().clone();

        let handle = spawn_tokio!(async move { matrix_room.get_member_no_sync(&user_id).await });

        match handle.await.unwrap() {
            Ok(Some(matrix_member)) => own_member.update_from_room_member(&matrix_member),
            Ok(None) => {}
            Err(error) => error!(
                "Failed to load own member for room {}: {error}",
                self.room_id()
            ),
        }
    }

    /// The members of this room.
    ///
    /// This creates the [`MemberList`] if no strong reference to it exists.
    pub fn get_or_create_members(&self) -> MemberList {
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

    /// Set the number of joined members in the room, according to the
    /// homeserver.
    fn set_joined_members_count(&self, count: u64) {
        if self.joined_members_count() == count {
            return;
        }

        self.imp().joined_members_count.set(count);
        self.notify_joined_members_count();
    }

    fn update_highlight(&self) {
        let mut highlight = HighlightFlags::empty();

        if matches!(self.category(), RoomType::Left) {
            // Consider that all left rooms are read.
            self.set_highlight(highlight);
            return;
        }

        let counts = self.matrix_room().unread_notification_counts();

        if counts.highlight_count > 0 {
            highlight = HighlightFlags::all();
        } else if counts.notification_count > 0 || !self.is_read() {
            highlight = HighlightFlags::BOLD;
        }

        self.set_highlight(highlight);
    }

    /// Set how this room is highlighted.
    fn set_highlight(&self, highlight: HighlightFlags) {
        if self.highlight() == highlight {
            return;
        }

        self.imp().highlight.set(highlight);
        self.notify_highlight();
    }

    async fn update_is_read(&self) {
        if let Some(has_unread) = self.timeline().has_unread_messages().await {
            self.set_is_read(!has_unread);
        }

        self.update_highlight();
    }

    /// Set whether all messages of this room are read.
    fn set_is_read(&self, is_read: bool) {
        if is_read == self.is_read() {
            return;
        }

        self.imp().is_read.set(is_read);
        self.notify_is_read();
    }

    /// Set the display name of this room.
    fn set_display_name(&self, display_name: Option<String>) {
        if Some(self.display_name()) == display_name {
            return;
        }

        self.imp().display_name.replace(display_name);
        self.notify_display_name();
    }

    /// Load the display name from the SDK.
    async fn load_display_name(&self) {
        let matrix_room = self.matrix_room().clone();
        let handle = spawn_tokio!(async move { matrix_room.display_name().await });

        // FIXME: We should retry if the request failed
        match handle.await.unwrap() {
            Ok(display_name) => {
                let name = match display_name {
                    DisplayName::Named(s)
                    | DisplayName::Calculated(s)
                    | DisplayName::Aliased(s) => s,
                    // Translators: This is the name of a room that is empty but had another user
                    // before. Do NOT translate the content between '{' and '}',
                    // this is a variable name.
                    DisplayName::EmptyWas(s) => {
                        gettext_f("Empty Room (was {user})", &[("user", &s)])
                    }
                    // Translators: This is the name of a room without other users.
                    DisplayName::Empty => gettext("Empty Room"),
                };
                self.set_display_name(Some(name))
            }
            Err(error) => error!("Couldn’t fetch display name: {error}"),
        };
    }

    /// Load the member that invited us to this room, when applicable.
    async fn load_inviter(&self) {
        let Some(session) = self.session() else {
            return;
        };

        let matrix_room = self.matrix_room();

        if matrix_room.state() != RoomState::Invited {
            return;
        }

        let own_user_id = session.user_id().clone();
        let matrix_room_clone = matrix_room.clone();
        let handle =
            spawn_tokio!(async move { matrix_room_clone.get_member_no_sync(&own_user_id).await });

        let own_member = match handle.await.unwrap() {
            Ok(Some(member)) => member,
            Ok(None) => return,
            Err(error) => {
                error!("Failed to get room member: {error}");
                return;
            }
        };

        let inviter_id = match &**own_member.event() {
            MemberEvent::Sync(_) => return,
            MemberEvent::Stripped(event) => event.sender.clone(),
        };

        let inviter_id_clone = inviter_id.clone();
        let matrix_room = matrix_room.clone();
        let handle =
            spawn_tokio!(async move { matrix_room.get_member_no_sync(&inviter_id_clone).await });

        let inviter_member = match handle.await.unwrap() {
            Ok(Some(member)) => member,
            Ok(None) => return,
            Err(error) => {
                error!("Failed to get room member: {error}");
                return;
            }
        };

        let inviter = Member::new(self, inviter_id);
        inviter.update_from_room_member(&inviter_member);

        inviter.upcast_ref::<User>().connect_is_ignored_notify(
            clone!(@weak self as obj => move |_| {
                obj.load_category();
            }),
        );

        self.imp().inviter.replace(Some(inviter));

        self.notify_inviter();
        self.load_category();
    }

    /// Update the room state based on the new sync response
    /// FIXME: We should use the sdk's event handler to get updates
    pub fn update_for_events(&self, batch: Vec<SyncTimelineEvent>) {
        // FIXME: notify only when the count has changed
        self.notify_notification_count();

        let events: Vec<_> = batch
            .iter()
            .flat_map(|e| e.event.deserialize().ok())
            .collect();
        let own_member = self.own_member();
        let own_user_id = own_member.user_id();
        let direct_member = self.direct_member();
        let direct_member_id = direct_member.as_ref().map(|m| m.user_id());

        for event in events.iter() {
            if let AnySyncTimelineEvent::State(state_event) = event {
                match state_event {
                    AnySyncStateEvent::RoomMember(SyncStateEvent::Original(event)) => {
                        if let Some(members) = self.members() {
                            members.update_member_for_member_event(event);
                        } else if event.state_key == *own_user_id {
                            own_member.update_from_member_event(event);
                        } else if Some(&event.state_key) == direct_member_id {
                            if let Some(member) = &direct_member {
                                member.update_from_member_event(event);
                            }
                        }

                        // It might change the direct member.
                        spawn!(clone!(@weak self as obj => async move {
                            obj.load_direct_member().await;
                            obj.load_display_name().await;
                        }));
                    }
                    AnySyncStateEvent::RoomAvatar(SyncStateEvent::Original(_)) => {
                        self.update_avatar();
                    }
                    AnySyncStateEvent::RoomName(_) => {
                        self.notify_name();
                        spawn!(clone!(@weak self as obj => async move {
                            obj.load_display_name().await;
                        }));
                    }
                    AnySyncStateEvent::RoomTopic(_) => {
                        self.notify_topic();
                    }
                    AnySyncStateEvent::RoomTombstone(_) => {
                        self.load_tombstone();
                    }
                    AnySyncStateEvent::RoomJoinRules(_) => {
                        self.emit_by_name::<()>("join-rule-changed", &[]);
                        self.notify_anyone_can_join();
                    }
                    AnySyncStateEvent::RoomCanonicalAlias(_) => {
                        self.notify_alias_string();
                    }
                    _ => {}
                }
            }
        }
    }

    /// Set the timestamp of the room's latest possibly unread event.
    fn set_latest_activity(&self, latest_activity: u64) {
        if latest_activity == self.latest_activity() {
            return;
        }

        self.imp().latest_activity.set(latest_activity);
        self.notify_latest_activity();
    }

    /// Send a message with the given `content` in this room.
    pub fn send_room_message_event(&self, content: impl Into<AnyMessageLikeEventContent>) {
        let timeline = self.timeline().matrix_timeline();
        let content = content.into();

        let handle = spawn_tokio!(async move { timeline.send(content).await });

        spawn!(
            glib::Priority::DEFAULT_IDLE,
            clone!(@weak self as obj => async move {
                handle.await.unwrap();
            })
        );
    }

    /// Send a `key` reaction for the `relates_to` event ID in this room.
    pub async fn send_reaction(&self, key: String, relates_to: OwnedEventId) -> MatrixResult<()> {
        let matrix_room = self.matrix_room().clone();

        spawn_tokio!(async move {
            matrix_room
                .send(ReactionEventContent::new(Annotation::new(relates_to, key)))
                .await
        })
        .await
        .unwrap()?;

        Ok(())
    }

    /// Redact `redacted_event_id` in this room because of `reason`.
    pub async fn redact(
        &self,
        redacted_event_id: OwnedEventId,
        reason: Option<String>,
    ) -> Result<(), HttpError> {
        let matrix_room = self.matrix_room();
        if matrix_room.state() != RoomState::Joined {
            return Ok(());
        };

        let matrix_room = matrix_room.clone();
        let handle = spawn_tokio!(async move {
            matrix_room
                .redact(&redacted_event_id, reason.as_deref(), None)
                .await
        });

        // FIXME: We should retry the request if it fails
        handle.await.unwrap()?;

        Ok(())
    }

    pub fn send_typing_notification(&self, is_typing: bool) {
        let matrix_room = self.matrix_room();
        if matrix_room.state() != RoomState::Joined {
            return;
        };

        let matrix_room = matrix_room.clone();
        let handle = spawn_tokio!(async move { matrix_room.typing_notice(is_typing).await });

        spawn!(
            glib::Priority::DEFAULT_IDLE,
            clone!(@weak self as obj => async move {
                match handle.await.unwrap() {
                    Ok(_) => {},
                    Err(error) => error!("Couldn’t send typing notification: {error}"),
                };
            })
        );
    }

    pub async fn accept_invite(&self) -> MatrixResult<()> {
        let matrix_room = self.matrix_room();

        if matrix_room.state() != RoomState::Invited {
            error!("Can’t accept invite, because this room isn’t an invited room");
            return Ok(());
        }

        let matrix_room = matrix_room.clone();
        let handle = spawn_tokio!(async move { matrix_room.join().await });
        match handle.await.unwrap() {
            Ok(_) => Ok(()),
            Err(error) => {
                error!("Accepting invitation failed: {error}");
                Err(error)
            }
        }
    }

    pub async fn decline_invite(&self) -> MatrixResult<()> {
        let matrix_room = self.matrix_room();

        if matrix_room.state() != RoomState::Invited {
            error!("Cannot decline invite, because this room is not an invited room");
            return Ok(());
        }

        let matrix_room = matrix_room.clone();
        let handle = spawn_tokio!(async move { matrix_room.leave().await });
        match handle.await.unwrap() {
            Ok(_) => Ok(()),
            Err(error) => {
                error!("Declining invitation failed: {error}");

                Err(error)
            }
        }
    }

    /// Reload the room from the SDK when its state might have changed.
    pub fn update_room(&self) {
        let state = self.matrix_room().state();
        let category = self.category();

        // Check if the previous state was different.
        if category.is_state(state) {
            // Nothing needs to be reloaded.
            return;
        }

        debug!(room_id = %self.room_id(), ?state, "The state of `Room` changed");

        if state == RoomState::Joined {
            if let Some(members) = self.members() {
                // If we where invited or left before, the list was likely not completed or
                // might have changed.
                members.reload();
            }
        }

        self.load_category();
        spawn!(clone!(@weak self as obj => async move {
            obj.load_inviter().await;
        }));
    }

    pub fn handle_left_response(&self, response_room: LeftRoom) {
        self.update_for_events(response_room.timeline.events);
    }

    pub fn handle_joined_response(&self, response_room: JoinedRoom) {
        if response_room
            .account_data
            .iter()
            .any(|e| matches!(e.deserialize(), Ok(AnyRoomAccountDataEvent::Tag(_))))
        {
            self.load_category();
        }

        self.update_for_events(response_room.timeline.events);
    }

    /// Connect to the signal emitted when the room was forgotten.
    pub fn connect_room_forgotten<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "room-forgotten",
            true,
            closure_local!(move |obj: Self| {
                f(&obj);
            }),
        )
    }

    /// Connect to the signal emitted when the join rule of the room changed.
    pub fn connect_join_rule_changed<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "join-rule-changed",
            true,
            closure_local!(move |obj: Self| {
                f(&obj);
            }),
        )
    }

    /// The ID of the predecessor of this room, if this room is an upgrade to a
    /// previous room.
    pub fn predecessor_id(&self) -> Option<&OwnedRoomId> {
        self.imp().predecessor_id.get()
    }

    /// Load the predecessor of this room.
    fn load_predecessor(&self) {
        if self.predecessor_id().is_some() {
            return;
        }

        let Some(event) = self.matrix_room().create_content() else {
            return;
        };
        let Some(predecessor) = event.predecessor else {
            return;
        };

        self.imp().predecessor_id.set(predecessor.room_id).unwrap();
        self.notify_predecessor_id_string();
    }

    /// The ID of the successor of this Room, if this room was upgraded.
    pub fn successor_id(&self) -> Option<&RoomId> {
        self.imp().successor_id.get().map(std::ops::Deref::deref)
    }

    /// Set the successor of this Room.
    fn set_successor(&self, successor: &Room) {
        self.imp().successor.set(Some(successor));
        self.notify_successor();
    }

    /// Load the tombstone for this room.
    pub fn load_tombstone(&self) {
        let imp = self.imp();

        if !self.is_tombstoned() || self.successor_id().is_some() {
            return;
        }

        if let Some(room_tombstone) = self.matrix_room().tombstone() {
            imp.successor_id
                .set(room_tombstone.replacement_room)
                .unwrap();
            self.notify_successor_id_string();
        };

        if !self.update_outdated() {
            if let Some(session) = self.session() {
                session
                    .room_list()
                    .add_tombstoned_room(self.room_id().to_owned());
            }
        }

        self.notify_is_tombstoned();
    }

    /// Update whether this `Room` is outdated.
    ///
    /// A room is outdated when it was tombstoned and we joined its successor.
    ///
    /// Returns `true` if the `Room` was set as outdated, `false` otherwise.
    pub fn update_outdated(&self) -> bool {
        if self.category() == RoomType::Outdated {
            return true;
        }

        let Some(session) = self.session() else {
            return false;
        };
        let room_list = session.room_list();

        if let Some(successor_id) = self.successor_id() {
            if let Some(successor) = room_list.get(successor_id) {
                // The Matrix spec says that we should use the "predecessor" field of the
                // m.room.create event of the successor, not the "successor" field of the
                // m.room.tombstone event, so check it just to be sure.
                if let Some(predecessor_id) = successor.predecessor_id() {
                    if predecessor_id == self.room_id() {
                        self.set_successor(&successor);
                        self.set_category_internal(RoomType::Outdated);
                        return true;
                    }
                }
            }
        }

        // The tombstone event can be redacted and we lose the successor, so search in
        // the room predecessors of other rooms.
        for room in room_list.iter::<Room>() {
            let Ok(room) = room else {
                break;
            };

            if let Some(predecessor_id) = room.predecessor_id() {
                if predecessor_id == self.room_id() {
                    self.set_successor(&room);
                    self.set_category_internal(RoomType::Outdated);
                    return true;
                }
            }
        }

        false
    }

    pub fn send_attachment(
        &self,
        bytes: Vec<u8>,
        mime: mime::Mime,
        body: &str,
        info: AttachmentInfo,
    ) {
        let matrix_room = self.matrix_room();
        if matrix_room.state() != RoomState::Joined {
            return;
        };

        let matrix_room = matrix_room.clone();
        let body = body.to_string();
        spawn_tokio!(async move {
            // Needed to hold the thumbnail data until it is sent.
            let data_slot;

            // The method will filter compatible mime types so we don't need to
            // since we ignore errors.
            let thumbnail = match generate_image_thumbnail(&mime, Cursor::new(&bytes), None) {
                Ok((data, info)) => {
                    data_slot = data;
                    Some(Thumbnail {
                        data: data_slot,
                        content_type: mime::IMAGE_JPEG,
                        info: Some(info),
                    })
                }
                _ => None,
            };

            let config = if let Some(thumbnail) = thumbnail {
                AttachmentConfig::with_thumbnail(thumbnail)
            } else {
                AttachmentConfig::new()
            }
            .info(info);

            matrix_room
                // TODO This should be added to pending messages instead of
                // sending it directly.
                .send_attachment(&body, &mime, bytes, config)
                .await
                .unwrap();
        });
    }

    /// Invite the given users to this room.
    ///
    /// Returns `Ok(())` if all the invites are sent successfully, otherwise
    /// returns the list of users who could not be invited.
    pub async fn invite<'a>(&self, user_ids: &'a [OwnedUserId]) -> Result<(), Vec<&'a UserId>> {
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
        for (index, result) in handle.await.unwrap().iter().enumerate() {
            match result {
                Ok(_) => {}
                Err(error) => {
                    error!("Failed to invite user with ID {}: {error}", user_ids[index],);
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
    pub async fn kick<'a>(
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
        for (index, result) in handle.await.unwrap().iter().enumerate() {
            match result {
                Ok(_) => {}
                Err(error) => {
                    error!("Failed to kick user with ID {}: {error}", users[index].0);
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
    pub async fn ban<'a>(
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
        for (index, result) in handle.await.unwrap().iter().enumerate() {
            match result {
                Ok(_) => {}
                Err(error) => {
                    error!("Failed to ban user with ID {}: {error}", users[index].0);
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
    pub async fn unban<'a>(
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
        for (index, result) in handle.await.unwrap().iter().enumerate() {
            match result {
                Ok(_) => {}
                Err(error) => {
                    error!("Failed to unban user with ID {}: {error}", users[index].0);
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

    /// Update the latest activity of the room with the given events.
    ///
    /// The events must be in reverse chronological order.
    pub fn update_latest_activity<'a>(&self, events: impl IntoIterator<Item = &'a Event>) {
        let mut latest_activity = self.latest_activity();

        for event in events {
            if event.counts_as_unread() {
                latest_activity = latest_activity.max(event.origin_server_ts_u64());
                break;
            }
        }

        self.set_latest_activity(latest_activity);
    }

    /// Set whether this room is encrypted.
    fn set_is_encrypted(&self, is_encrypted: bool) {
        let was_encrypted = self.encrypted();
        if was_encrypted == is_encrypted {
            return;
        }

        if was_encrypted && !is_encrypted {
            error!("Encryption for a room can't be disabled");
            return;
        }

        // if self.matrix_room().is_encrypted() != is_encrypted {
        // TODO: enable encryption if it isn't enabled yet
        // }

        spawn!(clone!(@strong self as obj => async move {
            obj.load_is_encrypted().await;
        }));
    }

    /// Listen to changes in room encryption.
    fn set_up_is_encrypted(&self) {
        let matrix_room = self.matrix_room();

        let obj_weak = glib::SendWeakRef::from(self.downgrade());
        matrix_room.add_event_handler(move |_: SyncRoomEncryptionEvent| {
            let obj_weak = obj_weak.clone();
            async move {
                let ctx = glib::MainContext::default();
                ctx.spawn(async move {
                    if let Some(obj) = obj_weak.upgrade() {
                        obj.set_is_encrypted(true);
                    }
                });
            }
        });

        spawn!(
            glib::Priority::DEFAULT_IDLE,
            clone!(@weak self as obj => async move {
                obj.load_is_encrypted().await;
            })
        );
    }

    /// Load whether the room is encrypted from the SDK.
    async fn load_is_encrypted(&self) {
        let matrix_room = self.matrix_room().clone();
        let handle = spawn_tokio!(async move { matrix_room.is_encrypted().await });

        if handle
            .await
            .unwrap()
            .ok()
            .filter(|encrypted| *encrypted)
            .is_none()
        {
            return;
        }

        self.imp().encrypted.set(true);
        self.notify_encrypted();
    }

    /// Get a `Pill` representing this `Room`.
    pub fn to_pill(&self) -> Pill {
        Pill::for_room(self)
    }

    /// Get a human-readable ID for this `Room`.
    ///
    /// This is to identify the room easily in logs.
    pub fn human_readable_id(&self) -> String {
        format!("{} ({})", self.display_name(), self.room_id())
    }

    /// Update the avatar for the room.
    fn update_avatar(&self) {
        let Some(session) = self.session() else {
            return;
        };
        let imp = self.imp();

        if let Some(avatar_url) = self.matrix_room().avatar_url() {
            imp.set_has_avatar(true);

            let avatar_image = if let Some(avatar_image) = imp
                .avatar_data
                .image()
                .filter(|i| i.uri_source() == AvatarUriSource::Room)
            {
                avatar_image
            } else {
                let avatar_image =
                    AvatarImage::new(&session, Some(&avatar_url), AvatarUriSource::Room);
                imp.avatar_data.set_image(Some(avatar_image.clone()));
                avatar_image
            };
            avatar_image.set_uri(Some(avatar_url.to_string()));

            return;
        }

        imp.set_has_avatar(false);

        if let Some(direct_member) = self.direct_member() {
            imp.avatar_data
                .set_image(direct_member.avatar_data().image());
        }

        if imp.avatar_data.image().is_none() {
            imp.avatar_data.set_image(Some(AvatarImage::new(
                &session,
                None,
                AvatarUriSource::Room,
            )))
        }
    }

    /// Whether our own user can join this room on their own.
    pub fn can_join(&self) -> bool {
        if self.own_member().membership() == Membership::Ban {
            return false;
        }

        let join_rule = self.matrix_room().join_rule();

        match join_rule {
            JoinRule::Public => true,
            JoinRule::Restricted(rules) => rules
                .allow
                .into_iter()
                .all(|rule| self.passes_restricted_allow_rule(rule)),
            _ => false,
        }
    }

    /// Whether our account passes the given restricted allow rule.
    fn passes_restricted_allow_rule(&self, rule: AllowRule) -> bool {
        match rule {
            AllowRule::RoomMembership(room_membership) => self.session().is_some_and(|s| {
                s.room_list()
                    .joined_room(&room_membership.room_id.into())
                    .is_some()
            }),
            _ => false,
        }
    }

    /// The `matrix.to` URI representation for this room.
    pub async fn matrix_to_uri(&self) -> MatrixToUri {
        let matrix_room = self.matrix_room().clone();

        let handle = spawn_tokio!(async move { matrix_room.matrix_to_permalink().await });
        match handle.await.unwrap() {
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

    /// The `matrix:` URI representation for this room.
    pub async fn matrix_uri(&self) -> MatrixUri {
        let matrix_room = self.matrix_room().clone();

        let handle = spawn_tokio!(async move { matrix_room.matrix_permalink(false).await });
        match handle.await.unwrap() {
            Ok(permalink) => {
                return permalink;
            }
            Err(error) => {
                error!("Could not get room event permalink: {error}");
            }
        }

        // Fallback to using just the room ID, without routing.
        self.room_id().matrix_uri(false)
    }

    /// The `matrix.to` URI representation for the given event in this room.
    pub async fn matrix_to_event_uri(&self, event_id: OwnedEventId) -> MatrixToUri {
        let matrix_room = self.matrix_room().clone();

        let event_id_clone = event_id.clone();
        let handle =
            spawn_tokio!(
                async move { matrix_room.matrix_to_event_permalink(event_id_clone).await }
            );
        match handle.await.unwrap() {
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

    /// The `matrix:` URI representation for the given event in this room.
    pub async fn matrix_event_uri(&self, event_id: OwnedEventId) -> MatrixUri {
        let matrix_room = self.matrix_room().clone();

        let event_id_clone = event_id.clone();
        let handle =
            spawn_tokio!(async move { matrix_room.matrix_event_permalink(event_id_clone).await });
        match handle.await.unwrap() {
            Ok(permalink) => {
                return permalink;
            }
            Err(error) => {
                error!("Could not get room event permalink: {error}");
            }
        }

        // Fallback to using just the room ID, without routing.
        self.room_id().matrix_event_uri(event_id)
    }
}
