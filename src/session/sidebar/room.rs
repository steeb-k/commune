use gtk::subclass::prelude::*;
use gtk::{self, prelude::*};
use gtk::{gio, glib};
use gtk_macros::spawn;
use matrix_sdk::room::Room as MatrixRoom;

#[glib::gflags("SidebarHighlightFlags")]
pub enum HighlightFlags {
    #[glib::gflags(name = "NONE")]
    NONE = 0b00000000,
    #[glib::gflags(name = "HIGHLIGHT")]
    HIGHLIGHT = 0b00000001,
    #[glib::gflags(name = "BOLD")]
    BOLD = 0b00000010,
    #[glib::gflags(skip)]
    HIGHLIGHT_BOLD = Self::HIGHLIGHT.bits() | Self::BOLD.bits(),
}

impl Default for HighlightFlags {
    fn default() -> Self {
        HighlightFlags::NONE
    }
}

mod imp {
    use super::*;
    use once_cell::sync::OnceCell;
    use std::cell::RefCell;

    #[derive(Debug)]
    pub struct Room {
        pub room: OnceCell<MatrixRoom>,
        pub name: RefCell<Option<String>>,
        pub avatar: RefCell<Option<gio::LoadableIcon>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Room {
        const NAME: &'static str = "SidebarRoom";
        type Type = super::Room;
        type ParentType = glib::Object;

        fn new() -> Self {
            Self {
                room: OnceCell::new(),
                name: RefCell::new(Some("Unknown".to_string())),
                avatar: RefCell::new(None),
            }
        }
    }

    impl ObjectImpl for Room {
        fn properties() -> &'static [glib::ParamSpec] {
            use once_cell::sync::Lazy;
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpec::new_boxed(
                        "room",
                        "Room",
                        "The matrix room",
                        BoxedRoom::static_type(),
                        glib::ParamFlags::WRITABLE | glib::ParamFlags::CONSTRUCT_ONLY,
                    ),
                    glib::ParamSpec::new_string(
                        "display-name",
                        "Display Name",
                        "The display name of this room",
                        None,
                        glib::ParamFlags::READABLE | glib::ParamFlags::EXPLICIT_NOTIFY,
                    ),
                    glib::ParamSpec::new_object(
                        "avatar",
                        "Avatar",
                        "The url of the avatar of this room",
                        gio::LoadableIcon::static_type(),
                        glib::ParamFlags::READABLE | glib::ParamFlags::EXPLICIT_NOTIFY,
                    ),
                    glib::ParamSpec::new_flags(
                        "highlight",
                        "Highlight",
                        "How this room is highlighted",
                        HighlightFlags::static_type(),
                        HighlightFlags::default().bits(),
                        glib::ParamFlags::READABLE | glib::ParamFlags::EXPLICIT_NOTIFY,
                    ),
                    glib::ParamSpec::new_uint64(
                        "notification-count",
                        "Notification count",
                        "The notification count of this room",
                        std::u64::MIN,
                        std::u64::MAX,
                        0,
                        glib::ParamFlags::READABLE | glib::ParamFlags::EXPLICIT_NOTIFY,
                    ),
                ]
                // TODO: add other needed properties e.g. is_direct, and category
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
                "room" => {
                    let room = value
                        .get::<BoxedRoom>()
                        .expect("type conformity checked by `Object::set_property`");
                    let _ = self.room.set(room.0);
                    obj.update();
                }
                _ => unimplemented!(),
            }
        }

        fn property(&self, _obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            let room = self.room.get().unwrap();
            match pspec.name() {
                "display-name" => self.name.borrow().to_value(),
                "avatar" => self.avatar.borrow().to_value(),
                "highlight" => {
                    let count = room.unread_notification_counts().highlight_count;

                    // TODO: how do we know when to set the row to be bold
                    if count > 0 {
                        HighlightFlags::HIGHLIGHT
                    } else {
                        HighlightFlags::NONE
                    }
                    .to_value()
                }
                "notification-count" => {
                    let highlight = room.unread_notification_counts().highlight_count;
                    let notification = room.unread_notification_counts().notification_count;

                    if highlight > 0 {
                        highlight
                    } else {
                        notification
                    }
                    .to_value()
                }
                _ => unimplemented!(),
            }
        }
    }
}

glib::wrapper! {
    pub struct Room(ObjectSubclass<imp::Room>);
}

#[derive(Clone, Debug, glib::GBoxed)]
#[gboxed(type_name = "BoxedRoom")]
struct BoxedRoom(MatrixRoom);

impl Room {
    pub fn new(room: &MatrixRoom) -> Self {
        glib::Object::new(&[("room", &BoxedRoom(room.clone()))]).expect("Failed to create Room")
    }

    /// This should be called when any field on the Room has changed
    pub fn update(&self) {
        self.load_display_name();
        self.load_avatar();
        self.notify("highlight");
        self.notify("notification-count");
    }

    fn load_display_name(&self) {
        let obj = self.downgrade();
        spawn!(async move {
            if let Some(obj) = obj.upgrade() {
                let priv_ = imp::Room::from_instance(&obj);
                let name = &priv_.name;
                let new_name = priv_.room.get().unwrap().display_name().await.ok();

                if *name.borrow() != new_name {
                    name.replace(new_name);
                    obj.notify("display-name");
                }
            }
        });
    }

    fn load_avatar(&self) {
        // TODO: load avatar and create a LoadableIcon
    }
}
