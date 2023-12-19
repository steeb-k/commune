use adw::{prelude::*, subclass::prelude::*};
use gtk::{
    glib::{self, clone},
    CompositeTemplate,
};

use crate::{components::SpinnerButton, session::model::Session, spawn, toast};

mod imp {
    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/account_settings/general_page/log_out_subpage.ui"
    )]
    #[properties(wrapper_type = super::LogOutSubpage)]
    pub struct LogOutSubpage {
        /// The current session.
        #[property(get, set, nullable)]
        pub session: glib::WeakRef<Session>,
        #[template_child]
        pub logout_button: TemplateChild<SpinnerButton>,
        #[template_child]
        pub make_backup_button: TemplateChild<gtk::Button>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LogOutSubpage {
        const NAME: &'static str = "LogOutSubpage";
        type Type = super::LogOutSubpage;
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
    impl ObjectImpl for LogOutSubpage {}

    impl WidgetImpl for LogOutSubpage {}
    impl NavigationPageImpl for LogOutSubpage {}
}

glib::wrapper! {
    /// Account settings page about the user and the session.
    pub struct LogOutSubpage(ObjectSubclass<imp::LogOutSubpage>)
        @extends gtk::Widget, adw::NavigationPage, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl LogOutSubpage {
    pub fn new(session: &Session) -> Self {
        glib::Object::builder().property("session", session).build()
    }

    #[template_callback]
    fn logout_button_clicked_cb(&self) {
        let Some(session) = self.session() else {
            return;
        };

        let imp = self.imp();
        imp.logout_button.set_loading(true);
        imp.make_backup_button.set_sensitive(false);

        spawn!(clone!(@weak self as obj, @weak session => async move {
            if let Err(error) = session.logout().await {
                toast!(obj, error);
            }

            let imp = obj.imp();
            imp.logout_button.set_loading(false);
            imp.make_backup_button.set_sensitive(true);
        }));
    }
}
