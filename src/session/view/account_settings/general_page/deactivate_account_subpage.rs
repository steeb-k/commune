use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{glib, CompositeTemplate};
use tracing::error;

use crate::{
    components::{AuthDialog, LoadingButtonRow},
    prelude::*,
    session::model::Session,
    toast,
};

mod imp {
    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/account_settings/general_page/deactivate_account_subpage.ui"
    )]
    #[properties(wrapper_type = super::DeactivateAccountSubpage)]
    pub struct DeactivateAccountSubpage {
        /// The current session.
        #[property(get, set = Self::set_session, nullable)]
        pub session: glib::WeakRef<Session>,
        #[template_child]
        pub confirmation: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub button: TemplateChild<LoadingButtonRow>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DeactivateAccountSubpage {
        const NAME: &'static str = "DeactivateAccountSubpage";
        type Type = super::DeactivateAccountSubpage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for DeactivateAccountSubpage {}

    impl WidgetImpl for DeactivateAccountSubpage {}
    impl NavigationPageImpl for DeactivateAccountSubpage {}

    impl DeactivateAccountSubpage {
        /// Set the current session.
        fn set_session(&self, session: Option<Session>) {
            if let Some(session) = session {
                self.session.set(Some(&session));
                self.confirmation.set_title(session.user_id().as_str());
            }
        }
    }
}

glib::wrapper! {
    /// Account settings page about the user and the session.
    pub struct DeactivateAccountSubpage(ObjectSubclass<imp::DeactivateAccountSubpage>)
        @extends gtk::Widget, adw::NavigationPage, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl DeactivateAccountSubpage {
    pub fn new(session: &Session) -> Self {
        glib::Object::builder().property("session", session).build()
    }

    /// Update the state of the button.
    #[template_callback]
    fn update_button(&self) {
        self.imp()
            .button
            .set_sensitive(self.can_deactivate_account());
    }

    fn can_deactivate_account(&self) -> bool {
        let confirmation = &self.imp().confirmation;
        confirmation.text() == confirmation.title()
    }

    /// Deactivate the account of the current session.
    #[template_callback]
    async fn deactivate_account(&self) {
        let Some(session) = self.session() else {
            return;
        };

        if !self.can_deactivate_account() {
            return;
        }

        let imp = self.imp();
        imp.button.set_is_loading(true);
        imp.confirmation.set_sensitive(false);

        let dialog = AuthDialog::new(&session);

        let result = dialog
            .authenticate(self, move |client, auth| async move {
                client
                    .account()
                    .deactivate(None, auth, false)
                    .await
                    .map_err(Into::into)
            })
            .await;

        match result {
            Ok(_) => {
                if let Some(session) = self.session() {
                    if let Some(window) = self.root().and_downcast_ref::<gtk::Window>() {
                        toast!(window, gettext("Account successfully deactivated"));
                    }
                    session.clean_up().await;
                }
                self.activate_action("account-settings.close", None)
                    .unwrap();
            }
            Err(error) => {
                error!("Could not deactivate account: {error:?}");
                toast!(self, gettext("Could not deactivate account"));
            }
        }
        imp.button.set_is_loading(false);
        imp.confirmation.set_sensitive(true);
    }
}
