use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{glib, glib::clone, CompositeTemplate};

use super::ToastableDialog;
use crate::{
    components::{Avatar, LoadingButton},
    i18n::ngettext_f,
    prelude::*,
    session::model::{RemoteRoom, Session},
    toast,
    utils::{
        matrix::{MatrixIdUri, MatrixRoomIdUri},
        LoadingState,
    },
    Window,
};

mod imp {
    use std::cell::{Cell, RefCell};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/dialogs/join_room.ui")]
    #[properties(wrapper_type = super::JoinRoomDialog)]
    pub struct JoinRoomDialog {
        #[template_child]
        go_back_btn: TemplateChild<gtk::Button>,
        #[template_child]
        stack: TemplateChild<gtk::Stack>,
        #[template_child]
        entry_page: TemplateChild<gtk::Box>,
        #[template_child]
        search_entry: TemplateChild<gtk::SearchEntry>,
        #[template_child]
        look_up_btn: TemplateChild<LoadingButton>,
        #[template_child]
        room_avatar: TemplateChild<Avatar>,
        #[template_child]
        room_name: TemplateChild<gtk::Label>,
        #[template_child]
        room_alias: TemplateChild<gtk::Label>,
        #[template_child]
        room_topic: TemplateChild<gtk::Label>,
        #[template_child]
        room_members_box: TemplateChild<gtk::Box>,
        #[template_child]
        room_members_count: TemplateChild<gtk::Label>,
        #[template_child]
        join_btn: TemplateChild<LoadingButton>,
        /// The current session.
        #[property(get, set = Self::set_session, construct_only)]
        session: glib::WeakRef<Session>,
        /// The URI to preview.
        uri: RefCell<Option<MatrixRoomIdUri>>,
        /// The room that is previewed.
        #[property(get)]
        room: RefCell<Option<RemoteRoom>>,
        /// Whether the "Go back" button is disabled.
        disable_go_back: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for JoinRoomDialog {
        const NAME: &'static str = "JoinRoomDialog";
        type Type = super::JoinRoomDialog;
        type ParentType = ToastableDialog;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for JoinRoomDialog {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            self.room_topic.connect_activate_link(clone!(
                #[weak]
                obj,
                #[upgrade_or]
                glib::Propagation::Proceed,
                move |_, uri| {
                    let Ok(uri) = MatrixIdUri::parse(uri) else {
                        return glib::Propagation::Proceed;
                    };
                    let Some(parent_window) =
                        obj.ancestor(Window::static_type()).and_downcast::<Window>()
                    else {
                        return glib::Propagation::Proceed;
                    };

                    parent_window.session_view().show_matrix_uri(uri);
                    glib::Propagation::Stop
                }
            ));
        }
    }

    impl WidgetImpl for JoinRoomDialog {}
    impl AdwDialogImpl for JoinRoomDialog {}
    impl ToastableDialogImpl for JoinRoomDialog {}

    #[gtk::template_callbacks]
    impl JoinRoomDialog {
        /// Set the current session.
        fn set_session(&self, session: Option<&Session>) {
            self.session.set(session);

            self.obj().notify_session();
            self.update_entry_page();
        }

        /// Set the room URI to look up.
        pub(super) fn set_uri(&self, uri: MatrixRoomIdUri) {
            self.uri.replace(Some(uri.clone()));
            self.disable_go_back(true);
            self.set_visible_page("loading");

            self.look_up_room_inner(uri);
        }

        /// Set the room that is previewed.
        pub(super) fn set_room(&self, room: Option<RemoteRoom>) {
            if *self.room.borrow() == room {
                return;
            }

            self.room.replace(room.clone());

            if let Some(room) = room {
                if matches!(
                    room.loading_state(),
                    LoadingState::Ready | LoadingState::Error
                ) {
                    self.fill_details();
                } else {
                    room.connect_loading_state_notify(clone!(
                        #[weak(rename_to = imp)]
                        self,
                        move |room| {
                            if matches!(
                                room.loading_state(),
                                LoadingState::Ready | LoadingState::Error
                            ) {
                                imp.fill_details();
                            }
                        }
                    ));
                }
            }

            self.obj().notify_room();
        }

        /// Se whether to disable the "Go back" button.
        pub(super) fn disable_go_back(&self, disable: bool) {
            self.disable_go_back.set(disable);
        }

        /// Whether we can go back to the previous screen.
        fn can_go_back(&self) -> bool {
            !self.disable_go_back.get()
                && self.stack.visible_child_name().as_deref() == Some("details")
        }

        /// Set the currently visible page.
        fn set_visible_page(&self, page_name: &str) {
            self.stack.set_visible_child_name(page_name);
            self.go_back_btn.set_visible(self.can_go_back());
        }

        /// Update the state of the entry page.
        #[template_callback]
        fn update_entry_page(&self) {
            let Some(session) = self.session.upgrade() else {
                self.entry_page.set_sensitive(false);
                return;
            };
            self.entry_page.set_sensitive(true);

            let Some(uri) = MatrixRoomIdUri::parse(&self.search_entry.text()) else {
                self.look_up_btn.set_sensitive(false);
                self.uri.take();
                return;
            };
            self.look_up_btn.set_sensitive(true);

            let id = uri.id.clone();
            self.uri.replace(Some(uri));

            if session.room_list().joined_room(&id).is_some() {
                // Translators: This is a verb, as in 'View Room'.
                self.look_up_btn.set_content_label(gettext("View"));
            } else {
                // Translators: This is a verb, as in 'Look up Room'.
                self.look_up_btn.set_content_label(gettext("Look Up"));
            }
        }

        /// Look up the room that was entered, if it is valid.
        ///
        /// If the room is known, this will open it instead.
        #[template_callback]
        fn look_up_room(&self) {
            let Some(uri) = self.uri.borrow().clone() else {
                return;
            };
            let obj = self.obj();

            let Some(window) = obj.root().and_downcast::<Window>() else {
                return;
            };

            self.look_up_btn.set_is_loading(true);
            self.entry_page.set_sensitive(false);

            // Join or view the room with the given identifier.
            if window.session_view().select_room_if_exists(&uri.id) {
                obj.close();
            } else {
                self.look_up_room_inner(uri);
            }
        }

        fn look_up_room_inner(&self, uri: MatrixRoomIdUri) {
            let Some(session) = self.session.upgrade() else {
                return;
            };

            // Reset state before switching to possible pages.
            self.go_back_btn.set_sensitive(true);
            self.join_btn.set_is_loading(false);

            let room = RemoteRoom::new(&session, uri);
            self.set_room(Some(room));
        }

        /// Fill the details with the given result.
        fn fill_details(&self) {
            let Some(room) = self.room.borrow().clone() else {
                return;
            };

            self.room_name.set_label(&room.display_name());

            let alias = room.alias();
            if let Some(alias) = &alias {
                self.room_alias.set_label(alias.as_str());
            }
            self.room_alias
                .set_visible(room.name().is_some() && alias.is_some());

            self.room_avatar.set_data(Some(room.avatar_data()));

            if room.loading_state() == LoadingState::Error {
                self.room_topic.set_label(&gettext(
                "The room details cannot be previewed. It can be because the room is not known by the homeserver or because its details are private. You can still try to join it."
            ));
                self.room_topic.set_visible(true);
                self.room_members_box.set_visible(false);

                self.set_visible_page("details");
                return;
            }

            if let Some(topic) = room.topic_linkified() {
                self.room_topic.set_label(&topic);
                self.room_topic.set_visible(true);
            } else {
                self.room_topic.set_visible(false);
            }

            let members_count = room.joined_members_count();
            self.room_members_count
                .set_label(&members_count.to_string());

            let members_tooltip = ngettext_f(
                // Translators: Do NOT translate the content between '{' and '}',
                // this is a variable name.
                "1 member",
                "{n} members",
                members_count,
                &[("n", &members_count.to_string())],
            );
            self.room_members_box
                .set_tooltip_text(Some(&members_tooltip));
            self.room_members_box.set_visible(true);

            self.set_visible_page("details");
        }

        /// Join the room that was entered, if it is valid.
        #[template_callback]
        async fn join_room(&self) {
            let Some(session) = self.session.upgrade() else {
                return;
            };
            let Some(room) = self.room.borrow().clone() else {
                return;
            };

            self.go_back_btn.set_sensitive(false);
            self.join_btn.set_is_loading(true);

            // Join the room with the given identifier.
            let room_list = session.room_list();
            let uri = room.uri().clone();

            match room_list.join_by_id_or_alias(uri.id, uri.via).await {
                Ok(room_id) => {
                    let obj = self.obj();

                    if let Some(room) = room_list.get_wait(&room_id).await {
                        if let Some(window) = obj.root().and_downcast_ref::<Window>() {
                            window.session_view().select_room(room);
                        }
                    }

                    obj.close();
                }
                Err(error) => {
                    let obj = self.obj();
                    toast!(obj, error);

                    self.join_btn.set_is_loading(false);
                    self.go_back_btn.set_sensitive(true);
                }
            }
        }

        /// Go back to the previous screen.
        ///
        /// If we can't go back, closes the window.
        #[template_callback]
        fn go_back(&self) {
            if self.can_go_back() {
                // There is only one screen to go back to.
                self.look_up_btn.set_is_loading(false);
                self.entry_page.set_sensitive(true);
                self.set_visible_page("entry");
            } else {
                self.obj().close();
            }
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
    pub(crate) fn set_uri(&self, uri: MatrixRoomIdUri) {
        self.imp().set_uri(uri);
    }

    /// Set the room to preview.
    pub(crate) fn set_room(&self, room: RemoteRoom) {
        let imp = self.imp();
        imp.disable_go_back(true);
        imp.set_room(Some(room));
    }
}
