use gtk::{glib, prelude::*, subclass::prelude::*, CompositeTemplate};

use super::DmUser;

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session/view/create_dm_dialog/dm_user_row.ui")]
    #[properties(wrapper_type = super::DmUserRow)]
    pub struct DmUserRow {
        /// The user displayed by this row.
        #[property(get, set = Self::set_user, explicit_notify)]
        pub user: RefCell<Option<DmUser>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DmUserRow {
        const NAME: &'static str = "CreateDmDialogUserRow";
        type Type = super::DmUserRow;
        type ParentType = gtk::ListBoxRow;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for DmUserRow {}

    impl WidgetImpl for DmUserRow {}
    impl ListBoxRowImpl for DmUserRow {}

    impl DmUserRow {
        /// Set the user displayed by this row.
        fn set_user(&self, user: Option<DmUser>) {
            if *self.user.borrow() == user {
                return;
            }

            self.user.replace(user);
            self.obj().notify_user();
        }
    }
}

glib::wrapper! {
    /// A row of the DM user list.
    pub struct DmUserRow(ObjectSubclass<imp::DmUserRow>)
        @extends gtk::Widget, gtk::ListBoxRow, @implements gtk::Accessible;
}

impl DmUserRow {
    pub fn new(user: &DmUser) -> Self {
        glib::Object::builder().property("user", user).build()
    }
}
