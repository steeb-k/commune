use crate::components::Avatar;
use adw::subclass::prelude::*;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{glib, CompositeTemplate};

use crate::prelude::*;
use crate::session::{Room, User};

mod imp {
    use super::*;
    use glib::subclass::InitializingObject;
    use std::cell::RefCell;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/pill.ui")]
    pub struct Pill {
        /// The user displayed by this widget
        pub user: RefCell<Option<User>>,
        /// The room displayed by this widget
        pub room: RefCell<Option<Room>>,
        #[template_child]
        pub display_name: TemplateChild<gtk::Label>,
        #[template_child]
        pub avatar: TemplateChild<Avatar>,
        pub bindings: RefCell<Vec<glib::Binding>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Pill {
        const NAME: &'static str = "Pill";
        type Type = super::Pill;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for Pill {
        fn properties() -> &'static [glib::ParamSpec] {
            use once_cell::sync::Lazy;
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpec::new_object(
                        "user",
                        "User",
                        "The user displayed by this widget",
                        User::static_type(),
                        glib::ParamFlags::READWRITE | glib::ParamFlags::EXPLICIT_NOTIFY,
                    ),
                    glib::ParamSpec::new_object(
                        "room",
                        "Room",
                        "The room displayed by this widget",
                        Room::static_type(),
                        glib::ParamFlags::READWRITE | glib::ParamFlags::EXPLICIT_NOTIFY,
                    ),
                ]
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
                "user" => obj.set_user(value.get().unwrap()),
                "room" => obj.set_room(value.get().unwrap()),
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "user" => obj.user().to_value(),
                "room" => obj.room().to_value(),
                _ => unimplemented!(),
            }
        }
    }

    impl WidgetImpl for Pill {}

    impl BinImpl for Pill {}
}

glib::wrapper! {
    /// Inline widget displaying an emphasized `User` or `Room`.
    pub struct Pill(ObjectSubclass<imp::Pill>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl Pill {
    pub fn new() -> Self {
        glib::Object::new(&[]).expect("Failed to create Pill")
    }

    pub fn set_user(&self, user: Option<User>) {
        let priv_ = imp::Pill::from_instance(self);

        if *priv_.user.borrow() == user {
            return;
        }

        while let Some(binding) = priv_.bindings.borrow_mut().pop() {
            binding.unbind();
        }

        if let Some(ref user) = user {
            let display_name_binding = user
                .bind_property("display-name", &*priv_.display_name, "label")
                .flags(glib::BindingFlags::SYNC_CREATE)
                .build()
                .unwrap();

            priv_.bindings.borrow_mut().push(display_name_binding);
        }

        priv_
            .avatar
            .set_item(user.clone().map(|user| user.avatar().clone()));
        priv_.user.replace(user);

        self.notify("user");
    }

    pub fn user(&self) -> Option<User> {
        let priv_ = imp::Pill::from_instance(self);
        priv_.user.borrow().clone()
    }

    pub fn set_room(&self, room: Option<Room>) {
        let priv_ = imp::Pill::from_instance(self);

        if *priv_.room.borrow() == room {
            return;
        }

        while let Some(binding) = priv_.bindings.borrow_mut().pop() {
            binding.unbind();
        }

        if let Some(ref room) = room {
            let display_name_binding = room
                .bind_property("display-name", &*priv_.display_name, "label")
                .flags(glib::BindingFlags::SYNC_CREATE)
                .build()
                .unwrap();

            priv_.bindings.borrow_mut().push(display_name_binding);
        }

        priv_
            .avatar
            .set_item(room.clone().map(|room| room.avatar().clone()));
        priv_.room.replace(room);

        self.notify("room");
    }

    pub fn room(&self) -> Option<Room> {
        let priv_ = imp::Pill::from_instance(self);
        priv_.room.borrow().clone()
    }
}

impl Default for Pill {
    fn default() -> Self {
        Self::new()
    }
}
