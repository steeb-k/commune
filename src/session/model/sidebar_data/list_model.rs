use gtk::{glib, glib::clone, prelude::*, subclass::prelude::*};

use super::{item_list::ItemList, selection::Selection};
use crate::{
    session::model::{IdentityVerification, Room},
    utils::BoundObjectWeakRef,
};

mod imp {
    use std::cell::OnceCell;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::SidebarListModel)]
    pub struct SidebarListModel {
        /// The list of items in the sidebar.
        #[property(get, set = Self::set_item_list, construct_only)]
        pub item_list: OnceCell<ItemList>,
        /// The tree list model.
        #[property(get)]
        pub tree_model: OnceCell<gtk::TreeListModel>,
        /// The string filter.
        #[property(get)]
        pub string_filter: gtk::StringFilter,
        /// The selection model.
        #[property(get)]
        pub selection_model: Selection,
        /// The selected item, if it has signal handlers.
        pub selected_item: BoundObjectWeakRef<glib::Object>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SidebarListModel {
        const NAME: &'static str = "SidebarListModel";
        type Type = super::SidebarListModel;
    }

    #[glib::derived_properties]
    impl ObjectImpl for SidebarListModel {
        fn constructed(&self) {
            self.parent_constructed();

            self.selection_model.connect_selected_item_notify(
                clone!(@weak self as imp => move |selection_model| {
                    imp.selected_item.disconnect_signals();

                    if let Some(item) = &selection_model.selected_item() {
                        if let Some(verification) = item.downcast_ref::<IdentityVerification>() {
                            let verification_handler = verification.connect_replaced(
                                clone!(@weak selection_model => move |_, new_verification| {
                                    selection_model.set_selected_item(Some(new_verification.clone()));
                                }),
                            );
                            imp.selected_item.set(item, vec![verification_handler]);
                        }
                    }
                }),
            );
        }
    }

    impl SidebarListModel {
        /// Set the list of items in the sidebar.
        fn set_item_list(&self, item_list: ItemList) {
            self.item_list.set(item_list.clone()).unwrap();

            let tree_model = gtk::TreeListModel::new(item_list, false, true, |item| {
                item.clone().downcast().ok()
            });
            self.tree_model.set(tree_model.clone()).unwrap();

            let room_expression =
                gtk::TreeListRow::this_expression("item").chain_property::<Room>("display-name");
            self.string_filter
                .set_match_mode(gtk::StringFilterMatchMode::Substring);
            self.string_filter.set_expression(Some(&room_expression));
            self.string_filter.set_ignore_case(true);
            // Default to an empty string to be able to bind to GtkEditable::text.
            self.string_filter.set_search(Some(""));

            let filter_model =
                gtk::FilterListModel::new(Some(tree_model), Some(self.string_filter.clone()));

            self.selection_model.set_model(Some(filter_model));
        }
    }
}

glib::wrapper! {
    /// A wrapper for the sidebar list model of a `Session`.
    ///
    /// It allows to keep the state for selection and filtering.
    pub struct SidebarListModel(ObjectSubclass<imp::SidebarListModel>);
}

impl SidebarListModel {
    /// Create a new `SidebarListModel`.
    pub fn new(item_list: &ItemList) -> Self {
        glib::Object::builder()
            .property("item-list", item_list)
            .build()
    }
}
