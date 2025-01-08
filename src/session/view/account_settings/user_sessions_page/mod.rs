use adw::subclass::prelude::*;
use gtk::{glib, glib::clone, prelude::*, CompositeTemplate};
use tracing::error;

mod user_session_row;

use self::user_session_row::UserSessionRow;
use super::AccountSettings;
use crate::{
    session::model::{UserSession, UserSessionsList},
    utils::{BoundObject, LoadingState},
};

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
        #[template_child]
        current_session_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        current_session: TemplateChild<gtk::ListBox>,
        #[template_child]
        other_sessions_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        stack: TemplateChild<gtk::Stack>,
        #[template_child]
        other_sessions: TemplateChild<gtk::ListBox>,
        /// The ancestor [`AccountSettings`].
        #[property(get, set = Self::set_account_settings, explicit_notify, nullable)]
        account_settings: glib::WeakRef<AccountSettings>,
        /// The list of user sessions.
        #[property(get, set = Self::set_user_sessions, explicit_notify, nullable)]
        user_sessions: BoundObject<UserSessionsList>,
        other_sessions_handler: RefCell<Option<glib::SignalHandlerId>>,
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
    impl ObjectImpl for UserSessionsPage {
        fn dispose(&self) {
            if let Some(user_sessions) = self.user_sessions.obj() {
                if let Some(handler) = self.other_sessions_handler.take() {
                    user_sessions.other_sessions().disconnect(handler);
                }
            }

            // AdwPreferencesPage doesn't handle children other than AdwPreferencesGroup.
            self.stack.unparent();
        }
    }

    impl WidgetImpl for UserSessionsPage {}
    impl PreferencesPageImpl for UserSessionsPage {}

    impl UserSessionsPage {
        /// Set the ancestor [`AccountSettings`].
        fn set_account_settings(&self, account_settings: Option<&AccountSettings>) {
            self.account_settings.set(account_settings);
            self.update_other_sessions();
        }

        /// Set the list of user sessions.
        fn set_user_sessions(&self, user_sessions: Option<UserSessionsList>) {
            let prev_user_sessions = self.user_sessions.obj();

            if prev_user_sessions == user_sessions {
                return;
            }

            if let Some(user_sessions) = prev_user_sessions {
                if let Some(handler) = self.other_sessions_handler.take() {
                    user_sessions.other_sessions().disconnect(handler);
                }
            }
            self.user_sessions.disconnect_signals();

            if let Some(user_sessions) = user_sessions {
                let other_sessions = user_sessions.other_sessions();

                let other_sessions_handler = other_sessions.connect_items_changed(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |other_sessions, _, _, _| {
                        imp.other_sessions_group
                            .set_visible(other_sessions.n_items() > 0);
                    }
                ));
                self.other_sessions_handler
                    .replace(Some(other_sessions_handler));
                self.other_sessions_group
                    .set_visible(other_sessions.n_items() > 0);

                let loading_state_handler = user_sessions.connect_loading_state_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_other_sessions_state();
                    }
                ));
                let is_empty_handler = user_sessions.connect_is_empty_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_other_sessions_state();
                    }
                ));
                let current_session_handler = user_sessions.connect_current_session_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_current_session();
                    }
                ));

                self.user_sessions.set(
                    user_sessions,
                    vec![
                        loading_state_handler,
                        is_empty_handler,
                        current_session_handler,
                    ],
                );
            }

            self.obj().notify_user_sessions();

            self.update_current_session();
            self.update_other_sessions();
            self.update_other_sessions_state();
        }

        /// Update the list of other sessions.
        fn update_other_sessions(&self) {
            let Some(account_settings) = self.account_settings.upgrade() else {
                self.other_sessions.unbind_model();
                return;
            };
            let Some(user_sessions) = self.user_sessions.obj() else {
                self.other_sessions.unbind_model();
                return;
            };

            self.other_sessions.bind_model(
                Some(&user_sessions.other_sessions()),
                clone!(
                    #[weak]
                    account_settings,
                    #[upgrade_or_else]
                    || adw::Bin::new().upcast(),
                    move |item| {
                        let Some(user_session) = item.downcast_ref::<UserSession>() else {
                            error!("Did not get a user session as an item of user session list");
                            return adw::Bin::new().upcast();
                        };

                        UserSessionRow::new(user_session, &account_settings).upcast()
                    }
                ),
            );
        }

        /// The current page of the other sessions stack according to the
        /// current state.
        fn current_other_sessions_page(&self) -> &str {
            if self.account_settings.upgrade().is_none() {
                return "loading";
            }

            let Some(user_sessions) = self.user_sessions.obj() else {
                return "loading";
            };

            if user_sessions.is_empty() {
                match user_sessions.loading_state() {
                    LoadingState::Error | LoadingState::Ready => "error",
                    _ => "loading",
                }
            } else {
                "list"
            }
        }

        /// Update the state of the UI according to the current state.
        fn update_other_sessions_state(&self) {
            self.stack
                .set_visible_child_name(self.current_other_sessions_page());
        }

        /// Update the section about the current session.
        fn update_current_session(&self) {
            if let Some(child) = self.current_session.first_child() {
                self.current_session.remove(&child);
            }

            let Some(account_settings) = self.account_settings.upgrade() else {
                self.current_session_group.set_visible(false);
                return;
            };

            let current_session = self.user_sessions.obj().and_then(|s| s.current_session());
            let Some(current_session) = current_session else {
                self.current_session_group.set_visible(false);
                return;
            };

            self.current_session
                .append(&UserSessionRow::new(&current_session, &account_settings));
            self.current_session_group.set_visible(true);
        }
    }
}

glib::wrapper! {
    /// Page to present the sessions of a user.
    pub struct UserSessionsPage(ObjectSubclass<imp::UserSessionsPage>)
        @extends gtk::Widget, gtk::Window, adw::Window, adw::PreferencesPage,
        @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl UserSessionsPage {
    /// Construct a new empty `UserSessionsPage`.
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Reload the user sessions list.
    #[template_callback]
    async fn reload_list(&self) {
        let Some(user_sessions) = self.user_sessions() else {
            return;
        };

        user_sessions.load().await;
    }
}

impl Default for UserSessionsPage {
    fn default() -> Self {
        Self::new()
    }
}
