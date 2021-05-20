use adw::subclass::prelude::*;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{glib, CompositeTemplate};

use crate::session::User;

mod imp {
    use super::*;
    use glib::subclass::InitializingObject;
    use std::cell::RefCell;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/user-pill.ui")]
    pub struct UserPill {
        /// The user displayed by this widget
        pub user: RefCell<Option<User>>,
        #[template_child]
        pub display_name: TemplateChild<gtk::Label>,
        #[template_child]
        pub avatar: TemplateChild<adw::Avatar>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for UserPill {
        const NAME: &'static str = "UserPill";
        type Type = super::UserPill;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for UserPill {
        fn properties() -> &'static [glib::ParamSpec] {
            use once_cell::sync::Lazy;
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![glib::ParamSpec::new_object(
                    "user",
                    "User",
                    "The user displayed by this widget",
                    User::static_type(),
                    glib::ParamFlags::READWRITE | glib::ParamFlags::EXPLICIT_NOTIFY,
                )]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(
            &self,
            obj: &Self::Type,
            _id: usize,
            value: &glib::Value,
            pspec: &glib::ParamSpec,
        ) {
            match pspec.name() {
                "user" => {
                    let user = value.get().unwrap();
                    obj.set_user(user);
                }
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "user" => obj.user().to_value(),
                _ => unimplemented!(),
            }
        }
    }

    impl WidgetImpl for UserPill {}

    impl BinImpl for UserPill {}
}

glib::wrapper! {
    pub struct UserPill(ObjectSubclass<imp::UserPill>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

/// A widget displaying a `User`
impl UserPill {
    pub fn new() -> Self {
        glib::Object::new(&[]).expect("Failed to create UserPill")
    }

    pub fn set_user(&self, user: Option<User>) {
        let priv_ = imp::UserPill::from_instance(self);

        if *priv_.user.borrow() == user {
            return;
        }

        priv_.user.replace(user);

        self.notify("user");
    }

    pub fn user(&self) -> Option<User> {
        let priv_ = imp::UserPill::from_instance(self);
        priv_.user.borrow().clone()
    }
}
