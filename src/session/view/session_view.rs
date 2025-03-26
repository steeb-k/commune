use adw::{prelude::*, subclass::prelude::*};
use gtk::{
    self, gdk,
    glib::{self, clone, signal::SignalHandlerId},
    CompositeTemplate,
};
use ruma::{OwnedUserId, RoomId, RoomOrAliasId};
use tracing::{error, warn};

use super::{Content, CreateDmDialog, MediaViewer, RoomCreation, Sidebar};
use crate::{
    components::{JoinRoomDialog, UserProfileDialog},
    prelude::*,
    session::model::{
        Event, IdentityVerification, Room, RoomCategory, Selection, Session, SidebarListModel,
        VerificationKey,
    },
    toast,
    utils::matrix::{MatrixEventIdUri, MatrixIdUri, MatrixRoomIdUri},
    Window,
};

/// A predicate to filter rooms depending on whether they have unread messages
#[derive(Eq, PartialEq, Copy, Clone)]
pub enum ReadState {
    /// Any room can be selected
    Any,
    /// Only unread rooms can be selected
    Unread,
}

/// A direction in the room list
#[derive(Eq, PartialEq, Copy, Clone)]
pub enum Direction {
    Up,
    Down,
}

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session/view/session_view.ui")]
    #[properties(wrapper_type = super::SessionView)]
    pub struct SessionView {
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub overlay: TemplateChild<gtk::Overlay>,
        #[template_child]
        pub split_view: TemplateChild<adw::NavigationSplitView>,
        #[template_child]
        pub sidebar: TemplateChild<Sidebar>,
        #[template_child]
        pub content: TemplateChild<Content>,
        #[template_child]
        pub media_viewer: TemplateChild<MediaViewer>,
        /// The current session.
        #[property(get, set = Self::set_session, explicit_notify, nullable)]
        pub session: glib::WeakRef<Session>,
        pub window_active_handler_id: RefCell<Option<SignalHandlerId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SessionView {
        const NAME: &'static str = "SessionView";
        type Type = super::SessionView;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);

            klass.install_action("session.close-room", None, |obj, _, _| {
                obj.select_room(None);
            });

            klass.install_action(
                "session.show-room",
                Some(&String::static_variant_type()),
                |obj, _, parameter| {
                    if let Ok(room_id) =
                        <&RoomId>::try_from(&*parameter.unwrap().get::<String>().unwrap())
                    {
                        obj.select_room_by_id(room_id);
                    } else {
                        error!("Cannot show room with invalid ID");
                    }
                },
            );

            klass.install_action_async("session.logout", None, |obj, _, _| async move {
                if let Some(session) = obj.session() {
                    if let Err(error) = session.log_out().await {
                        toast!(obj, error);
                    }
                }
            });

            klass.install_action("session.show-content", None, |obj, _, _| {
                obj.show_content();
            });

            klass.install_action("session.room-creation", None, |obj, _, _| {
                obj.show_room_creation_dialog();
            });

            klass.install_action("session.show-join-room", None, |obj, _, _| {
                obj.show_join_room_dialog(None);
            });

            klass.install_action_async("session.create-dm", None, |obj, _, _| async move {
                obj.show_create_dm_dialog().await;
            });

            klass.add_binding_action(
                gdk::Key::Escape,
                gdk::ModifierType::empty(),
                "session.close-room",
            );

            klass.install_action("session.toggle-room-search", None, |obj, _, _| {
                obj.toggle_room_search();
            });

            klass.add_binding_action(
                gdk::Key::k,
                gdk::ModifierType::CONTROL_MASK,
                "session.toggle-room-search",
            );

            klass.install_action("session.select-unread-room", None, |obj, _, _| {
                obj.select_unread_room();
            });

            klass.add_binding_action(
                gdk::Key::asterisk,
                gdk::ModifierType::CONTROL_MASK,
                "session.select-unread-room",
            );

            klass.install_action("session.select-prev-room", None, |obj, _, _| {
                obj.select_next_room(ReadState::Any, Direction::Up);
            });

            klass.install_action("session.select-prev-unread-room", None, |obj, _, _| {
                obj.select_next_room(ReadState::Unread, Direction::Up);
            });

            klass.install_action("session.select-next-room", None, |obj, _, _| {
                obj.select_next_room(ReadState::Any, Direction::Down);
            });

            klass.install_action("session.select-next-unread-room", None, |obj, _, _| {
                obj.select_next_room(ReadState::Unread, Direction::Down);
            });

            klass.install_action(
                "session.show-matrix-uri",
                Some(&MatrixIdUri::static_variant_type()),
                |obj, _, parameter| {
                    if let Some(uri) = parameter.unwrap().get::<MatrixIdUri>() {
                        obj.show_matrix_uri(uri);
                    } else {
                        error!("Cannot show invalid Matrix URI");
                    }
                },
            );
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for SessionView {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            self.sidebar
                .property_expression("list-model")
                .chain_property::<SidebarListModel>("selection-model")
                .chain_property::<Selection>("selected-item")
                .watch(
                    glib::Object::NONE,
                    clone!(
                        #[weak(rename_to = imp)]
                        self,
                        move || {
                            let show_content = imp
                                .sidebar
                                .list_model()
                                .is_some_and(|m| m.selection_model().selected_item().is_some());
                            imp.split_view.set_show_content(show_content);

                            // Only grab focus for the sidebar here. We handle the other case in
                            // `Content::set_item()` directly, because we need to grab focus only
                            // after the visible content changed.
                            if !show_content {
                                imp.sidebar.grab_focus();
                            }
                        }
                    ),
                );

            self.content.connect_item_notify(clone!(
                #[weak]
                obj,
                move |_| {
                    // Withdraw the notifications of the newly selected item.
                    obj.withdraw_selected_item_notifications();
                }
            ));

            obj.connect_root_notify(|obj| {
                let Some(window) = obj.parent_window() else {
                    return;
                };

                let handler_id = window.connect_is_active_notify(clone!(
                    #[weak]
                    obj,
                    move |window| {
                        if !window.is_active() {
                            return;
                        }

                        // When the window becomes active, withdraw the notifications
                        // of the selected item.
                        obj.withdraw_selected_item_notifications();
                    }
                ));
                obj.imp().window_active_handler_id.replace(Some(handler_id));
            });

            // Make sure all header bars on the same screen have the same height.
            // Necessary when the text scaling changes.
            let size_group = gtk::SizeGroup::new(gtk::SizeGroupMode::Vertical);
            size_group.add_widget(self.sidebar.header_bar());

            for header_bar in self.content.header_bars() {
                size_group.add_widget(header_bar);
            }
        }

        fn dispose(&self) {
            if let Some(handler_id) = self.window_active_handler_id.take() {
                if let Some(window) = self.obj().parent_window() {
                    window.disconnect(handler_id);
                }
            }
        }
    }

    impl WidgetImpl for SessionView {}
    impl BinImpl for SessionView {}

    impl SessionView {
        /// Set the current session.
        fn set_session(&self, session: Option<&Session>) {
            if self.session.upgrade().as_ref() == session {
                return;
            }

            self.session.set(session);
            self.obj().notify_session();
        }
    }
}

glib::wrapper! {
    /// A view for a Matrix user session.
    pub struct SessionView(ObjectSubclass<imp::SessionView>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl SessionView {
    /// Create a new session view.
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// The currently selected item, if any.
    pub fn selected_item(&self) -> Option<glib::Object> {
        self.imp().content.item()
    }

    /// Select the given item.
    pub fn select_item(&self, item: Option<impl IsA<glib::Object>>) {
        let Some(session) = self.session() else {
            return;
        };

        session
            .sidebar_list_model()
            .selection_model()
            .set_selected_item(item);
    }

    /// The currently selected room, if any.
    pub fn selected_room(&self) -> Option<Room> {
        self.selected_item().and_downcast()
    }

    /// Select the given room.
    pub fn select_room(&self, room: Option<Room>) {
        let imp = self.imp();

        self.select_item(room.clone());

        // If we selected a room, make sure it is visible in the sidebar.
        let Some(room) = room else {
            return;
        };

        // First, ensure that the section containing the room is expanded.
        if let Some(section) = imp.sidebar.list_model().and_then(|list_model| {
            list_model
                .item_list()
                .section_from_room_category(room.category())
        }) {
            section.set_is_expanded(true);
        }

        // Now scroll to the room to make sure that it is in the viewport, and that it
        // is focused in the list for users using keyboard navigation.
        imp.sidebar.scroll_to_selection();
    }

    /// Select the room with the given ID in this view.
    pub fn select_room_by_id(&self, room_id: &RoomId) {
        if let Some(room) = self.session().and_then(|s| s.room_list().get(room_id)) {
            self.select_room(Some(room));
        } else {
            warn!("A room with ID {room_id} could not be found");
        }
    }

    /// Select the room with the given identifier in this view, if it exists.
    ///
    /// Returns `true` if the room was found.
    pub fn select_room_if_exists(&self, identifier: &RoomOrAliasId) -> bool {
        if let Some(room) = self
            .session()
            .and_then(|s| s.room_list().joined_room(identifier))
        {
            self.select_room(Some(room));
            true
        } else {
            false
        }
    }

    /// Select the next room with the given read state in the given direction.
    ///
    /// The search wraps: if no room matches below (for `direction == Down`)
    /// then search continues in the down direction from the first room.
    pub fn select_next_room(&self, read_state: ReadState, direction: Direction) {
        let Some(session) = self.session() else {
            return;
        };

        let item_list = session.sidebar_list_model().selection_model();
        let len = item_list.n_items();
        let current_index = item_list.selected().min(len);

        let search_order: Box<dyn Iterator<Item = u32>> = {
            // Iterate over every item except the current one.
            let order = ((current_index + 1)..len).chain(0..current_index);
            match direction {
                Direction::Up => Box::new(order.rev()),
                Direction::Down => Box::new(order),
            }
        };

        for index in search_order {
            let Some(item) = item_list.item(index) else {
                // The list of rooms was mutated: let's give up responding to the key binding.
                return;
            };

            if let Some(room) = item.downcast_ref::<Room>() {
                if read_state == ReadState::Any || !room.is_read() {
                    self.select_room(Some(room.clone()));
                    return;
                }
            }
        }
    }

    /// Select a room which should be read.
    pub fn select_unread_room(&self) {
        let Some(session) = self.session() else {
            return;
        };
        let room_list = session.room_list().snapshot();
        let current_room = self.selected_room();

        if let Some((unread_room, _score)) = room_list
            .into_iter()
            .filter(|room| Some(room) != current_room.as_ref())
            .filter_map(|room| Self::score_for_unread_room(&room).map(|score| (room, score)))
            .max_by_key(|(_room, score)| *score)
        {
            self.select_room(Some(unread_room));
        }
    }

    /// The score to determine the order in which unread rooms are selected.
    ///
    /// First by category, then by notification count so DMs are selected
    /// before group chats, and finally by recency.
    ///
    /// Returns `None` if the room should never be selected.
    fn score_for_unread_room(room: &Room) -> Option<(u8, u64, u64)> {
        if room.is_read() {
            return None;
        }
        let category = match room.category() {
            RoomCategory::Invited => 5,
            RoomCategory::Favorite => 4,
            RoomCategory::Normal => 3,
            RoomCategory::LowPriority => 2,
            RoomCategory::Left => 1,
            RoomCategory::Ignored | RoomCategory::Outdated | RoomCategory::Space => return None,
        };
        Some((category, room.notification_count(), room.latest_activity()))
    }

    /// Select the identity verification with the given key in this view.
    pub fn select_identity_verification_by_id(&self, key: &VerificationKey) {
        if let Some(verification) = self.session().and_then(|s| s.verification_list().get(key)) {
            self.select_identity_verification(verification);
        } else {
            warn!(
                "An identity verification for user {} with flow ID {} could not be found",
                key.user_id, key.flow_id
            );
        }
    }

    /// Select the given identity verification in this view.
    pub fn select_identity_verification(&self, verification: IdentityVerification) {
        self.select_item(Some(verification));
    }

    fn toggle_room_search(&self) {
        let room_search = self.imp().sidebar.room_search_bar();
        room_search.set_search_mode(!room_search.is_search_mode());
    }

    /// Returns the ancestor window containing this widget.
    fn parent_window(&self) -> Option<Window> {
        self.root().and_downcast()
    }

    fn show_room_creation_dialog(&self) {
        let Some(session) = self.session() else {
            return;
        };

        let dialog = RoomCreation::new(&session);
        dialog.present(Some(self));
    }

    async fn show_create_dm_dialog(&self) {
        let Some(session) = self.session() else {
            return;
        };

        let dialog = CreateDmDialog::new(&session);
        dialog.start_direct_chat(self).await;
    }

    /// Offer to the user to join a room.
    ///
    /// If no room URI is provided, the user will have to enter one.
    pub fn show_join_room_dialog(&self, room_uri: Option<MatrixRoomIdUri>) {
        let Some(session) = self.session() else {
            return;
        };

        if let Some(room_uri) = &room_uri {
            if self.select_room_if_exists(&room_uri.id) {
                return;
            }
        }

        let dialog = JoinRoomDialog::new(&session);

        if let Some(uri) = room_uri {
            dialog.set_uri(uri);
        }

        dialog.present(Some(self));
    }

    pub fn handle_paste_action(&self) {
        self.imp().content.handle_paste_action();
    }

    /// Show the content of the session
    pub fn show_content(&self) {
        let imp = self.imp();

        imp.stack.set_visible_child(&*imp.overlay);

        if let Some(window) = self.parent_window() {
            window.show_selected_session();
        }
    }

    /// Show a media event.
    pub fn show_media(&self, event: &Event, source_widget: &impl IsA<gtk::Widget>) {
        let Some(media_message) = event.visual_media_message() else {
            error!(
                "Trying to open the media viewer with an event that is not a visual media message"
            );
            return;
        };

        let imp = self.imp();
        imp.media_viewer
            .set_message(&event.room(), event.event_id().unwrap(), media_message);
        imp.media_viewer.reveal(source_widget);
    }

    /// Show the profile of the given user.
    pub fn show_user_profile_dialog(&self, user_id: OwnedUserId) {
        let Some(session) = self.session() else {
            return;
        };

        let dialog = UserProfileDialog::new();
        dialog.load_user(&session, user_id);
        dialog.present(Some(self));
    }

    /// Withdraw the notifications for the currently selected item.
    fn withdraw_selected_item_notifications(&self) {
        let Some(session) = self.session() else {
            return;
        };
        let Some(item) = self.selected_item() else {
            return;
        };

        let notifications = session.notifications();

        if let Some(room) = item.downcast_ref::<Room>() {
            notifications.withdraw_all_for_room(room.room_id());
        } else if let Some(verification) = item.downcast_ref::<IdentityVerification>() {
            notifications.withdraw_identity_verification(&verification.key());
        }
    }

    /// Show the given `MatrixIdUri`.
    pub fn show_matrix_uri(&self, uri: MatrixIdUri) {
        match uri {
            MatrixIdUri::Room(room_uri) | MatrixIdUri::Event(MatrixEventIdUri { room_uri, .. }) => {
                if !self.select_room_if_exists(&room_uri.id) {
                    self.show_join_room_dialog(Some(room_uri));
                }
            }
            MatrixIdUri::User(user_id) => {
                self.show_user_profile_dialog(user_id);
            }
        }
    }
}
