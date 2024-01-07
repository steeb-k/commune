use adw::subclass::prelude::*;
use gtk::{glib, glib::clone, prelude::*, CompositeTemplate};

mod user_session;
mod user_session_row;
mod user_sessions_list;
mod user_sessions_list_item;

use self::{
    user_session::UserSession,
    user_session_row::UserSessionRow,
    user_sessions_list::UserSessionsList,
    user_sessions_list_item::{UserSessionsListItem, UserSessionsListItemType},
};
use crate::{components::LoadingRow, session::model::User};

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
        #[template_child]
        pub other_sessions_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub other_sessions: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub current_session: TemplateChild<gtk::ListBox>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for UserSessionsPage {
        const NAME: &'static str = "UserSessionsPage";
        type Type = super::UserSessionsPage;
        type ParentType = adw::PreferencesPage;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
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
                let user_sessions_list = UserSessionsList::new(&user.session());
                self.other_sessions.bind_model(
                    Some(&user_sessions_list),
                    clone!(@weak user_sessions_list => @default-panic, move |item| {
                        match item.downcast_ref::<UserSessionsListItem>().unwrap().item_type() {
                            UserSessionsListItemType::UserSession(user_session) => UserSessionRow::new(&user_session, false).upcast(),
                            UserSessionsListItemType::Error(error) => {
                                let row = LoadingRow::new();
                                row.set_error(Some(error.clone()));
                                row.connect_retry(clone!(@weak user_sessions_list => move|_| {
                                    user_sessions_list.load()
                                }));
                                row.upcast()
                            }
                            UserSessionsListItemType::LoadingSpinner => {
                                LoadingRow::new().upcast()
                            }
                        }
                    }),
                );

                user_sessions_list.connect_items_changed(
                    clone!(@weak obj => move |user_sessions_list, _, _, _| {
                        obj.set_other_sessions_visibility(user_sessions_list.n_items() > 0)
                    }),
                );

                obj.set_other_sessions_visibility(user_sessions_list.n_items() > 0);

                user_sessions_list.connect_current_user_session_notify(
                    clone!(@weak obj => move |user_sessions_list| {
                        obj.set_current_user_session(user_sessions_list);
                    }),
                );

                obj.set_current_user_session(&user_sessions_list);
            } else {
                self.other_sessions.unbind_model();

                if let Some(child) = self.current_session.first_child() {
                    self.current_session.remove(&child);
                }
            }

            self.user.replace(user);
            obj.notify_user();
        }
    }
}

glib::wrapper! {
    /// User sessions page.
    pub struct UserSessionsPage(ObjectSubclass<imp::UserSessionsPage>)
        @extends gtk::Widget, gtk::Window, adw::Window, adw::PreferencesWindow, @implements gtk::Accessible;
}

impl UserSessionsPage {
    pub fn new(user: &User) -> Self {
        glib::Object::builder().property("user", user).build()
    }

    fn set_other_sessions_visibility(&self, visible: bool) {
        self.imp().other_sessions_group.set_visible(visible);
    }

    fn set_current_user_session(&self, user_sessions_list: &UserSessionsList) {
        let imp = self.imp();
        if let Some(child) = imp.current_session.first_child() {
            imp.current_session.remove(&child);
        }
        let row: gtk::Widget = match user_sessions_list.current_user_session().item_type() {
            UserSessionsListItemType::UserSession(user_session) => {
                UserSessionRow::new(&user_session, true).upcast()
            }
            UserSessionsListItemType::Error(error) => {
                let row = LoadingRow::new();
                row.set_error(Some(error.clone()));
                row.connect_retry(clone!(@weak user_sessions_list => move|_| {
                    user_sessions_list.load()
                }));
                row.upcast()
            }
            UserSessionsListItemType::LoadingSpinner => LoadingRow::new().upcast(),
        };
        imp.current_session.append(&row);
    }
}
