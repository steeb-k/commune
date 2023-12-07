use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{glib, glib::clone, CompositeTemplate};

use crate::{
    components::{Avatar, SpinnerButton},
    prelude::*,
    session::model::User,
    toast,
    utils::BoundObject,
    Window,
};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/Fractal/ui/session/view/user_page.ui")]
    pub struct UserPage {
        #[template_child]
        pub avatar: TemplateChild<Avatar>,
        #[template_child]
        pub verified_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub verified_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub verify_button: TemplateChild<SpinnerButton>,
        /// The current user.
        pub user: BoundObject<User>,
        pub bindings: RefCell<Vec<glib::Binding>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for UserPage {
        const NAME: &'static str = "UserPage";
        type Type = super::UserPage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);

            klass.install_action_async("user-page.verify-user", None, |widget, _, _| async move {
                widget.verify_user().await;
            });
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for UserPage {
        fn properties() -> &'static [glib::ParamSpec] {
            use once_cell::sync::Lazy;
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![glib::ParamSpecObject::builder::<User>("user")
                    .construct_only()
                    .build()]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            match pspec.name() {
                "user" => self.obj().set_user(value.get().unwrap()),
                _ => unimplemented!(),
            }
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "user" => self.obj().user().to_value(),
                _ => unimplemented!(),
            }
        }

        fn dispose(&self) {
            for binding in self.bindings.take() {
                binding.unbind();
            }
        }
    }

    impl WidgetImpl for UserPage {}
    impl NavigationPageImpl for UserPage {}
}

glib::wrapper! {
    /// Page to view details about a user.
    pub struct UserPage(ObjectSubclass<imp::UserPage>)
        @extends gtk::Widget, adw::NavigationPage, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl UserPage {
    /// Construct a new `UserPage` for the given user.
    pub fn new(user: &impl IsA<User>) -> Self {
        glib::Object::builder().property("user", user).build()
    }

    /// The current user.
    pub fn user(&self) -> Option<User> {
        self.imp().user.obj()
    }

    /// Set the current user.
    fn set_user(&self, user: Option<User>) {
        let Some(user) = user else {
            // Ignore missing user.
            return;
        };
        let imp = self.imp();

        let title_binding = user
            .bind_property("display-name", self, "title")
            .sync_create()
            .build();
        let avatar_binding = user
            .bind_property("avatar-data", &*imp.avatar, "data")
            .sync_create()
            .build();
        imp.bindings.replace(vec![title_binding, avatar_binding]);

        let is_verified_handler = user.connect_notify_local(
            Some("is-verified"),
            clone!(@weak self as obj => move |_, _| {
                obj.update_verified();
            }),
        );

        self.imp().user.set(user, vec![is_verified_handler]);
        self.update_verified();
    }

    /// Update the verified row.
    fn update_verified(&self) {
        let Some(user) = self.user() else {
            return;
        };
        let imp = self.imp();

        if user.is_verified() {
            imp.verified_row.set_title(&gettext("Identity verified"));
            imp.verified_stack.set_visible_child_name("icon");
            self.action_set_enabled("user-page.verify-user", false);
        } else {
            self.action_set_enabled("user-page.verify-user", true);
            imp.verified_stack.set_visible_child_name("button");
            imp.verified_row
                .set_title(&gettext("Identity not verified"));
        }
    }

    /// Launch the verification for the current user.
    async fn verify_user(&self) {
        let Some(user) = self.user() else {
            return;
        };
        let imp = self.imp();

        self.action_set_enabled("user-page.verify-user", false);
        imp.verify_button.set_loading(true);
        let verification = user.verify_identity().await;

        let Some(flow_id) = verification.flow_id() else {
            toast!(self, gettext("Failed to start user verification"));
            self.action_set_enabled("user-page.verify-user", true);
            imp.verify_button.set_loading(false);

            return;
        };

        let Some(parent_window) = self.root().and_downcast::<gtk::Window>() else {
            return;
        };

        if let Some(main_window) = parent_window.transient_for().and_downcast::<Window>() {
            main_window.show_verification(user.session().session_id(), &user.user_id(), flow_id);
        }

        parent_window.close();
    }
}
