use adw::subclass::prelude::*;
use gtk::{glib, glib::clone, prelude::*, CompositeTemplate};
use tracing::error;

mod user_session;
mod user_session_row;
mod user_sessions_list;

use self::{
    user_session::UserSession, user_session_row::UserSessionRow,
    user_sessions_list::UserSessionsList,
};
use crate::{prelude::*, session::model::User, spawn, utils::LoadingState};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/account_settings/user_sessions_page/mod.ui"
    )]
    #[properties(wrapper_type = super::UserSessionsPage)]
    pub struct UserSessionsPage {
        /// The logged-in user.
        #[property(get, set = Self::set_user, explicit_notify)]
        pub user: RefCell<Option<User>>,
        /// The list of user sessions.
        #[property(get)]
        pub user_sessions: RefCell<Option<UserSessionsList>>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub current_session_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub current_session: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub other_sessions_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub other_sessions: TemplateChild<gtk::ListBox>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for UserSessionsPage {
        const NAME: &'static str = "UserSessionsPage";
        type Type = super::UserSessionsPage;
        type ParentType = adw::PreferencesPage;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for UserSessionsPage {}

    impl WidgetImpl for UserSessionsPage {}
    impl PreferencesPageImpl for UserSessionsPage {}

    impl UserSessionsPage {
        /// Set the logged-in user.
        fn set_user(&self, user: Option<User>) {
            if *self.user.borrow() == user {
                return;
            }
            let obj = self.obj();

            if let Some(user) = &user {
                let user_sessions = UserSessionsList::new(&user.session(), user.user_id().clone());

                user_sessions.connect_loading_state_notify(clone!(@weak self as imp => move |_| {
                    imp.update_state();
                }));

                user_sessions.connect_is_empty_notify(clone!(@weak self as imp => move |_| {
                    imp.update_state();
                }));

                user_sessions.connect_current_session_notify(
                    clone!(@weak self as imp => move |_| {
                        imp.update_current_session();
                    }),
                );

                let other_sessions = user_sessions.other_sessions();
                self.other_sessions
                    .bind_model(Some(&other_sessions), |item| {
                        let Some(user_session) = item.downcast_ref::<UserSession>() else {
                            error!("Did not get a user session as an item of user session list");
                            return adw::Bin::new().upcast();
                        };

                        UserSessionRow::new(user_session).upcast()
                    });

                other_sessions.connect_items_changed(
                    clone!(@weak self as imp => move |other_sessions, _, _, _| {
                        imp.other_sessions_group.set_visible(other_sessions.n_items() > 0);
                    }),
                );
                self.other_sessions_group
                    .set_visible(other_sessions.n_items() > 0);

                self.user_sessions.replace(Some(user_sessions));
            } else {
                self.other_sessions.unbind_model();
                self.user_sessions.take();
            }

            self.user.replace(user);
            obj.notify_user();

            self.update_current_session();
            self.update_state();
        }

        /// Update the state of the UI according to the current state.
        fn update_state(&self) {
            let (is_empty, state) = self
                .user_sessions
                .borrow()
                .as_ref()
                .map(|s| (s.is_empty(), s.loading_state()))
                .unwrap_or((true, LoadingState::Loading));

            let page = if is_empty {
                match state {
                    LoadingState::Error | LoadingState::Ready => "error",
                    _ => "loading",
                }
            } else {
                "list"
            };
            self.stack.set_visible_child_name(page);
        }

        /// Update the section about the current session.
        fn update_current_session(&self) {
            if let Some(child) = self.current_session.first_child() {
                self.current_session.remove(&child);
            }

            let current_session = self
                .user_sessions
                .borrow()
                .as_ref()
                .and_then(|s| s.current_session());
            let Some(current_session) = current_session else {
                self.current_session_group.set_visible(false);
                return;
            };

            self.current_session
                .append(&UserSessionRow::new(&current_session));
            self.current_session_group.set_visible(true);
        }
    }
}

glib::wrapper! {
    /// User sessions page.
    pub struct UserSessionsPage(ObjectSubclass<imp::UserSessionsPage>)
        @extends gtk::Widget, gtk::Window, adw::Window, adw::PreferencesWindow, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl UserSessionsPage {
    pub fn new(user: &User) -> Self {
        glib::Object::builder().property("user", user).build()
    }

    /// Reload the user sessions list.
    #[template_callback]
    fn reload_list(&self) {
        let Some(user_sessions) = self.user_sessions() else {
            return;
        };

        spawn!(clone!(@weak user_sessions => async move {
            user_sessions.load().await;
        }));
    }
}
