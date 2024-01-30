use adw::subclass::prelude::*;
use gtk::{gdk, glib, glib::clone, prelude::*, CompositeTemplate};

mod dm_user;
mod dm_user_list;

use self::{
    dm_user::DmUser,
    dm_user_list::{DmUserList, DmUserListState},
};
use crate::{
    components::{PillSource, PillSourceRow},
    gettext,
    session::model::{Session, User},
    spawn, Window,
};

mod imp {
    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session/view/create_dm_dialog/mod.ui")]
    #[properties(wrapper_type = super::CreateDmDialog)]
    pub struct CreateDmDialog {
        /// The current session.
        #[property(get, set = Self::set_session, explicit_notify)]
        pub session: glib::WeakRef<Session>,
        #[template_child]
        pub list_box: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub search_entry: TemplateChild<gtk::SearchEntry>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub error_page: TemplateChild<adw::StatusPage>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CreateDmDialog {
        const NAME: &'static str = "CreateDmDialog";
        type Type = super::CreateDmDialog;
        type ParentType = adw::Window;

        fn class_init(klass: &mut Self::Class) {
            PillSourceRow::static_type();
            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);

            klass.add_binding_action(
                gdk::Key::Escape,
                gdk::ModifierType::empty(),
                "window.close",
                None,
            );
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for CreateDmDialog {}

    impl WidgetImpl for CreateDmDialog {}
    impl WindowImpl for CreateDmDialog {}
    impl AdwWindowImpl for CreateDmDialog {}

    impl CreateDmDialog {
        /// Set the current session.
        pub fn set_session(&self, session: Option<Session>) {
            if self.session.upgrade() == session {
                return;
            }
            let obj = self.obj();

            if let Some(session) = &session {
                let user_list = DmUserList::new(session);

                // We don't need to disconnect this signal since the `DmUserList` will be
                // disposed once unbound from the `gtk::ListBox`
                user_list.connect_state_notify(clone!(@weak obj => move |model| {
                    obj.update_view(model);
                }));

                self.search_entry
                    .bind_property("text", &user_list, "search-term")
                    .sync_create()
                    .build();

                self.list_box.bind_model(Some(&user_list), |user| {
                    let source = user
                        .downcast_ref::<PillSource>()
                        .expect("DmUserList must contain only `DmUser`");
                    let row = PillSourceRow::new();
                    row.set_source(Some(source.clone()));

                    row.upcast()
                });

                obj.update_view(&user_list);
            } else {
                self.list_box.unbind_model();
            }

            self.session.set(session.as_ref());
            obj.notify_session();
        }
    }
}

glib::wrapper! {
    /// Dialog to create a new direct chat.
    pub struct CreateDmDialog(ObjectSubclass<imp::CreateDmDialog>)
        @extends gtk::Widget, gtk::Window, adw::Window, adw::Bin, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl CreateDmDialog {
    pub fn new(parent_window: Option<&impl IsA<gtk::Window>>, session: &Session) -> Self {
        glib::Object::builder()
            .property("transient-for", parent_window)
            .property("session", session)
            .build()
    }

    fn update_view(&self, model: &DmUserList) {
        let visible_child_name = match model.state() {
            DmUserListState::Initial => "no-search-page",
            DmUserListState::Loading => "loading-page",
            DmUserListState::NoMatching => "no-matching-page",
            DmUserListState::Matching => "matching-page",
            DmUserListState::Error => {
                self.show_error(&gettext("An error occurred while searching for users"));
                return;
            }
        };

        self.imp().stack.set_visible_child_name(visible_child_name);
    }

    fn show_error(&self, message: &str) {
        self.imp().error_page.set_description(Some(message));
        self.imp().stack.set_visible_child_name("error-page");
    }

    #[template_callback]
    fn row_activated_cb(&self, row: gtk::ListBoxRow) {
        let Some(user) = row
            .downcast_ref::<PillSourceRow>()
            .and_then(|r| r.source())
            .and_downcast::<User>()
        else {
            return;
        };

        // TODO: For now we show the loading page while we create the room,
        // ideally we would like to have the same behavior as Element:
        // Create the room only once the user sends a message
        let imp = self.imp();
        imp.stack.set_visible_child_name("loading-page");
        imp.search_entry.set_sensitive(false);

        spawn!(clone!(@weak self as obj => async move {
            obj.start_direct_chat(&user).await;
        }));
    }

    async fn start_direct_chat(&self, user: &User) {
        match user.get_or_create_direct_chat().await {
            Ok(room) => {
                let Some(window) = self.transient_for().and_downcast::<Window>() else {
                    return;
                };

                window.session_view().select_room(Some(room));
                self.close();
            }
            Err(_) => {
                self.show_error(&gettext("Failed to create a new Direct Chat"));
                self.imp().search_entry.set_sensitive(true);
            }
        }
    }
}
