use gtk::{glib, prelude::*, subclass::prelude::*};

use super::CategoryType;

mod imp {
    use std::cell::{Cell, RefCell};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::CategoryFilter)]
    pub struct CategoryFilter {
        /// The expression to watch.
        #[property(get, set = Self::set_expression, explicit_notify, nullable)]
        pub expression: RefCell<Option<gtk::Expression>>,
        /// The category type to filter.
        #[property(get, set = Self::set_category_type, explicit_notify, builder(CategoryType::default()))]
        pub category_type: Cell<CategoryType>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CategoryFilter {
        const NAME: &'static str = "CategoryFilter";
        type Type = super::CategoryFilter;
        type ParentType = gtk::Filter;
    }

    #[glib::derived_properties]
    impl ObjectImpl for CategoryFilter {}

    impl FilterImpl for CategoryFilter {
        fn strictness(&self) -> gtk::FilterMatch {
            if self.category_type.get() == CategoryType::None {
                return gtk::FilterMatch::All;
            }

            if self.expression.borrow().is_none() {
                return gtk::FilterMatch::None;
            }

            gtk::FilterMatch::Some
        }

        fn match_(&self, item: &glib::Object) -> bool {
            let category_type = self.category_type.get();
            if category_type == CategoryType::None {
                return true;
            }

            let Some(value) = self
                .expression
                .borrow()
                .as_ref()
                .and_then(|e| e.evaluate(Some(item)))
                .map(|v| v.get::<CategoryType>().unwrap())
            else {
                return false;
            };

            value == category_type
        }
    }

    impl CategoryFilter {
        /// Set the expression to watch.
        ///
        /// This expression must return a [`CategoryType`].
        fn set_expression(&self, expression: Option<gtk::Expression>) {
            let prev_expression = self.expression.borrow().clone();

            if prev_expression.is_none() && expression.is_none() {
                return;
            }
            let obj = self.obj();

            let change = if self.category_type.get() == CategoryType::None {
                None
            } else if prev_expression.is_none() {
                Some(gtk::FilterChange::LessStrict)
            } else if expression.is_none() {
                Some(gtk::FilterChange::MoreStrict)
            } else {
                Some(gtk::FilterChange::Different)
            };

            self.expression.replace(expression);
            if let Some(change) = change {
                obj.changed(change)
            }
            obj.notify_expression();
        }

        /// Set the category type to filter.
        fn set_category_type(&self, category_type: CategoryType) {
            let prev_category_type = self.category_type.get();

            if prev_category_type == category_type {
                return;
            }
            let obj = self.obj();

            let change = if self.expression.borrow().is_none() {
                None
            } else if prev_category_type == CategoryType::None {
                Some(gtk::FilterChange::MoreStrict)
            } else if category_type == CategoryType::None {
                Some(gtk::FilterChange::LessStrict)
            } else {
                Some(gtk::FilterChange::Different)
            };

            self.category_type.set(category_type);
            if let Some(change) = change {
                obj.changed(change)
            }
            obj.notify_category_type();
        }
    }
}

glib::wrapper! {
    /// A filter by `CategoryType`.
    pub struct CategoryFilter(ObjectSubclass<imp::CategoryFilter>)
        @extends gtk::Filter;
}

impl CategoryFilter {
    pub fn new() -> Self {
        glib::Object::new()
    }
}

impl Default for CategoryFilter {
    fn default() -> Self {
        Self::new()
    }
}
