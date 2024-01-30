use gtk::{
    gio, glib,
    glib::{clone, closure},
    prelude::*,
    subclass::prelude::*,
};

mod category_filter;
mod category_type;

use self::category_filter::CategoryFilter;
pub use self::category_type::CategoryType;
use crate::{
    session::model::{Room, RoomList, RoomType},
    utils::ExpressionListModel,
};

mod imp {
    use std::{
        cell::{Cell, OnceCell},
        marker::PhantomData,
    };

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::Category)]
    pub struct Category {
        /// The filter list model of this category.
        #[property(get, set = Self::set_model, construct_only)]
        pub model: OnceCell<gio::ListModel>,
        /// The filter of this category.
        pub filter: CategoryFilter,
        /// The type of this category.
        #[property(get = Self::category_type, set = Self::set_category_type, construct_only, builder(CategoryType::default()))]
        pub category_type: PhantomData<CategoryType>,
        /// Whether this category is empty.
        #[property(get)]
        pub empty: Cell<bool>,
        /// The display name of this category.
        #[property(get = Self::display_name)]
        pub display_name: PhantomData<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Category {
        const NAME: &'static str = "Category";
        type Type = super::Category;
        type Interfaces = (gio::ListModel,);
    }

    #[glib::derived_properties]
    impl ObjectImpl for Category {}

    impl ListModelImpl for Category {
        fn item_type(&self) -> glib::Type {
            glib::Object::static_type()
        }

        fn n_items(&self) -> u32 {
            self.model.get().unwrap().n_items()
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            self.model.get().unwrap().item(position)
        }
    }

    impl Category {
        /// Set the filter list model of this category.
        fn set_model(&self, model: gio::ListModel) {
            let obj = self.obj();

            // Special case room lists so that they are sorted and in the right category
            let model = if model.is::<RoomList>() {
                let room_category_type = Room::this_expression("category")
                    .chain_closure::<CategoryType>(closure!(
                        |_: Option<glib::Object>, room_type: RoomType| {
                            CategoryType::from(room_type)
                        }
                    ));
                self.filter
                    .set_expression(Some(room_category_type.clone().upcast()));

                let category_type_expr_model = ExpressionListModel::new();
                category_type_expr_model.set_expressions(vec![room_category_type.upcast()]);
                category_type_expr_model.set_model(Some(model));

                let filter_model = gtk::FilterListModel::new(
                    Some(category_type_expr_model),
                    Some(self.filter.clone()),
                );

                let room_latest_activity = Room::this_expression("latest-activity");
                let sorter = gtk::NumericSorter::builder()
                    .expression(&room_latest_activity)
                    .sort_order(gtk::SortType::Descending)
                    .build();

                let latest_activity_expr_model = ExpressionListModel::new();
                latest_activity_expr_model.set_expressions(vec![room_latest_activity.upcast()]);
                latest_activity_expr_model.set_model(Some(filter_model.upcast()));

                let sort_model =
                    gtk::SortListModel::new(Some(latest_activity_expr_model), Some(sorter));
                sort_model.upcast()
            } else {
                model
            };

            model.connect_items_changed(clone!(@weak obj => move |model, pos, removed, added| {
                obj.items_changed(pos, removed, added);
                obj.imp().set_empty(model.n_items() == 0);
            }));

            self.set_empty(model.n_items() == 0);
            self.model.set(model).unwrap();
        }

        /// The type of this category.
        fn category_type(&self) -> CategoryType {
            self.filter.category_type()
        }

        /// Set the type of this category.
        fn set_category_type(&self, type_: CategoryType) {
            self.filter.set_category_type(type_);
        }

        /// Set whether this category is empty.
        fn set_empty(&self, empty: bool) {
            if empty == self.empty.get() {
                return;
            }

            self.empty.set(empty);
            self.obj().notify_empty();
        }

        /// The display name of this category.
        fn display_name(&self) -> String {
            self.category_type().to_string()
        }
    }
}

glib::wrapper! {
    /// A list of Items in the same category, implementing ListModel.
    ///
    /// This struct is used in ItemList for the sidebar.
    pub struct Category(ObjectSubclass<imp::Category>)
        @implements gio::ListModel;
}

impl Category {
    pub fn new(category_type: CategoryType, model: &impl IsA<gio::ListModel>) -> Self {
        glib::Object::builder()
            .property("category-type", category_type)
            .property("model", model)
            .build()
    }

    /// Whether this category should be shown for a drag-n-drop from the given
    /// category.
    pub fn visible_for_category(&self, for_category: CategoryType) -> bool {
        if !self.empty() {
            return true;
        }

        let room_types = RoomType::try_from(for_category)
            .ok()
            .zip(RoomType::try_from(self.category_type()).ok());

        room_types.is_some_and(|(source_room_type, target_room_type)| {
            source_room_type.can_change_to(target_room_type)
        })
    }
}
