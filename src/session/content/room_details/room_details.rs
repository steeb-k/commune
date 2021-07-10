use adw::subclass::prelude::*;
use gettextrs::gettext;
use gtk::{
    glib::{self, clone},
    prelude::*,
    subclass::prelude::*,
    CompositeTemplate,
};

use crate::components::CustomEntry;
use crate::session::Room;

mod imp {
    use super::*;
    use glib::subclass::InitializingObject;
    use once_cell::unsync::OnceCell;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/content-room-details.ui")]
    pub struct RoomDetails {
        pub room: OnceCell<Room>,
        #[template_child]
        pub edit_toggle: TemplateChild<gtk::ToggleButton>,
        #[template_child]
        pub room_name_entry: TemplateChild<gtk::Entry>,
        #[template_child]
        pub room_topic_text_view: TemplateChild<gtk::TextView>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RoomDetails {
        const NAME: &'static str = "RoomDetails";
        type Type = super::RoomDetails;
        type ParentType = adw::PreferencesWindow;

        fn class_init(klass: &mut Self::Class) {
            CustomEntry::static_type();
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for RoomDetails {
        fn properties() -> &'static [glib::ParamSpec] {
            use once_cell::sync::Lazy;
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![glib::ParamSpec::new_object(
                    "room",
                    "Room",
                    "The room backing all details of the preference window",
                    Room::static_type(),
                    glib::ParamFlags::READWRITE | glib::ParamFlags::CONSTRUCT_ONLY,
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
                "room" => obj.set_room(value.get().unwrap()),
                _ => unimplemented!(),
            }
        }

        fn property(&self, _obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "room" => self.room.get().to_value(),
                _ => unimplemented!(),
            }
        }

        fn constructed(&self, obj: &Self::Type) {
            self.parent_constructed(obj);

            obj.init_edit_toggle();
        }
    }

    impl WidgetImpl for RoomDetails {}
    impl WindowImpl for RoomDetails {}
    impl AdwWindowImpl for RoomDetails {}
    impl PreferencesWindowImpl for RoomDetails {}
}

glib::wrapper! {
    /// Preference Window to display and update room details.
    pub struct RoomDetails(ObjectSubclass<imp::RoomDetails>)
        @extends gtk::Widget, gtk::Window, adw::Window, adw::PreferencesWindow, @implements gtk::Accessible;
}

impl RoomDetails {
    pub fn new(parent_window: &Option<gtk::Window>, room: &Room) -> Self {
        glib::Object::new(&[("transient-for", parent_window), ("room", room)])
            .expect("Failed to create RoomDetails")
    }

    pub fn room(&self) -> &Room {
        let priv_ = imp::RoomDetails::from_instance(self);
        // Use unwrap because room property is CONSTRUCT_ONLY.
        priv_.room.get().unwrap()
    }

    fn set_room(&self, room: Room) {
        let priv_ = imp::RoomDetails::from_instance(self);
        priv_.room.set(room).expect("Room already initialized");
    }

    fn init_edit_toggle(&self) {
        let priv_ = imp::RoomDetails::from_instance(self);
        let edit_toggle = &priv_.edit_toggle;
        let label_enabled = gettext("Save Details");
        let label_disabled = gettext("Edit Details");

        edit_toggle.set_active(false);
        edit_toggle.set_label(&label_disabled);

        // Save changes of name and topic on toggle button release.
        edit_toggle.connect_toggled(clone!(@weak self as this => move |button| {
            if button.is_active() {
                button.set_label(&label_enabled);
                return;
            }
            button.set_label(&label_disabled);

            let priv_ = imp::RoomDetails::from_instance(&this);
            let room = this.room();

            let room_name = priv_.room_name_entry.buffer().text();
            let topic_buffer = priv_.room_topic_text_view.buffer();
            let topic = topic_buffer.text(&topic_buffer.start_iter(), &topic_buffer.end_iter(), true);
            room.store_room_name(room_name);
            room.store_topic(topic.to_string());
        }));

        // End editing on enter.
        priv_
            .room_name_entry
            .connect_activate(clone!(@weak self as this => move |_entry| {
                let priv_ = imp::RoomDetails::from_instance(&this);
                priv_.edit_toggle.set_active(false);
            }));
    }
}
