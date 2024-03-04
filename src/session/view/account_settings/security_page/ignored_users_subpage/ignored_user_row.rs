use gettextrs::gettext;
use gtk::{glib, glib::clone, prelude::*, subclass::prelude::*, CompositeTemplate};
use ruma::UserId;

use crate::{components::SpinnerButton, session::model::IgnoredUsers, spawn, toast};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/account_settings/security_page/ignored_users_subpage/ignored_user_row.ui"
    )]
    #[properties(wrapper_type = super::IgnoredUserRow)]
    pub struct IgnoredUserRow {
        #[template_child]
        pub stop_ignoring_button: TemplateChild<SpinnerButton>,
        /// The item containing the user ID presented by this row.
        #[property(get, set = Self::set_item, explicit_notify, nullable)]
        pub item: RefCell<Option<gtk::StringObject>>,
        /// The current list of ignored users.
        #[property(get, set, nullable)]
        pub ignored_users: RefCell<Option<IgnoredUsers>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for IgnoredUserRow {
        const NAME: &'static str = "IgnoredUserRow";
        type Type = super::IgnoredUserRow;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for IgnoredUserRow {}

    impl WidgetImpl for IgnoredUserRow {}
    impl BoxImpl for IgnoredUserRow {}

    impl IgnoredUserRow {
        /// Set the item containing the user ID presented by this row.
        fn set_item(&self, item: Option<gtk::StringObject>) {
            if *self.item.borrow() == item {
                return;
            }

            self.item.replace(item);
            self.obj().notify_item();

            // Reset the state of the button.
            self.stop_ignoring_button.set_loading(false);
        }
    }
}

glib::wrapper! {
    /// A row presenting an ignored user.
    pub struct IgnoredUserRow(ObjectSubclass<imp::IgnoredUserRow>)
        @extends gtk::Widget, gtk::Box, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl IgnoredUserRow {
    pub fn new(ignored_users: &IgnoredUsers) -> Self {
        glib::Object::builder()
            .property("ignored-users", ignored_users)
            .build()
    }

    /// Stop ignoring the user of this row.
    #[template_callback]
    fn stop_ignoring_user(&self) {
        let Some(user_id) = self
            .item()
            .map(|i| i.string())
            .and_then(|s| UserId::parse(&s).ok())
        else {
            return;
        };
        let Some(ignored_users) = self.ignored_users() else {
            return;
        };

        self.imp().stop_ignoring_button.set_loading(true);

        spawn!(
            clone!(@weak self as obj, @weak ignored_users => async move {
                if ignored_users.remove(&user_id).await.is_err() {
                    toast!(obj, gettext("Could not stop ignoring user"));
                    obj.imp().stop_ignoring_button.set_loading(false);
                }
            })
        );
    }
}
