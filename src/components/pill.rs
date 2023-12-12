use adw::subclass::prelude::*;
use gtk::{glib, prelude::*, CompositeTemplate};

use crate::{
    components::Avatar,
    session::model::{Room, User},
};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/pill.ui")]
    #[properties(wrapper_type = super::Pill)]
    pub struct Pill {
        /// The user displayed by this widget.
        #[property(get, set = Self::set_user, explicit_notify, nullable)]
        pub user: RefCell<Option<User>>,
        /// The room displayed by this widget.
        #[property(get, set = Self::set_room, explicit_notify, nullable)]
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

    #[glib::derived_properties]
    impl ObjectImpl for Pill {}

    impl WidgetImpl for Pill {}
    impl BinImpl for Pill {}

    impl Pill {
        /// Set the user displayed by this widget.
        ///
        /// This removes the room, if one was set.
        fn set_user(&self, user: Option<User>) {
            if *self.user.borrow() == user {
                return;
            }

            while let Some(binding) = self.bindings.borrow_mut().pop() {
                binding.unbind();
            }
            self.set_room(None);

            if let Some(user) = &user {
                let display_name_binding = user
                    .bind_property("display-name", &*self.display_name, "label")
                    .sync_create()
                    .build();

                self.bindings.borrow_mut().push(display_name_binding);
            }

            self.avatar
                .set_data(user.as_ref().map(|user| user.avatar_data().clone()));
            self.user.replace(user);

            self.obj().notify_user();
        }

        /// Set the room displayed by this widget.
        ///
        /// This removes the user, if one was set.
        pub fn set_room(&self, room: Option<Room>) {
            if *self.room.borrow() == room {
                return;
            }

            while let Some(binding) = self.bindings.borrow_mut().pop() {
                binding.unbind();
            }
            self.set_user(None);

            if let Some(room) = &room {
                let display_name_binding = room
                    .bind_property("display-name", &*self.display_name, "label")
                    .sync_create()
                    .build();

                self.bindings.borrow_mut().push(display_name_binding);
            }

            self.avatar
                .set_data(room.as_ref().map(|room| room.avatar_data().clone()));
            self.room.replace(room);

            self.obj().notify_room();
        }
    }
}

glib::wrapper! {
    /// Inline widget displaying an emphasized `User` or `Room`.
    pub struct Pill(ObjectSubclass<imp::Pill>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl Pill {
    pub fn for_user(user: &User) -> Self {
        glib::Object::builder().property("user", user).build()
    }

    pub fn for_room(room: &Room) -> Self {
        glib::Object::builder().property("room", room).build()
    }
}
