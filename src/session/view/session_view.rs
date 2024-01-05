use adw::{prelude::*, subclass::prelude::*};
use gtk::{
    self, gdk, glib,
    glib::{clone, signal::SignalHandlerId},
    CompositeTemplate,
};
use ruma::{OwnedUserId, RoomId};
use tracing::{error, warn};

use super::{
    Content, CreateDmDialog, JoinRoomDialog, MediaViewer, RoomCreation, Sidebar, UserProfileDialog,
};
use crate::{
    session::model::{Event, IdentityVerification, Room, Selection, Session, SidebarListModel},
    spawn, toast,
    utils::matrix::{MatrixRoomId, MatrixRoomIdUri},
    Window,
};

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

            klass.install_action("session.close-room", None, move |obj, _, _| {
                obj.select_room(None);
            });

            klass.install_action("session.show-room", Some("s"), move |obj, _, parameter| {
                if let Ok(room_id) =
                    <&RoomId>::try_from(&*parameter.unwrap().get::<String>().unwrap())
                {
                    obj.select_room_by_id(room_id);
                } else {
                    error!("Cannot show room with invalid ID");
                }
            });

            klass.install_action("session.logout", None, move |obj, _, _| {
                if let Some(session) = obj.session() {
                    spawn!(clone!(@weak obj, @weak session => async move {
                        if let Err(error) = session.logout().await {
                            toast!(obj, error);
                        }
                    }));
                }
            });

            klass.install_action("session.show-content", None, move |obj, _, _| {
                obj.show_content();
            });

            klass.install_action("session.room-creation", None, move |obj, _, _| {
                obj.show_room_creation_dialog();
            });

            klass.install_action("session.show-join-room", None, move |obj, _, _| {
                obj.show_join_room_dialog(None);
            });

            klass.install_action("session.create-dm", None, move |obj, _, _| {
                obj.show_create_dm_dialog();
            });

            klass.add_binding_action(
                gdk::Key::Escape,
                gdk::ModifierType::empty(),
                "session.close-room",
                None,
            );

            klass.install_action("session.toggle-room-search", None, move |obj, _, _| {
                obj.toggle_room_search();
            });

            klass.add_binding_action(
                gdk::Key::k,
                gdk::ModifierType::CONTROL_MASK,
                "session.toggle-room-search",
                None,
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

            self.sidebar.property_expression("list-model").chain_property::<SidebarListModel>("selection-model").chain_property::<Selection>("selected-item").watch(glib::Object::NONE,
                clone!(@weak self as imp => move || {
                    let show_content = imp.sidebar.list_model().is_some_and(|m| m.selection_model().selected_item().is_some());
                    imp.split_view.set_show_content(show_content);
                }),
            );

            self.content
                .connect_item_notify(clone!(@weak obj => move |_| {
                    let Some(session) = obj.session() else {
                        return;
                    };
                    let Some(room) = obj.selected_room() else {
                        return;
                    };

                    // When switching to a room, withdraw its notifications.
                    session.notifications().withdraw_all_for_room(&room);
                }));

            obj.connect_root_notify(|obj| {
                let Some(window) = obj.parent_window() else {
                    return;
                };

                let handler_id =
                    window.connect_is_active_notify(clone!(@weak obj => move |window| {
                        if !window.is_active() {
                            return;
                        }
                        let Some(session) = obj.session() else {
                            return;
                        };
                        let Some(room) = obj.selected_room() else {
                            return;
                        };

                        // When the window becomes active, withdraw the notifications
                        // of the room that is displayed.
                        session.notifications().withdraw_all_for_room(&room);
                    }));
                obj.imp().window_active_handler_id.replace(Some(handler_id));
            });
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
        fn set_session(&self, session: Option<Session>) {
            if self.session.upgrade() == session {
                return;
            }

            self.session.set(session.as_ref());
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
    pub async fn new() -> Self {
        glib::Object::new()
    }

    /// The currently selected room, if any.
    pub fn selected_room(&self) -> Option<Room> {
        self.imp().content.item().and_downcast()
    }

    pub fn select_room(&self, room: Option<Room>) {
        self.select_item(room);
    }

    pub fn select_item(&self, item: Option<impl IsA<glib::Object>>) {
        let Some(session) = self.session() else {
            return;
        };

        session
            .sidebar_list_model()
            .selection_model()
            .set_selected_item(item);
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
    pub fn select_room_if_exists(&self, room_id: &MatrixRoomId) -> bool {
        if let Some(room) = self
            .session()
            .and_then(|s| s.room_list().joined_room(room_id))
        {
            self.select_room(Some(room));
            true
        } else {
            false
        }
    }

    /// Select the given verification in this view.
    pub fn select_verification(&self, verification: IdentityVerification) {
        self.select_item(Some(verification));
    }

    fn toggle_room_search(&self) {
        let room_search = self.imp().sidebar.room_search_bar();
        room_search.set_search_mode(!room_search.is_search_mode());
    }

    /// Returns the parent GtkWindow containing this widget.
    fn parent_window(&self) -> Option<Window> {
        self.root().and_downcast()
    }

    fn show_room_creation_dialog(&self) {
        let Some(session) = self.session() else {
            return;
        };

        let window = RoomCreation::new(self.parent_window().as_ref(), &session);
        window.present();
    }

    fn show_create_dm_dialog(&self) {
        let Some(session) = self.session() else {
            return;
        };

        let window = CreateDmDialog::new(self.parent_window().as_ref(), &session);
        window.present();
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

        let dialog = JoinRoomDialog::new(self.parent_window().as_ref(), &session);

        if let Some(uri) = room_uri {
            dialog.set_uri(uri);
        }

        dialog.present();
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
        let Some(message) = event.message() else {
            error!("Trying to open the media viewer with an event that is not a message");
            return;
        };

        let imp = self.imp();
        imp.media_viewer
            .set_message(&event.room(), event.event_id().unwrap(), message);
        imp.media_viewer.reveal(source_widget);
    }

    /// Show the profile of the given user.
    pub fn show_user_profile_dialog(&self, user_id: OwnedUserId) {
        let Some(session) = self.session() else {
            return;
        };

        let dialog = UserProfileDialog::new(self.parent_window().as_ref(), &session, user_id);
        dialog.present();
    }
}
