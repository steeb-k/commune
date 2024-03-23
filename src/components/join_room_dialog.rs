use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{glib, glib::clone, CompositeTemplate};

use super::{Avatar, Spinner, SpinnerButton, ToastableDialog};
use crate::{
    i18n::ngettext_f,
    prelude::*,
    session::model::{RemoteRoom, Session},
    toast,
    utils::{matrix::MatrixRoomIdUri, LoadingState},
    Window,
};

mod imp {
    use std::cell::{Cell, RefCell};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/join_room_dialog.ui")]
    #[properties(wrapper_type = super::JoinRoomDialog)]
    pub struct JoinRoomDialog {
        #[template_child]
        pub go_back_btn: TemplateChild<gtk::Button>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub entry_page: TemplateChild<gtk::Box>,
        #[template_child]
        pub search_entry: TemplateChild<gtk::SearchEntry>,
        #[template_child]
        pub look_up_btn: TemplateChild<SpinnerButton>,
        #[template_child]
        pub room_avatar: TemplateChild<Avatar>,
        #[template_child]
        pub room_name: TemplateChild<gtk::Label>,
        #[template_child]
        pub room_alias: TemplateChild<gtk::Label>,
        #[template_child]
        pub room_topic: TemplateChild<gtk::Label>,
        #[template_child]
        pub room_members_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub room_members_count: TemplateChild<gtk::Label>,
        #[template_child]
        pub join_btn: TemplateChild<SpinnerButton>,
        /// The current session.
        #[property(get, set = Self::set_session, construct_only)]
        pub session: glib::WeakRef<Session>,
        /// The URI to preview.
        pub uri: RefCell<Option<MatrixRoomIdUri>>,
        /// The room that is previewed.
        #[property(get)]
        pub room: RefCell<Option<RemoteRoom>>,
        pub disable_go_back: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for JoinRoomDialog {
        const NAME: &'static str = "JoinRoomDialog";
        type Type = super::JoinRoomDialog;
        type ParentType = ToastableDialog;

        fn class_init(klass: &mut Self::Class) {
            Spinner::ensure_type();

            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for JoinRoomDialog {}

    impl WidgetImpl for JoinRoomDialog {}
    impl AdwDialogImpl for JoinRoomDialog {}
    impl ToastableDialogImpl for JoinRoomDialog {}

    impl JoinRoomDialog {
        /// Set the current session.
        fn set_session(&self, session: Option<Session>) {
            self.session.set(session.as_ref());

            let obj = self.obj();
            obj.notify_session();
            obj.update_entry_page();
        }

        /// Set the room that is previewed.
        pub(super) fn set_room(&self, room: Option<RemoteRoom>) {
            if *self.room.borrow() == room {
                return;
            }
            let obj = self.obj();

            self.room.replace(room.clone());

            if let Some(room) = room {
                if matches!(
                    room.loading_state(),
                    LoadingState::Ready | LoadingState::Error
                ) {
                    obj.fill_details();
                } else {
                    room.connect_loading_state_notify(clone!(@weak obj => move |room| {
                    if matches!(room.loading_state(), LoadingState::Ready | LoadingState::Error) {
                        obj.fill_details();
                    }
                }));
                }
            }

            obj.notify_room();
        }

        /// Whether we can go back to the previous screen.
        pub fn can_go_back(&self) -> bool {
            !self.disable_go_back.get()
                && self.stack.visible_child_name().as_deref() == Some("details")
        }

        /// Set the currently visible page.
        pub fn set_visible_page(&self, page_name: &str) {
            self.stack.set_visible_child_name(page_name);
            self.go_back_btn.set_visible(self.can_go_back());
        }
    }
}

glib::wrapper! {
    /// Dialog to join a room.
    pub struct JoinRoomDialog(ObjectSubclass<imp::JoinRoomDialog>)
        @extends gtk::Widget, adw::Dialog, ToastableDialog, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl JoinRoomDialog {
    pub fn new(session: &Session) -> Self {
        glib::Object::builder().property("session", session).build()
    }

    /// Set the room URI to look up.
    pub fn set_uri(&self, uri: MatrixRoomIdUri) {
        let imp = self.imp();

        imp.uri.replace(Some(uri.clone()));
        imp.disable_go_back.set(true);
        imp.set_visible_page("loading");

        self.look_up_room_inner(uri);
    }

    /// Set the remote room.
    pub fn set_room(&self, room: RemoteRoom) {
        let imp = self.imp();

        imp.disable_go_back.set(true);
        imp.set_room(Some(room));
    }

    /// Update the state of the entry page.
    #[template_callback]
    fn update_entry_page(&self) {
        let imp = self.imp();

        let Some(session) = self.session() else {
            imp.entry_page.set_sensitive(false);
            return;
        };
        imp.entry_page.set_sensitive(true);

        let Some(uri) = MatrixRoomIdUri::parse(&imp.search_entry.text()) else {
            imp.look_up_btn.set_sensitive(false);
            imp.uri.take();
            return;
        };
        imp.look_up_btn.set_sensitive(true);

        let id = uri.id.clone();
        imp.uri.replace(Some(uri));

        if session.room_list().joined_room(&id).is_some() {
            // Translators: This is a verb, as in 'View Room'.
            imp.look_up_btn.set_content_label(gettext("View"));
        } else {
            // Translators: This is a verb, as in 'Look up Room'.
            imp.look_up_btn.set_content_label(gettext("Look Up"));
        }
    }

    /// Look up the room that was entered, if it is valid.
    ///
    /// If the room is not, this will open it instead.
    #[template_callback]
    fn look_up_room(&self) {
        let imp = self.imp();

        let Some(uri) = imp.uri.borrow().clone() else {
            return;
        };
        let Some(window) = self.root().and_downcast::<Window>() else {
            return;
        };

        imp.look_up_btn.set_loading(true);
        imp.entry_page.set_sensitive(false);

        // Join or view the room with the given identifier.
        if window.session_view().select_room_if_exists(&uri.id) {
            self.close();
        } else {
            self.look_up_room_inner(uri);
        }
    }

    fn look_up_room_inner(&self, uri: MatrixRoomIdUri) {
        let Some(session) = self.session() else {
            return;
        };
        let imp = self.imp();

        // Reset state before switching to possible pages.
        imp.go_back_btn.set_sensitive(true);
        imp.join_btn.set_loading(false);

        let room = RemoteRoom::new(&session, uri);
        imp.set_room(Some(room));
    }

    /// Fill the details with the given result.
    fn fill_details(&self) {
        let imp = self.imp();
        let Some(room) = imp.room.borrow().clone() else {
            return;
        };

        imp.room_name.set_label(&room.display_name());

        let alias = room.alias();
        if let Some(alias) = &alias {
            imp.room_alias.set_label(alias.as_str());
        }
        imp.room_alias
            .set_visible(room.name().is_some() && alias.is_some());

        imp.room_avatar.set_data(Some(room.avatar_data()));

        if room.loading_state() == LoadingState::Error {
            imp.room_topic.set_label(&gettext(
                "The room details cannot be previewed. It can be because the room is not known by the homeserver or because its details are private. You can still try to join it."
            ));
            imp.room_topic.set_visible(true);
            imp.room_members_box.set_visible(false);

            imp.set_visible_page("details");
            return;
        }

        if let Some(topic) = room.topic() {
            imp.room_topic.set_label(&topic);
            imp.room_topic.set_visible(true);
        } else {
            imp.room_topic.set_visible(false);
        }

        let members_count = room.joined_members_count();
        imp.room_members_count.set_label(&members_count.to_string());

        let members_tooltip = ngettext_f(
            // Translators: Do NOT translate the content between '{' and '}',
            // this is a variable name.
            "1 member",
            "{n} members",
            members_count,
            &[("n", &members_count.to_string())],
        );
        imp.room_members_box
            .set_tooltip_text(Some(&members_tooltip));
        imp.room_members_box.set_visible(true);

        imp.set_visible_page("details");
    }

    /// Join the room that was entered, if it is valid.
    #[template_callback]
    async fn join_room(&self) {
        let Some(session) = self.session() else {
            return;
        };

        let Some(room) = self.room() else {
            return;
        };

        let imp = self.imp();
        imp.go_back_btn.set_sensitive(false);
        imp.join_btn.set_loading(true);

        // Join the room with the given identifier.
        let room_list = session.room_list();
        let uri = room.uri().clone();

        match room_list.join_by_id_or_alias(uri.id.into(), uri.via).await {
            Ok(room_id) => {
                if let Some(room) = room_list.get_wait(&room_id).await {
                    if let Some(window) = self.root().and_downcast_ref::<Window>() {
                        window.session_view().select_room(Some(room));
                    }
                }

                self.close();
            }
            Err(error) => {
                toast!(self, error);

                imp.join_btn.set_loading(false);
                imp.go_back_btn.set_sensitive(true);
            }
        }
    }

    /// Go back to the previous screen.
    ///
    /// If we can't go back, closes the window.
    #[template_callback]
    fn go_back(&self) {
        let imp = self.imp();

        if imp.can_go_back() {
            // There is only one screen to go back to.
            imp.look_up_btn.set_loading(false);
            imp.entry_page.set_sensitive(true);
            imp.set_visible_page("entry");
        } else {
            self.close();
        }
    }
}
