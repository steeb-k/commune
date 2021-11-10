use gtk::{gio, glib, prelude::*, subclass::prelude::*};

use crate::session::{
    room_list::RoomList,
    sidebar::CategoryType,
    sidebar::EntryType,
    sidebar::{Category, Entry},
    verification::VerificationList,
};

mod imp {
    use once_cell::sync::Lazy;
    use once_cell::unsync::OnceCell;

    use super::*;

    #[derive(Debug, Default)]
    pub struct ItemList {
        pub list: OnceCell<[glib::Object; 7]>,
        pub room_list: OnceCell<RoomList>,
        pub verification_list: OnceCell<VerificationList>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ItemList {
        const NAME: &'static str = "ItemList";
        type Type = super::ItemList;
        type ParentType = glib::Object;
        type Interfaces = (gio::ListModel,);
    }

    impl ObjectImpl for ItemList {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpec::new_object(
                        "room-list",
                        "Room list",
                        "The list of rooms",
                        RoomList::static_type(),
                        glib::ParamFlags::READWRITE | glib::ParamFlags::CONSTRUCT_ONLY,
                    ),
                    glib::ParamSpec::new_object(
                        "verification-list",
                        "Verification list",
                        "The list of verification requests",
                        VerificationList::static_type(),
                        glib::ParamFlags::READWRITE | glib::ParamFlags::CONSTRUCT_ONLY,
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
                "room-list" => obj.set_room_list(value.get().unwrap()),
                "verification-list" => obj.set_verification_list(value.get().unwrap()),
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "room-list" => obj.room_list().to_value(),
                "verification-list" => obj.verification_list().to_value(),
                _ => unimplemented!(),
            }
        }

        fn constructed(&self, obj: &Self::Type) {
            self.parent_constructed(obj);

            let room_list = obj.room_list();
            let verification_list = obj.verification_list();

            let list = [
                Entry::new(EntryType::Explore).upcast::<glib::Object>(),
                Category::new(CategoryType::VerificationRequest, verification_list)
                    .upcast::<glib::Object>(),
                Category::new(CategoryType::Invited, room_list).upcast::<glib::Object>(),
                Category::new(CategoryType::Favorite, room_list).upcast::<glib::Object>(),
                Category::new(CategoryType::Normal, room_list).upcast::<glib::Object>(),
                Category::new(CategoryType::LowPriority, room_list).upcast::<glib::Object>(),
                Category::new(CategoryType::Left, room_list).upcast::<glib::Object>(),
            ];

            self.list.set(list).unwrap();
        }
    }

    impl ListModelImpl for ItemList {
        fn item_type(&self, _list_model: &Self::Type) -> glib::Type {
            glib::Object::static_type()
        }
        fn n_items(&self, _list_model: &Self::Type) -> u32 {
            self.list.get().map(|l| l.len()).unwrap_or(0) as u32
        }
        fn item(&self, _list_model: &Self::Type, position: u32) -> Option<glib::Object> {
            self.list
                .get()
                .and_then(|l| l.get(position as usize))
                .map(glib::object::Cast::upcast_ref::<glib::Object>)
                .cloned()
        }
    }
}

glib::wrapper! {
    /// Fixed list of all subcomponents in the sidebar.
    ///
    /// ItemList implements the ListModel interface and yields the subcomponents
    /// from the sidebar, namely Entries and Categories.
    pub struct ItemList(ObjectSubclass<imp::ItemList>)
        @implements gio::ListModel;
}

impl ItemList {
    pub fn new(room_list: &RoomList, verification_list: &VerificationList) -> Self {
        glib::Object::new(&[
            ("room-list", room_list),
            ("verification-list", verification_list),
        ])
        .expect("Failed to create ItemList")
    }

    fn set_room_list(&self, room_list: RoomList) {
        let priv_ = imp::ItemList::from_instance(self);
        priv_.room_list.set(room_list).unwrap();
    }

    fn set_verification_list(&self, verification_list: VerificationList) {
        let priv_ = imp::ItemList::from_instance(self);
        priv_.verification_list.set(verification_list).unwrap();
    }

    pub fn room_list(&self) -> &RoomList {
        let priv_ = imp::ItemList::from_instance(self);
        priv_.room_list.get().unwrap()
    }

    pub fn verification_list(&self) -> &VerificationList {
        let priv_ = imp::ItemList::from_instance(self);
        priv_.verification_list.get().unwrap()
    }
}
