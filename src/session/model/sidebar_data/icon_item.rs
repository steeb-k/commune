use std::fmt;

use gettextrs::gettext;
use gtk::{glib, prelude::*, subclass::prelude::*};

use super::{CategoryType, SidebarItem, SidebarItemExt, SidebarItemImpl};

#[derive(Debug, Default, Hash, Eq, PartialEq, Clone, Copy, glib::Enum)]
#[repr(u32)]
#[enum_type(name = "ItemType")]
pub enum ItemType {
    #[default]
    Explore = 0,
    Forget = 1,
}

impl ItemType {
    /// The icon name for this item type.
    pub fn icon_name(&self) -> &'static str {
        match self {
            Self::Explore => "explore-symbolic",
            Self::Forget => "user-trash-symbolic",
        }
    }
}

impl fmt::Display for ItemType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Explore => gettext("Explore"),
            Self::Forget => gettext("Forget Room"),
        };

        f.write_str(&label)
    }
}

mod imp {
    use std::{cell::Cell, marker::PhantomData};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::IconItem)]
    pub struct IconItem {
        /// The type of this item.
        #[property(get, construct_only, builder(ItemType::default()))]
        pub r#type: Cell<ItemType>,
        /// The display name of this item.
        #[property(get = Self::display_name)]
        pub display_name: PhantomData<String>,
        /// The icon name used for this item.
        #[property(get = Self::icon_name)]
        pub icon_name: PhantomData<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for IconItem {
        const NAME: &'static str = "IconItem";
        type Type = super::IconItem;
        type ParentType = SidebarItem;
    }

    #[glib::derived_properties]
    impl ObjectImpl for IconItem {}

    impl SidebarItemImpl for IconItem {
        fn update_visibility(&self, for_category: CategoryType) {
            let obj = self.obj();

            match self.r#type.get() {
                ItemType::Explore => obj.set_visible(true),
                ItemType::Forget => obj.set_visible(for_category == CategoryType::Left),
            }
        }
    }

    impl IconItem {
        /// The display name of this item.
        fn display_name(&self) -> String {
            self.r#type.get().to_string()
        }

        /// The icon name used for this item.
        fn icon_name(&self) -> String {
            self.r#type.get().icon_name().to_owned()
        }
    }
}

glib::wrapper! {
    /// A top-level row in the sidebar with an icon.
    pub struct IconItem(ObjectSubclass<imp::IconItem>) @extends SidebarItem;
}

impl IconItem {
    pub fn new(type_: ItemType) -> Self {
        glib::Object::builder().property("type", type_).build()
    }
}
