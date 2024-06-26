use std::cell::Cell;

use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};

use super::{Category, CategoryType, SidebarIconItem, SidebarIconItemType};
use crate::session::model::{RoomList, VerificationList};

/// A top-level sidebar item.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SidebarItem {
    /// The item.
    item: glib::Object,
    /// Whether the item is visible.
    visible: Cell<bool>,
}

impl SidebarItem {
    fn new(item: impl IsA<glib::Object>) -> Self {
        Self {
            item: item.upcast(),
            visible: Cell::new(true),
        }
    }

    /// Whether this item is visible.
    fn visible(&self) -> bool {
        self.visible.get()
    }

    /// Update the visibility of this item for a drag-n-drop from the given
    /// category.
    fn update_visibility(&self, for_category: CategoryType) {
        let visible = if let Some(category) = self.item.downcast_ref::<Category>() {
            category.visible_for_category(for_category)
        } else if let Some(icon_item) = self.item.downcast_ref::<SidebarIconItem>() {
            icon_item.visible_for_category(for_category)
        } else {
            true
        };

        self.visible.set(visible);
    }
}

mod imp {
    use std::cell::OnceCell;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::ItemList)]
    pub struct ItemList {
        /// The list of top-level items.
        ///
        /// This is a list of `(item, visible)` tuples.
        list: OnceCell<[SidebarItem; 8]>,
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

            let list = [
                SidebarItem::new(SidebarIconItem::new(SidebarIconItemType::Explore)),
                SidebarItem::new(Category::new(
                    CategoryType::VerificationRequest,
                    &verification_list,
                )),
                SidebarItem::new(Category::new(CategoryType::Invited, &room_list)),
                SidebarItem::new(Category::new(CategoryType::Favorite, &room_list)),
                SidebarItem::new(Category::new(CategoryType::Normal, &room_list)),
                SidebarItem::new(Category::new(CategoryType::LowPriority, &room_list)),
                SidebarItem::new(Category::new(CategoryType::Left, &room_list)),
                SidebarItem::new(SidebarIconItem::new(SidebarIconItemType::Forget)),
            ];

            self.list.set(list.clone()).unwrap();

            for (pos, item) in list.iter().enumerate() {
                if let Some(category) = item.item.downcast_ref::<Category>() {
                    category.connect_empty_notify(clone!(
                        #[weak(rename_to = imp)]
                        self,
                        move |_| {
                            imp.update_item_at(pos);
                        }
                    ));
                }
                self.update_item_at(pos);
            }
        }
    }

    impl ListModelImpl for ItemList {
        fn item_type(&self) -> glib::Type {
            glib::Object::static_type()
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
                .map(|item| item.item.clone())
        }
    }

    impl ItemList {
        /// Set the `CategoryType` to show all compatible categories for.
        fn set_show_all_for_category(&self, category: CategoryType) {
            if category == self.show_all_for_category.get() {
                return;
            }

            self.show_all_for_category.set(category);
            for pos in 0..self.list.get().unwrap().len() {
                self.update_item_at(pos);
            }

            self.obj().notify_show_all_for_category();
        }

        /// Update the visibility of the item at the given absolute position.
        fn update_item_at(&self, abs_pos: usize) {
            let list = self.list.get().unwrap();
            let item = &list[abs_pos];
            let old_visible = item.visible();

            item.update_visibility(self.show_all_for_category.get());
            let visible = item.visible();

            if visible != old_visible {
                // Compute the position in the gio::ListModel.
                let hidden_before_position = list
                    .iter()
                    .take(abs_pos)
                    .filter(|item| !item.visible())
                    .count();

                let real_position = abs_pos - hidden_before_position;

                // If its not added, it's removed.
                let (removed, added) = if visible { (0, 1) } else { (1, 0) };
                self.obj()
                    .items_changed(real_position as u32, removed, added);
            }
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
}
