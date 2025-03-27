use adw::{prelude::*, subclass::prelude::*};
use futures_channel::oneshot;
use gtk::{glib, glib::clone, CompositeTemplate};
use tracing::error;

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
    Window,
};

mod imp {
    use std::cell::RefCell;

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
        pub sender: RefCell<Option<oneshot::Sender<Option<User>>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CreateDmDialog {
        const NAME: &'static str = "CreateDmDialog";
        type Type = super::CreateDmDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            PillSourceRow::ensure_type();

            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for CreateDmDialog {}

    impl WidgetImpl for CreateDmDialog {}

    impl AdwDialogImpl for CreateDmDialog {
        fn closed(&self) {
            if let Some(sender) = self.sender.take() {
                if sender.send(None).is_err() {
                    error!("Could not send selected session");
                }
            }
        }
    }

    impl CreateDmDialog {
        /// Set the current session.
        pub(super) fn set_session(&self, session: Option<&Session>) {
            if self.session.upgrade().as_ref() == session {
                return;
            }
            let obj = self.obj();

            if let Some(session) = session {
                let user_list = DmUserList::new(session);

                // We don't need to disconnect this signal since the `DmUserList` will be
                // disposed once unbound from the `gtk::ListBox`
                user_list.connect_state_notify(clone!(
                    #[weak]
                    obj,
                    move |model| {
                        obj.update_view(model);
                    }
                ));

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

            self.session.set(session);
            obj.notify_session();
        }
    }
}

glib::wrapper! {
    /// Dialog to create a new direct chat.
    pub struct CreateDmDialog(ObjectSubclass<imp::CreateDmDialog>)
        @extends gtk::Widget, adw::Dialog, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl CreateDmDialog {
    pub fn new(session: &Session) -> Self {
        glib::Object::builder().property("session", session).build()
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
    fn row_activated_cb(&self, row: &gtk::ListBoxRow) {
        let Some(user) = row
            .downcast_ref::<PillSourceRow>()
            .and_then(PillSourceRow::source)
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

        if let Some(sender) = imp.sender.take() {
            if sender.send(Some(user)).is_err() {
                error!("Could not send selected session");
            }
        }
    }

    /// Select a user to start a direct chat with.
    pub async fn start_direct_chat(&self, parent: &impl IsA<gtk::Widget>) {
        let (sender, receiver) = oneshot::channel();
        self.imp().sender.replace(Some(sender));

        self.present(Some(parent));

        let Ok(Some(user)) = receiver.await else {
            return;
        };

        if let Ok(room) = user.get_or_create_direct_chat().await {
            let Some(window) = parent.root().and_downcast::<Window>() else {
                return;
            };

            window.session_view().select_room(room);
            self.close();
        } else {
            self.show_error(&gettext("Could not create a new Direct Chat"));
            self.imp().search_entry.set_sensitive(true);
        }
    }
}
