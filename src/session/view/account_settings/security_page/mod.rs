use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{glib, glib::clone, CompositeTemplate};

mod ignored_users_subpage;
mod import_export_keys_subpage;

pub use self::{
    ignored_users_subpage::IgnoredUsersSubpage,
    import_export_keys_subpage::{ImportExportKeysSubpage, ImportExportKeysSubpageMode},
};
use crate::{
    components::{ButtonCountRow, ButtonRow},
    session::model::Session,
    spawn, spawn_tokio,
};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/account_settings/security_page/mod.ui"
    )]
    #[properties(wrapper_type = super::SecurityPage)]
    pub struct SecurityPage {
        #[template_child]
        pub public_read_receipts_row: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub typing_row: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub ignored_users_row: TemplateChild<ButtonCountRow>,
        #[template_child]
        pub master_key_status: TemplateChild<gtk::Label>,
        #[template_child]
        pub self_signing_key_status: TemplateChild<gtk::Label>,
        #[template_child]
        pub user_signing_key_status: TemplateChild<gtk::Label>,
        /// The current session.
        #[property(get, set = Self::set_session, nullable)]
        pub session: glib::WeakRef<Session>,
        pub ignored_users_count_handler: RefCell<Option<glib::SignalHandlerId>>,
        bindings: RefCell<Vec<glib::Binding>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SecurityPage {
        const NAME: &'static str = "SecurityPage";
        type Type = super::SecurityPage;
        type ParentType = adw::PreferencesPage;

        fn class_init(klass: &mut Self::Class) {
            ButtonRow::ensure_type();

            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for SecurityPage {
        fn dispose(&self) {
            if let Some(session) = self.session.upgrade() {
                if let Some(handler) = self.ignored_users_count_handler.take() {
                    session.ignored_users().disconnect(handler);
                }
            }

            for binding in self.bindings.take() {
                binding.unbind();
            }
        }
    }

    impl WidgetImpl for SecurityPage {}
    impl PreferencesPageImpl for SecurityPage {}

    impl SecurityPage {
        /// Set the current session.
        fn set_session(&self, session: Option<Session>) {
            let prev_session = self.session.upgrade();

            if prev_session == session {
                return;
            }
            let obj = self.obj();

            if let Some(session) = prev_session {
                if let Some(handler) = self.ignored_users_count_handler.take() {
                    session.ignored_users().disconnect(handler);
                }
            }
            for binding in self.bindings.take() {
                binding.unbind();
            }

            if let Some(session) = &session {
                let ignored_users = session.ignored_users();
                let ignored_users_count_handler = ignored_users.connect_items_changed(
                    clone!(@weak self as imp => move |ignored_users, _, _, _| {
                        imp.ignored_users_row.set_count(ignored_users.n_items().to_string());
                    }),
                );
                self.ignored_users_row
                    .set_count(ignored_users.n_items().to_string());

                self.ignored_users_count_handler
                    .replace(Some(ignored_users_count_handler));

                let session_settings = session.settings();

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

            self.session.set(session.as_ref());
            obj.notify_session();

            spawn!(clone!(@weak obj => async move {
                obj.load_cross_signing_status().await;
            }));
        }
    }
}

glib::wrapper! {
    /// Security settings page.
    pub struct SecurityPage(ObjectSubclass<imp::SecurityPage>)
        @extends gtk::Widget, adw::PreferencesPage, @implements gtk::Accessible;
}

impl SecurityPage {
    pub fn new(session: &Session) -> Self {
        glib::Object::builder().property("session", session).build()
    }

    async fn load_cross_signing_status(&self) {
        let Some(session) = self.session() else {
            return;
        };
        let client = session.client();

        let cross_signing_status =
            spawn_tokio!(async move { client.encryption().cross_signing_status().await })
                .await
                .unwrap();

        let imp = self.imp();
        update_cross_signing_key_status(
            &imp.master_key_status,
            cross_signing_status
                .as_ref()
                .map(|s| s.has_master)
                .unwrap_or_default(),
        );
        update_cross_signing_key_status(
            &imp.self_signing_key_status,
            cross_signing_status
                .as_ref()
                .map(|s| s.has_self_signing)
                .unwrap_or_default(),
        );
        update_cross_signing_key_status(
            &imp.user_signing_key_status,
            cross_signing_status
                .as_ref()
                .map(|s| s.has_user_signing)
                .unwrap_or_default(),
        );
    }
}

fn update_cross_signing_key_status(label: &gtk::Label, available: bool) {
    if available {
        label.add_css_class("success");
        label.remove_css_class("error");
        // Translators: As in "The signing key is available".
        label.set_text(&gettext("Available"));
    } else {
        label.add_css_class("error");
        label.remove_css_class("success");
        // Translators: As in "The signing key is not available".
        label.set_text(&gettext("Not available"));
    }
}
