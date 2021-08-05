use gtk::{gio, glib, prelude::*, subclass::prelude::*};

use crate::session::{
    content::ContentType,
    room::RoomType,
    room_list::RoomList,
    sidebar::{Category, Entry},
};

mod imp {
    use once_cell::sync::Lazy;
    use once_cell::unsync::OnceCell;

    use super::*;

    #[derive(Debug, Default)]
    pub struct ItemList {
        pub list: OnceCell<[glib::Object; 6]>,
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
                vec![glib::ParamSpec::new_object(
                    "room-list",
                    "Room list",
                    "Data model for the categories",
                    RoomList::static_type(),
                    glib::ParamFlags::WRITABLE | glib::ParamFlags::CONSTRUCT_ONLY,
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
                "room-list" => {
                    let x = value.get().unwrap();
                    obj.set_room_list(&x)
                }
                _ => unimplemented!(),
            }
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
    pub fn new(room_list: &RoomList) -> Self {
        glib::Object::new(&[("room-list", room_list)]).expect("Failed to create ItemList")
    }

    fn set_room_list(&self, room_list: &RoomList) {
        let priv_ = imp::ItemList::from_instance(self);

        let list = [
            Entry::new(ContentType::Explore).upcast::<glib::Object>(),
            Category::new(RoomType::Invited, room_list).upcast::<glib::Object>(),
            Category::new(RoomType::Favorite, room_list).upcast::<glib::Object>(),
            Category::new(RoomType::Normal, room_list).upcast::<glib::Object>(),
            Category::new(RoomType::LowPriority, room_list).upcast::<glib::Object>(),
            Category::new(RoomType::Left, room_list).upcast::<glib::Object>(),
        ];
        let len = list.len() as u32;

        priv_.list.set(list).unwrap();
        self.items_changed(0, 0, len);
    }
}
