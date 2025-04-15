use adw::{prelude::*, subclass::prelude::*};
use gtk::{glib, glib::clone, CompositeTemplate};
use tracing::error;

mod ignored_users_subpage;

pub(super) use self::ignored_users_subpage::IgnoredUsersSubpage;
use crate::{
    components::ButtonCountRow,
    session::model::{MediaPreviewsGlobalSetting, Session},
};

mod imp {
    use std::{cell::RefCell, marker::PhantomData};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session/view/account_settings/safety_page/mod.ui")]
    #[properties(wrapper_type = super::SafetyPage)]
    pub struct SafetyPage {
        #[template_child]
        public_read_receipts_row: TemplateChild<adw::SwitchRow>,
        #[template_child]
        typing_row: TemplateChild<adw::SwitchRow>,
        #[template_child]
        ignored_users_row: TemplateChild<ButtonCountRow>,
        /// The current session.
        #[property(get, set = Self::set_session, nullable)]
        session: glib::WeakRef<Session>,
        /// The media previews setting, as a string.
        #[property(get = Self::media_previews_enabled, set = Self::set_media_previews_enabled)]
        media_previews_enabled: PhantomData<String>,
        ignored_users_count_handler: RefCell<Option<glib::SignalHandlerId>>,
        session_settings_handler: RefCell<Option<glib::SignalHandlerId>>,
        bindings: RefCell<Vec<glib::Binding>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SafetyPage {
        const NAME: &'static str = "SafetyPage";
        type Type = super::SafetyPage;
        type ParentType = adw::PreferencesPage;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);

            klass.install_property_action(
                "safety.set-media-previews-enabled",
                "media-previews-enabled",
            );
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for SafetyPage {
        fn dispose(&self) {
            self.clear();
        }
    }

    impl WidgetImpl for SafetyPage {}
    impl PreferencesPageImpl for SafetyPage {}

    impl SafetyPage {
        /// Set the current session.
        fn set_session(&self, session: Option<&Session>) {
            if self.session.upgrade().as_ref() == session {
                return;
            }

            self.clear();
            let obj = self.obj();

            if let Some(session) = session {
                let ignored_users = session.ignored_users();
                let ignored_users_count_handler = ignored_users.connect_items_changed(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |ignored_users, _, _, _| {
                        imp.ignored_users_row
                            .set_count(ignored_users.n_items().to_string());
                    }
                ));
                self.ignored_users_row
                    .set_count(ignored_users.n_items().to_string());

                self.ignored_users_count_handler
                    .replace(Some(ignored_users_count_handler));

                let session_settings = session.settings();

                let media_previews_handler = session_settings
                    .connect_media_previews_enabled_changed(clone!(
                        #[weak]
                        obj,
                        move |_| {
                            // Update the active media previews radio button.
                            obj.notify_media_previews_enabled();
                        }
                    ));
                self.session_settings_handler
                    .replace(Some(media_previews_handler));

                let public_read_receipts_binding = session_settings
                    .bind_property(
                        "public-read-receipts-enabled",
                        &*self.public_read_receipts_row,
                        "active",
                    )
                    .bidirectional()
                    .sync_create()
                    .build();
                let typing_binding = session_settings
                    .bind_property("typing-enabled", &*self.typing_row, "active")
                    .bidirectional()
                    .sync_create()
                    .build();

                self.bindings
                    .replace(vec![public_read_receipts_binding, typing_binding]);
            }

            self.session.set(session);

            // Update the active media previews radio button.
            obj.notify_media_previews_enabled();
            obj.notify_session();
        }

        /// The media previews setting, as a string.
        fn media_previews_enabled(&self) -> String {
            let Some(session) = self.session.upgrade() else {
                return String::new();
            };

            session
                .settings()
                .media_previews_global_enabled()
                .as_ref()
                .to_owned()
        }

        /// Set the media previews setting, as a string.
        fn set_media_previews_enabled(&self, setting: &str) {
            let Some(session) = self.session.upgrade() else {
                return;
            };

            let Ok(setting) = setting.parse::<MediaPreviewsGlobalSetting>() else {
                error!("Invalid value to set global media previews setting: {setting}");
                return;
            };

            session
                .settings()
                .set_media_previews_global_enabled(setting);
        }

        /// Reset the signal handlers and bindings.
        fn clear(&self) {
            if let Some(session) = self.session.upgrade() {
                if let Some(handler) = self.ignored_users_count_handler.take() {
                    session.ignored_users().disconnect(handler);
                }

                if let Some(handler) = self.session_settings_handler.take() {
                    session.settings().disconnect(handler);
                }
            }

            for binding in self.bindings.take() {
                binding.unbind();
            }
        }
    }
}

glib::wrapper! {
    /// Safety settings page.
    pub struct SafetyPage(ObjectSubclass<imp::SafetyPage>)
        @extends gtk::Widget, adw::PreferencesPage, @implements gtk::Accessible;
}

impl SafetyPage {
    pub fn new(session: &Session) -> Self {
        glib::Object::builder().property("session", session).build()
    }
}
