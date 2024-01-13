use adw::subclass::prelude::*;
use gtk::{glib, prelude::*, CompositeTemplate};

use crate::{
    components::Avatar,
    session::model::{AvatarData, RemoteRoom, Room, User},
};

/// The source of the pill's data.
#[derive(Debug, Clone)]
pub enum PillSource {
    /// A user.
    User(User),
    /// A room.
    Room(Room),
    /// A remote room.
    RemoteRoom(RemoteRoom),
}

impl PillSource {
    /// The avatar data of this source.
    pub fn avatar_data(&self) -> AvatarData {
        match self {
            PillSource::User(user) => user.avatar_data(),
            PillSource::Room(room) => room.avatar_data(),
            PillSource::RemoteRoom(room) => room.avatar_data(),
        }
    }

    /// Bind the `display-name` property of the source to the given target.
    pub fn bind_display_name<O: glib::ObjectType>(
        &self,
        target: &O,
        target_property: &str,
    ) -> glib::Binding {
        let obj: &glib::Object = match self {
            PillSource::User(user) => user.upcast_ref(),
            PillSource::Room(room) => room.upcast_ref(),
            PillSource::RemoteRoom(room) => room.upcast_ref(),
        };

        obj.bind_property("display-name", target, target_property)
            .sync_create()
            .build()
    }

    /// Bind the property matching the identifier of the source to the given
    /// target.
    pub fn bind_identifier<O: glib::ObjectType>(
        &self,
        target: &O,
        target_property: &str,
    ) -> glib::Binding {
        let (source_property, obj): (&str, &glib::Object) = match self {
            PillSource::User(user) => ("user-id-string", user.upcast_ref()),
            PillSource::Room(room) => ("identifier-string", room.upcast_ref()),
            PillSource::RemoteRoom(room) => ("identifier-string", room.upcast_ref()),
        };

        obj.bind_property(source_property, target, target_property)
            .sync_create()
            .build()
    }
}

mod imp {
    use std::{cell::RefCell, marker::PhantomData};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/pill.ui")]
    #[properties(wrapper_type = super::Pill)]
    pub struct Pill {
        #[template_child]
        pub display_name: TemplateChild<gtk::Label>,
        #[template_child]
        pub avatar: TemplateChild<Avatar>,
        /// The source of the data displayed by this widget.
        pub source: RefCell<Option<PillSource>>,
        /// The user displayed by this widget, if any.
        #[property(get = Self::user, set = Self::set_user, explicit_notify, nullable)]
        user: PhantomData<Option<User>>,
        /// The room displayed by this widget, if any.
        #[property(get = Self::room, set = Self::set_room, explicit_notify, nullable)]
        room: PhantomData<Option<Room>>,
        /// The remote room displayed by this widget, if any.
        #[property(get = Self::remote_room, set = Self::set_remote_room, explicit_notify, nullable)]
        remote_room: PhantomData<Option<RemoteRoom>>,
        pub binding: RefCell<Option<glib::Binding>>,
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
    impl ObjectImpl for Pill {
        fn dispose(&self) {
            if let Some(binding) = self.binding.take() {
                binding.unbind();
            }
        }
    }

    impl WidgetImpl for Pill {}
    impl BinImpl for Pill {}

    impl Pill {
        /// Set the source of the data displayed by this widget.
        pub(super) fn set_source(&self, source: Option<PillSource>) {
            if let Some(binding) = self.binding.take() {
                binding.unbind();
            }

            if let Some(source) = &source {
                let display_name_binding = source.bind_display_name(&*self.display_name, "label");
                self.binding.replace(Some(display_name_binding));
            }

            self.avatar
                .set_data(source.as_ref().map(|s| s.avatar_data()));
            self.source.replace(source);

            let obj = self.obj();
            obj.notify_user();
            obj.notify_room();
            obj.notify_remote_room();
        }

        /// The user displayed by this widget, if any.
        fn user(&self) -> Option<User> {
            match self.source.borrow().as_ref()? {
                PillSource::User(user) => Some(user.clone()),
                _ => None,
            }
        }

        /// Set the user displayed by this widget.
        fn set_user(&self, user: Option<User>) {
            self.set_source(user.map(PillSource::User));
        }

        /// The room displayed by this widget, if any.
        fn room(&self) -> Option<Room> {
            match self.source.borrow().as_ref()? {
                PillSource::Room(room) => Some(room.clone()),
                _ => None,
            }
        }

        /// Set the room displayed by this widget.
        fn set_room(&self, room: Option<Room>) {
            self.set_source(room.map(PillSource::Room));
        }

        /// The remote room displayed by this widget, if any.
        fn remote_room(&self) -> Option<RemoteRoom> {
            match self.source.borrow().as_ref()? {
                PillSource::RemoteRoom(room) => Some(room.clone()),
                _ => None,
            }
        }

        /// Set the remote room displayed by this widget.
        fn set_remote_room(&self, room: Option<RemoteRoom>) {
            self.set_source(room.map(PillSource::RemoteRoom));
        }
    }
}

glib::wrapper! {
    /// Inline widget displaying an emphasized `User` or `Room`.
    pub struct Pill(ObjectSubclass<imp::Pill>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl Pill {
    /// Create a pill with the given source.
    pub fn new(source: PillSource) -> Self {
        let obj = glib::Object::new::<Self>();
        obj.imp().set_source(Some(source));
        obj
    }

    /// Create a pill for the given user.
    pub fn for_user(user: impl IsA<User>) -> Self {
        glib::Object::builder()
            .property("user", user.upcast_ref())
            .build()
    }

    /// Create a pill for the given room.
    pub fn for_room(room: &Room) -> Self {
        glib::Object::builder().property("room", room).build()
    }

    /// Create a pill for the given remote room.
    pub fn for_remote_room(room: &RemoteRoom) -> Self {
        glib::Object::builder()
            .property("remote-room", room)
            .build()
    }

    /// The source of the data displayed by this widget.
    pub fn source(&self) -> Option<PillSource> {
        self.imp().source.borrow().clone()
    }
}
