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
        #[template_child]
        pub ignored_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub ignored_button: TemplateChild<SpinnerButton>,
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
            Self::Type::bind_template_callbacks(klass);

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
            let is_ignored_handler = user.connect_is_ignored_notify(clone!(@weak obj => move |_| {
                obj.update_direct_chat();
                obj.update_ignored();
            }));

            // We don't need to listen to changes of the property, it never changes after
            // construction.
            let is_own_user = user.is_own_user();
            self.ignored_row.set_visible(!is_own_user);

            self.user
                .set(user, vec![is_verified_handler, is_ignored_handler]);

            spawn!(clone!(@weak obj => async move {
                obj.load_direct_chat().await;
            }));
            obj.update_direct_chat();
            obj.update_verified();
            obj.update_ignored();
        }
    }
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

    /// Update the visibility of the direct chat button.
    fn update_direct_chat(&self) {
        let is_visible = self
            .user()
            .is_some_and(|u| !u.is_own_user() && !u.is_ignored());
        self.imp().direct_chat_button.set_visible(is_visible);
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
        let verification = match user.verify_identity().await {
            Ok(verification) => verification,
            Err(()) => {
                toast!(self, gettext("Failed to start user verification"));
                self.action_set_enabled("user-page.verify-user", true);
                imp.verify_button.set_loading(false);
                return;
            }
        };

        let Some(parent_window) = self.root().and_downcast::<gtk::Window>() else {
            return;
        };

        if let Some(main_window) = parent_window.transient_for().and_downcast::<Window>() {
            main_window.show_verification(user.session().session_id(), verification);
        }

        parent_window.close();
    }

    /// Update the ignored row.
    fn update_ignored(&self) {
        let Some(user) = self.user() else {
            return;
        };
        let imp = self.imp();

        if user.is_ignored() {
            imp.ignored_row.set_title(&gettext("Ignored"));
            imp.ignored_button.set_label(gettext("Stop Ignoring"));
            imp.ignored_button.remove_css_class("destructive-action");
        } else {
            imp.ignored_row.set_title(&gettext("Not Ignored"));
            imp.ignored_button.set_label(gettext("Ignore"));
            imp.ignored_button.add_css_class("destructive-action");
        }
    }

    /// Toggle whether the user is ignored or not.
    #[template_callback]
    fn toggle_ignored(&self) {
        let Some(user) = self.user() else {
            return;
        };
        let is_ignored = user.is_ignored();

        self.imp().ignored_button.set_loading(true);

        spawn!(clone!(@weak self as obj, @weak user => async move {
            if is_ignored {
                if user.stop_ignoring().await.is_err() {
                    toast!(obj, gettext("Failed to stop ignoring user"));
                }
            } else if user.ignore().await.is_err() {
                toast!(obj, gettext("Failed to ignore user"));
            }

            obj.imp().ignored_button.set_loading(false);
        }));
    }
}
