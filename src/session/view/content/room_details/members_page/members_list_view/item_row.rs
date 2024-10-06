use adw::{prelude::BinExt, subclass::prelude::*};
use gtk::{glib, glib::prelude::*};

use super::MembershipSubpageRow;
use crate::{
    components::LoadingRow,
    session::{
        model::Member,
        view::content::room_details::{MemberRow, MembershipSubpageItem},
    },
};

mod imp {
    use std::cell::RefCell;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::ItemRow)]
    pub struct ItemRow {
        /// The item represented by this row.
        ///
        /// It can be a `Member` or a `MemberSubpageItem`.
        #[property(get, set = Self::set_item, explicit_notify, nullable)]
        item: RefCell<Option<glib::Object>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ItemRow {
        const NAME: &'static str = "ContentMemberItemRow";
        type Type = super::ItemRow;
        type ParentType = adw::Bin;
    }

    #[glib::derived_properties]
    impl ObjectImpl for ItemRow {}

    impl WidgetImpl for ItemRow {}
    impl BinImpl for ItemRow {}

    impl ItemRow {
        /// Set the item represented by this row.
        ///
        /// It must be a `Member` or a `MemberSubpageItem`.
        fn set_item(&self, item: Option<glib::Object>) {
            if *self.item.borrow() == item {
                return;
            }
            let obj = self.obj();

            if let Some(item) = &item {
                if let Some(member) = item.downcast_ref::<Member>() {
                    let child = if let Some(child) = obj.child().and_downcast::<MemberRow>() {
                        child
                    } else {
                        let child = MemberRow::new(true);
                        obj.set_child(Some(&child));
                        child
                    };
                    child.set_member(Some(member.clone()));
                } else if let Some(item) = item.downcast_ref::<MembershipSubpageItem>() {
                    let child =
                        if let Some(child) = obj.child().and_downcast::<MembershipSubpageRow>() {
                            child
                        } else {
                            let child = MembershipSubpageRow::new();
                            obj.set_child(Some(&child));
                            child
                        };

                    child.set_item(Some(item.clone()));
                } else if let Some(child) = item.downcast_ref::<LoadingRow>() {
                    obj.set_child(Some(child))
                } else {
                    unimplemented!("The object {item:?} doesn't have a widget implementation");
                }
            }

            self.item.replace(item);
            obj.notify_item();
        }
    }
}

glib::wrapper! {
    /// A row presenting an item in the list of room members.
    pub struct ItemRow(ObjectSubclass<imp::ItemRow>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl ItemRow {
    pub fn new() -> Self {
        glib::Object::new()
    }
}
