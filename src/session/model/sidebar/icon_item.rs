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
    use std::cell::Cell;

    use super::*;

    #[derive(Debug, Default)]
    pub struct IconItem {
        pub type_: Cell<ItemType>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for IconItem {
        const NAME: &'static str = "IconItem";
        type Type = super::IconItem;
        type ParentType = SidebarItem;
    }

    impl ObjectImpl for IconItem {
        fn properties() -> &'static [glib::ParamSpec] {
            use once_cell::sync::Lazy;
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpecEnum::builder::<ItemType>("type")
                        .construct_only()
                        .build(),
                    glib::ParamSpecString::builder("display-name")
                        .read_only()
                        .build(),
                    glib::ParamSpecString::builder("icon-name")
                        .read_only()
                        .build(),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            match pspec.name() {
                "type" => {
                    self.type_.set(value.get().unwrap());
                }
                _ => unimplemented!(),
            }
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            let obj = self.obj();

            match pspec.name() {
                "type" => obj.type_().to_value(),
                "display-name" => obj.display_name().to_value(),
                "icon-name" => obj.icon_name().to_value(),
                _ => unimplemented!(),
            }
        }
    }

    impl SidebarItemImpl for IconItem {
        fn update_visibility(&self, for_category: CategoryType) {
            let obj = self.obj();

            match obj.type_() {
                ItemType::Explore => obj.set_visible(true),
                ItemType::Forget => obj.set_visible(for_category == CategoryType::Left),
            }
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

    /// The type of this item.
    pub fn type_(&self) -> ItemType {
        self.imp().type_.get()
    }

    /// The display name of this item.
    pub fn display_name(&self) -> String {
        self.type_().to_string()
    }

    /// The icon name used for this item.
    pub fn icon_name(&self) -> &'static str {
        self.type_().icon_name()
    }
}
