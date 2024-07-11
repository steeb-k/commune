use gtk::{glib, glib::clone, prelude::*, subclass::prelude::*};

use super::{item_list::SidebarItemList, selection::Selection};
use crate::{
    session::model::{IdentityVerification, Room},
    utils::{expression, BoundObjectWeakRef},
};

mod imp {
    use std::{
        cell::{Cell, OnceCell},
        marker::PhantomData,
    };

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::SidebarListModel)]
    pub struct SidebarListModel {
        /// The list of items in the sidebar.
        #[property(get, set = Self::set_item_list, construct_only)]
        pub item_list: OnceCell<SidebarItemList>,
        /// The string filter.
        #[property(get)]
        pub string_filter: gtk::StringFilter,
        /// Whether the string filter is active.
        #[property(get)]
        pub is_filtered: Cell<bool>,
        /// The selection model.
        #[property(get = Self::selection_model)]
        pub selection_model: PhantomData<Selection>,
        /// The unfiltered selection model.
        pub unfiltered_selection_model: Selection,
        /// The filtered selection model.
        pub filtered_selection_model: Selection,
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

            // Keep the selected item in sync.
            self.unfiltered_selection_model
                .bind_property(
                    "selected-item",
                    &self.filtered_selection_model,
                    "selected-item",
                )
                .bidirectional()
                .sync_create()
                .build();

            // When a verification is replaced, select the replacement automatically.
            self.unfiltered_selection_model
                .connect_selected_item_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |selection_model| {
                        imp.selected_item.disconnect_signals();

                        if let Some(item) = &selection_model.selected_item() {
                            if let Some(verification) = item.downcast_ref::<IdentityVerification>()
                            {
                                let verification_handler = verification.connect_replaced(clone!(
                                    #[weak]
                                    selection_model,
                                    move |_, new_verification| {
                                        selection_model
                                            .set_selected_item(Some(new_verification.clone()));
                                    }
                                ));
                                imp.selected_item.set(item, vec![verification_handler]);
                            }
                        }
                    }
                ));

            // Switch between the filtered and unfiltered list models.
            self.string_filter.connect_search_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |string_filter| {
                    imp.set_is_filtered(string_filter.search().filter(|s| !s.is_empty()).is_some());
                }
            ));
        }
    }

    impl SidebarListModel {
        /// Set the list of items in the sidebar.
        fn set_item_list(&self, item_list: SidebarItemList) {
            self.item_list.set(item_list.clone()).unwrap();

            let unfiltered_tree_model =
                gtk::TreeListModel::new(item_list.clone(), false, true, |item| {
                    item.clone().downcast().ok()
                });
            self.unfiltered_selection_model
                .set_model(Some(unfiltered_tree_model));

            // We keep two separate models, so the filtered list is always expanded and
            // searches in all rooms.
            let filtered_tree_model = gtk::TreeListModel::new(item_list, false, true, |item| {
                item.clone().downcast().ok()
            });

            let room_name_expression =
                gtk::TreeListRow::this_expression("item").chain_property::<Room>("display-name");
            self.string_filter
                .set_match_mode(gtk::StringFilterMatchMode::Substring);
            self.string_filter
                .set_expression(Some(expression::normalize_string(room_name_expression)));
            self.string_filter.set_ignore_case(true);
            // Default to an empty string to be able to bind to GtkEditable::text.
            self.string_filter.set_search(Some(""));

            let filter_model = gtk::FilterListModel::new(
                Some(filtered_tree_model),
                Some(self.string_filter.clone()),
            );

            self.filtered_selection_model.set_model(Some(filter_model));
        }

        /// Set whether the string filter is active.
        fn set_is_filtered(&self, is_filtered: bool) {
            if self.is_filtered.get() == is_filtered {
                return;
            }

            self.is_filtered.set(is_filtered);

            let obj = self.obj();
            obj.notify_is_filtered();
            obj.notify_selection_model();
        }

        /// The selection model.
        fn selection_model(&self) -> Selection {
            if self.is_filtered.get() {
                self.filtered_selection_model.clone()
            } else {
                self.unfiltered_selection_model.clone()
            }
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
    pub fn new(item_list: &SidebarItemList) -> Self {
        glib::Object::builder()
            .property("item-list", item_list)
            .build()
    }
}
