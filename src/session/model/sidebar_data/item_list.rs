use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};

use super::{Category, CategoryType, IconItem, ItemType, SidebarItem, SidebarItemExt};
use crate::session::model::{RoomList, VerificationList};

mod imp {
    use std::cell::{Cell, OnceCell};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::ItemList)]
    pub struct ItemList {
        pub list: OnceCell<[SidebarItem; 8]>,
        /// The list of rooms.
        #[property(get, construct_only)]
        pub room_list: OnceCell<RoomList>,
        /// The list of verification requests.
        #[property(get, construct_only)]
        pub verification_list: OnceCell<VerificationList>,
        /// The `CategoryType` to show all compatible categories for.
        ///
        /// The UI is updated to show possible actions for the list items
        /// according to the `CategoryType`.
        #[property(get, set = Self::set_show_all_for_category, explicit_notify, builder(CategoryType::default()))]
        pub show_all_for_category: Cell<CategoryType>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ItemList {
        const NAME: &'static str = "ItemList";
        type Type = super::ItemList;
        type Interfaces = (gio::ListModel,);
    }

    #[glib::derived_properties]
    impl ObjectImpl for ItemList {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            let room_list = obj.room_list();
            let verification_list = obj.verification_list();

            let list: [SidebarItem; 8] = [
                IconItem::new(ItemType::Explore).upcast(),
                Category::new(CategoryType::VerificationRequest, &verification_list).upcast(),
                Category::new(CategoryType::Invited, &room_list).upcast(),
                Category::new(CategoryType::Favorite, &room_list).upcast(),
                Category::new(CategoryType::Normal, &room_list).upcast(),
                Category::new(CategoryType::LowPriority, &room_list).upcast(),
                Category::new(CategoryType::Left, &room_list).upcast(),
                IconItem::new(ItemType::Forget).upcast(),
            ];

            self.list.set(list.clone()).unwrap();

            for item in list.iter() {
                if let Some(category) = item.downcast_ref::<Category>() {
                    category.connect_notify_local(
                        Some("empty"),
                        clone!(@weak obj => move |category, _| {
                            obj.update_item(category);
                        }),
                    );
                }
                obj.update_item(item);
            }
        }
    }

    impl ListModelImpl for ItemList {
        fn item_type(&self) -> glib::Type {
            SidebarItem::static_type()
        }

        fn n_items(&self) -> u32 {
            self.list
                .get()
                .unwrap()
                .iter()
                .filter(|item| item.visible())
                .count() as u32
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            self.list
                .get()
                .unwrap()
                .iter()
                .filter(|item| item.visible())
                .nth(position as usize)
                .cloned()
                .map(|item| item.upcast())
        }
    }

    impl ItemList {
        /// Set the `CategoryType` to show all compatible categories for.
        fn set_show_all_for_category(&self, category: CategoryType) {
            if category == self.show_all_for_category.get() {
                return;
            }
            let obj = self.obj();

            self.show_all_for_category.set(category);
            for item in self.list.get().unwrap().iter() {
                obj.update_item(item)
            }

            obj.notify_show_all_for_category();
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
        glib::Object::builder()
            .property("room-list", room_list)
            .property("verification-list", verification_list)
            .build()
    }

    fn update_item(&self, item: &impl IsA<SidebarItem>) {
        let imp = self.imp();
        let item = item.upcast_ref::<SidebarItem>();

        let old_visible = item.visible();
        let old_pos = imp
            .list
            .get()
            .unwrap()
            .iter()
            .position(|obj| item == obj)
            .unwrap();

        item.update_visibility(self.show_all_for_category());

        let visible = item.visible();

        if visible != old_visible {
            let hidden_before_position = imp
                .list
                .get()
                .unwrap()
                .iter()
                .take(old_pos)
                .filter(|item| !item.visible())
                .count();
            let real_position = old_pos - hidden_before_position;

            let (removed, added) = if visible { (0, 1) } else { (1, 0) };
            self.items_changed(real_position as u32, removed, added);
        }
    }
}
