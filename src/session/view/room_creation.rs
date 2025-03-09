use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{glib, CompositeTemplate};
use matrix_sdk::{
    ruma::{
        api::client::{
            error::ErrorKind,
            room::{create_room, Visibility},
        },
        assign,
    },
    Error,
};
use ruma::events::{room::encryption::RoomEncryptionEventContent, InitialStateEvent};
use tracing::error;

use crate::{
    components::{LoadingButton, SubstringEntryRow, ToastableDialog},
    prelude::*,
    session::model::Session,
    spawn_tokio, toast, Window,
};

// MAX length of room addresses
const MAX_BYTES: usize = 255;

mod imp {
    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session/view/room_creation.ui")]
    #[properties(wrapper_type = super::RoomCreation)]
    pub struct RoomCreation {
        /// The current session.
        #[property(get, set = Self::set_session, explicit_notify, nullable)]
        pub session: glib::WeakRef<Session>,
        #[template_child]
        pub create_button: TemplateChild<LoadingButton>,
        #[template_child]
        pub content: TemplateChild<gtk::Box>,
        #[template_child]
        pub room_name: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub topic_text_view: TemplateChild<gtk::TextView>,
        #[template_child]
        pub visibility_private: TemplateChild<gtk::CheckButton>,
        #[template_child]
        pub encryption: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub room_address: TemplateChild<SubstringEntryRow>,
        #[template_child]
        pub room_address_error_revealer: TemplateChild<gtk::Revealer>,
        #[template_child]
        pub room_address_error: TemplateChild<gtk::Label>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RoomCreation {
        const NAME: &'static str = "RoomCreation";
        type Type = super::RoomCreation;
        type ParentType = ToastableDialog;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for RoomCreation {}

    impl WidgetImpl for RoomCreation {}
    impl AdwDialogImpl for RoomCreation {}
    impl ToastableDialogImpl for RoomCreation {}

    impl RoomCreation {
        /// Set the current session.
        fn set_session(&self, session: Option<&Session>) {
            if self.session.upgrade().as_ref() == session {
                return;
            }

            if let Some(session) = session {
                let server_name = session.user_id().server_name();
                self.room_address.set_suffix_text(format!(":{server_name}"));
            }

            self.session.set(session);
            self.obj().notify_session();
        }
    }
}

glib::wrapper! {
    /// Dialog to create a new room.
    pub struct RoomCreation(ObjectSubclass<imp::RoomCreation>)
        @extends gtk::Widget, adw::Dialog, ToastableDialog, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl RoomCreation {
    pub fn new(session: &Session) -> Self {
        glib::Object::builder().property("session", session).build()
    }

    /// Create the room, if it is allowed.
    #[template_callback]
    async fn create_room(&self) {
        if !self.can_create_room() {
            return;
        }

        let Some(session) = self.session() else {
            return;
        };

        let imp = self.imp();
        imp.create_button.set_is_loading(true);
        imp.content.set_sensitive(false);

        let name = Some(imp.room_name.text().trim())
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned);

        let buffer = imp.topic_text_view.buffer();
        let (start_iter, end_iter) = buffer.bounds();
        let topic = Some(buffer.text(&start_iter, &end_iter, false).trim())
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned);

        let mut request = assign!(
            create_room::v3::Request::new(),
            {
                name,
                topic,
            }
        );

        if imp.visibility_private.is_active() {
            // The room is private.
            request.visibility = Visibility::Private;

            if imp.encryption.is_active() {
                let event =
                    InitialStateEvent::new(RoomEncryptionEventContent::with_recommended_defaults());
                request.initial_state = vec![event.to_raw_any()];
            }
        } else {
            // The room is public.
            request.visibility = Visibility::Public;
            request.room_alias_name = Some(imp.room_address.text().to_string());
        }

        let client = session.client();
        let handle = spawn_tokio!(async move { client.create_room(request).await });

        match handle.await.unwrap() {
            Ok(matrix_room) => {
                let Some(window) = self.root().and_downcast::<Window>() else {
                    return;
                };
                let room = session.room_list().get_wait(matrix_room.room_id()).await;
                window.session_view().select_room(room);

                self.close();
            }
            Err(error) => {
                error!("Could not create a new room: {error}");
                self.handle_error(&error);
            }
        }
    }

    /// Display the error that occurred during creation.
    fn handle_error(&self, error: &Error) {
        let imp = self.imp();

        imp.create_button.set_is_loading(false);
        imp.content.set_sensitive(true);

        // Handle the room address already taken error.
        if let Some(kind) = error.client_api_error_kind() {
            if *kind == ErrorKind::RoomInUse {
                imp.room_address.add_css_class("error");
                imp.room_address_error
                    .set_text(&gettext("The address is already taken."));
                imp.room_address_error_revealer.set_reveal_child(true);

                return;
            }
        }

        toast!(self, error.to_user_facing());
    }

    /// Check whether a room can be created with the current input.
    ///
    /// This will also change the UI elements to reflect why the room can't be
    /// created.
    fn can_create_room(&self) -> bool {
        let imp = self.imp();
        let mut can_create = true;

        if imp.room_name.text().is_empty() {
            can_create = false;
        }

        // Only public rooms have an address.
        if imp.visibility_private.is_active() {
            return can_create;
        }

        let room_address = imp.room_address.text();

        // We don't allow #, : in the room address
        let address_has_error = if room_address.contains(':') {
            imp.room_address_error
                .set_text(&gettext("Cannot contain “:”"));
            can_create = false;
            true
        } else if room_address.contains('#') {
            imp.room_address_error
                .set_text(&gettext("Cannot contain “#”"));
            can_create = false;
            true
        } else if room_address.len() > MAX_BYTES {
            imp.room_address_error
                .set_text(&gettext("Too long. Use a shorter address."));
            can_create = false;
            true
        } else if room_address.is_empty() {
            can_create = false;
            false
        } else {
            false
        };

        if address_has_error {
            imp.room_address.add_css_class("error");
        } else {
            imp.room_address.remove_css_class("error");
        }
        imp.room_address_error_revealer
            .set_reveal_child(address_has_error);

        can_create
    }

    /// Validate the form and change the corresponding UI elements.
    #[template_callback]
    fn validate_form(&self) {
        self.imp()
            .create_button
            .set_sensitive(self.can_create_room());
    }
}
