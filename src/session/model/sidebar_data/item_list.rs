use std::cell::Cell;

use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};

use super::{Category, CategoryType, SidebarIconItem, SidebarIconItemType, SidebarItem};
use crate::session::model::{RoomList, VerificationList};

/// The number of top-level items in the sidebar.
const TOP_LEVEL_ITEMS_COUNT: usize = 8;

mod imp {
    use std::cell::OnceCell;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::SidebarItemList)]
    pub struct SidebarItemList {
        /// The list of top-level items.
        list: OnceCell<[SidebarItem; TOP_LEVEL_ITEMS_COUNT]>,
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
    impl ObjectSubclass for SidebarItemList {
        const NAME: &'static str = "SidebarItemList";
        type Type = super::SidebarItemList;
        type Interfaces = (gio::ListModel,);
    }

    #[glib::derived_properties]
    impl ObjectImpl for SidebarItemList {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            let room_list = obj.room_list();
            let verification_list = obj.verification_list();

            let list = self.list.get_or_init(|| {
                [
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
                ]
            });

            for item in list {
                if let Some(category) = item.inner_item().downcast_ref::<Category>() {
                    category.connect_empty_notify(clone!(
                        #[weak(rename_to = imp)]
                        self,
                        #[weak]
                        item,
                        move |_| {
                            imp.update_item_visibility(&item);
                        }
                    ));
                }
                self.update_item_visibility(item);
            }
        }
    }

    impl ListModelImpl for SidebarItemList {
        fn item_type(&self) -> glib::Type {
            SidebarItem::static_type()
        }

        fn n_items(&self) -> u32 {
            TOP_LEVEL_ITEMS_COUNT as u32
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            self.list().get(position as usize).cloned().and_upcast()
        }
    }

    impl SidebarItemList {
        /// The list of top-level items.
        fn list(&self) -> &[SidebarItem; TOP_LEVEL_ITEMS_COUNT] {
            self.list.get().unwrap()
        }

        /// Set the `CategoryType` to show all compatible categories for.
        fn set_show_all_for_category(&self, category: CategoryType) {
            if category == self.show_all_for_category.get() {
                return;
            }

            self.show_all_for_category.set(category);
            for item in self.list() {
                self.update_item_visibility(item);
            }

            self.obj().notify_show_all_for_category();
        }

        /// Update the visibility of the given item.
        fn update_item_visibility(&self, item: &SidebarItem) {
            item.update_visibility_for_category(self.show_all_for_category.get());
        }

        /// Set whether to inhibit the expanded state of the categories.
        ///
        /// It means that all the categories will be expanded regardless of
        /// their "is-expanded" property.
        pub(super) fn inhibit_expanded(&self, inhibit: bool) {
            for item in self.list() {
                item.set_inhibit_expanded(inhibit);
            }
        }
    }
}

glib::wrapper! {
    /// Fixed list of all subcomponents in the sidebar.
    ///
    /// Implements the `gio::ListModel` interface and yields the top-level
    /// items of the sidebar.
    pub struct SidebarItemList(ObjectSubclass<imp::SidebarItemList>)
        @implements gio::ListModel;
}

impl SidebarItemList {
    /// Construct a new `SidebarItemList` with the given room list and
    /// verification list.
    pub fn new(room_list: &RoomList, verification_list: &VerificationList) -> Self {
        glib::Object::builder()
            .property("room-list", room_list)
            .property("verification-list", verification_list)
            .build()
    }

    /// Set whether to inhibit the expanded state of the categories.
    ///
    /// It means that all the categories will be expanded regardless of their
    /// "is-expanded" property.
    pub fn inhibit_expanded(&self, inhibit: bool) {
        self.imp().inhibit_expanded(inhibit);
    }
}
