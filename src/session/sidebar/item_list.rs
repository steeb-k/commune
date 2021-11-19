use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};

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
    use std::cell::Cell;

    use super::*;

    #[derive(Debug, Default)]
    pub struct ItemList {
        pub list: OnceCell<[(glib::Object, Cell<bool>); 7]>,
        pub room_list: OnceCell<RoomList>,
        pub verification_list: OnceCell<VerificationList>,
        pub show_all: Cell<bool>,
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
                    glib::ParamSpec::new_boolean(
                        "show-all",
                        "Show All",
                        "Whether all room categories should be shown",
                        false,
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
                "room-list" => obj.set_room_list(value.get().unwrap()),
                "verification-list" => obj.set_verification_list(value.get().unwrap()),
                "show-all" => obj.set_show_all(value.get().unwrap()),
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "room-list" => obj.room_list().to_value(),
                "verification-list" => obj.verification_list().to_value(),
                "show-all" => obj.show_all().to_value(),
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

            for (index, item) in list.iter().enumerate() {
                if let Some(category) = item.downcast_ref::<Category>() {
                    category.connect_notify_local(
                        Some("empty"),
                        clone!(@weak obj => move |_, _| {
                            obj.update_category(index);
                        }),
                    );
                }
            }

            let list = list.map(|item| {
                let visible = if let Some(category) = item.downcast_ref::<Category>() {
                    !category.is_empty()
                } else {
                    true
                };
                (item, Cell::new(visible))
            });

            self.list.set(list).unwrap();
        }
    }

    impl ListModelImpl for ItemList {
        fn item_type(&self, _list_model: &Self::Type) -> glib::Type {
            glib::Object::static_type()
        }
        fn n_items(&self, _list_model: &Self::Type) -> u32 {
            self.list
                .get()
                .unwrap()
                .iter()
                .filter(|(_, visible)| visible.get())
                .count() as u32
        }
        fn item(&self, _list_model: &Self::Type, position: u32) -> Option<glib::Object> {
            self.list
                .get()
                .unwrap()
                .iter()
                .filter_map(
                    |(item, visible)| {
                        if visible.get() {
                            Some(item)
                        } else {
                            None
                        }
                    },
                )
                .nth(position as usize)
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

    fn update_category(&self, position: usize) {
        let priv_ = imp::ItemList::from_instance(self);
        let (item, old_visible) = priv_.list.get().unwrap().get(position).unwrap();
        let category = item.downcast_ref::<Category>().unwrap();

        let visible = !category.is_empty() || (self.show_all() && is_show_all_category(category));
        if visible != old_visible.get() {
            old_visible.set(visible);
            let hidden_before_position = priv_
                .list
                .get()
                .unwrap()
                .iter()
                .take(position)
                .filter(|(_, visible)| !visible.get())
                .count();
            let real_position = position - hidden_before_position;

            let (removed, added) = if visible { (0, 1) } else { (1, 0) };

            self.items_changed(real_position as u32, removed, added);
        }
    }

    // Whether all room categories are shown
    // This doesn't include `CategoryType::Invite` since the user can't move rooms to it.
    pub fn show_all(&self) -> bool {
        let priv_ = imp::ItemList::from_instance(self);
        priv_.show_all.get()
    }

    // Set whether all room categories should be shown
    // This doesn't include `CategoryType::Invite` since the user can't move rooms to it.
    pub fn set_show_all(&self, show_all: bool) {
        let priv_ = imp::ItemList::from_instance(self);
        if show_all == self.show_all() {
            return;
        }

        priv_.show_all.set(show_all);

        for (index, (item, _)) in priv_.list.get().unwrap().iter().enumerate() {
            if let Some(category) = item.downcast_ref::<Category>() {
                if is_show_all_category(category) {
                    self.update_category(index);
                }
            }
        }

        self.notify("show-all");
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

// Wheter this category should be shown when `show-all` is `true`
// This doesn't include `CategoryType::Invite` since the user can't move rooms to it.
fn is_show_all_category(category: &Category) -> bool {
    matches!(
        category.type_(),
        CategoryType::Favorite
            | CategoryType::Normal
            | CategoryType::LowPriority
            | CategoryType::Left
    )
}
