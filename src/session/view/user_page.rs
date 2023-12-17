use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{glib, glib::clone, CompositeTemplate};

use crate::{
    components::{Avatar, SpinnerButton},
    prelude::*,
    session::model::User,
    spawn, toast,
    utils::BoundObject,
    Window,
};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session/view/user_page.ui")]
    #[properties(wrapper_type = super::UserPage)]
    pub struct UserPage {
        #[template_child]
        pub avatar: TemplateChild<Avatar>,
        #[template_child]
        pub direct_chat_button: TemplateChild<SpinnerButton>,
        #[template_child]
        pub verified_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub verified_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub verify_button: TemplateChild<SpinnerButton>,
        /// The current user.
        #[property(get, set = Self::set_user, construct_only)]
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

            klass.install_action_async(
                "user-page.open-direct-chat",
                None,
                |widget, _, _| async move {
                    widget.open_direct_chat().await;
                },
            );

            klass.install_action_async("user-page.verify-user", None, |widget, _, _| async move {
                widget.verify_user().await;
            });
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for UserPage {
        fn dispose(&self) {
            for binding in self.bindings.take() {
                binding.unbind();
            }
        }
    }

    impl WidgetImpl for UserPage {}
    impl NavigationPageImpl for UserPage {}

    impl UserPage {
        /// Set the current user.
        fn set_user(&self, user: User) {
            let obj = self.obj();

            let title_binding = user
                .bind_property("display-name", &*obj, "title")
                .sync_create()
                .build();
            let avatar_binding = user
                .bind_property("avatar-data", &*self.avatar, "data")
                .sync_create()
                .build();
            self.bindings.replace(vec![title_binding, avatar_binding]);

            let is_verified_handler = user.connect_verified_notify(clone!(@weak obj => move |_| {
                obj.update_verified();
            }));

            // We don't need to listen to changes of the property, it never changes after
            // construction.
            self.direct_chat_button.set_visible(!user.is_own_user());

            self.user.set(user, vec![is_verified_handler]);

            spawn!(clone!(@weak obj => async move {
                obj.load_direct_chat().await;
            }));
            obj.update_verified();
        }
    }
}

glib::wrapper! {
    /// Page to view details about a user.
    pub struct UserPage(ObjectSubclass<imp::UserPage>)
        @extends gtk::Widget, adw::NavigationPage, @implements gtk::Accessible;
}

impl UserPage {
    /// Construct a new `UserPage` for the given user.
    pub fn new(user: &impl IsA<User>) -> Self {
        glib::Object::builder().property("user", user).build()
    }

    /// Load whether the current user has a direct chat or not.
    async fn load_direct_chat(&self) {
        self.set_direct_chat_loading(true);

        let Some(user) = self.user() else {
            return;
        };

        let direct_chat = user.direct_chat().await;

        let label = if direct_chat.is_some() {
            gettext("Open Direct Chat")
        } else {
            gettext("Create Direct Chat")
        };
        self.imp().direct_chat_button.set_label(label);

        self.set_direct_chat_loading(false);
    }

    /// Set whether the direct chat button is loading.
    fn set_direct_chat_loading(&self, loading: bool) {
        self.action_set_enabled("user-page.open-direct-chat", !loading);
        self.imp().direct_chat_button.set_loading(loading);
    }

    /// Open a direct chat with the current user.
    ///
    /// If one doesn't exist already, it is created.
    async fn open_direct_chat(&self) {
        let Some(user) = self.user() else {
            return;
        };

        self.set_direct_chat_loading(true);

        let Ok(room) = user.get_or_create_direct_chat().await else {
            toast!(self, &gettext("Failed to create a new Direct Chat"));
            self.set_direct_chat_loading(false);

            return;
        };

        let Some(parent_window) = self.root().and_downcast::<gtk::Window>() else {
            return;
        };

        if let Some(main_window) = parent_window.transient_for().and_downcast::<Window>() {
            main_window.show_room(user.session().session_id(), room.room_id());
        }

        parent_window.close();
    }

    /// Update the verified row.
    fn update_verified(&self) {
        let Some(user) = self.user() else {
            return;
        };
        let imp = self.imp();

        if user.verified() {
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
            main_window.show_verification(
                user.session().session_id(),
                &UserExt::user_id(&user),
                flow_id,
            );
        }

        parent_window.close();
    }
}
