use adw::subclass::prelude::BinImpl;
use gettextrs::gettext;
use gtk::{glib, prelude::*, subclass::prelude::*, CompositeTemplate};

use crate::session::model::{Category, CategoryType};

mod imp {
    use std::{
        cell::{Cell, RefCell},
        marker::PhantomData,
    };

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session/view/sidebar/category_row.ui")]
    #[properties(wrapper_type = super::CategoryRow)]
    pub struct CategoryRow {
        /// The category of this row.
        #[property(get, set = Self::set_category, explicit_notify, nullable)]
        pub category: RefCell<Option<Category>>,
        /// The expanded state of this row.
        #[property(get, set = Self::set_expanded, explicit_notify, construct, default = true)]
        pub expanded: Cell<bool>,
        /// The label to show for this row.
        #[property(get = Self::label)]
        pub label: PhantomData<Option<String>>,
        /// The `CategoryType` to show a label for during a drag-and-drop
        /// operation.
        ///
        /// This will change the label according to the action that can be
        /// performed when changing from the `CategoryType` to this
        /// row's `Category`.
        #[property(get, set = Self::set_show_label_for_category, explicit_notify, builder(CategoryType::default()))]
        pub show_label_for_category: Cell<CategoryType>,
        /// The label showing the category name.
        #[template_child]
        pub display_name: TemplateChild<gtk::Label>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CategoryRow {
        const NAME: &'static str = "SidebarCategoryRow";
        type Type = super::CategoryRow;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            klass.set_css_name("category");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for CategoryRow {}

    impl WidgetImpl for CategoryRow {}
    impl BinImpl for CategoryRow {}

    impl CategoryRow {
        /// Set the category represented by this row.
        fn set_category(&self, category: Option<Category>) {
            if *self.category.borrow() == category {
                return;
            }

            self.category.replace(category);

            let obj = self.obj();
            obj.notify_category();
            obj.notify_label();
        }

        /// The label to show for this row.
        fn label(&self) -> Option<String> {
            let to_type = self.category.borrow().as_ref()?.r#type();
            let from_type = self.show_label_for_category.get();

            let label = match from_type {
                CategoryType::Invited => match to_type {
                    // Translators: This is an action to join a room and put it in the "Favorites"
                    // section.
                    CategoryType::Favorite => gettext("Join Room as Favorite"),
                    CategoryType::Normal => gettext("Join Room"),
                    // Translators: This is an action to join a room and put it in the "Low
                    // Priority" section.
                    CategoryType::LowPriority => gettext("Join Room as Low Priority"),
                    CategoryType::Left => gettext("Reject Invite"),
                    _ => to_type.to_string(),
                },
                CategoryType::Favorite => match to_type {
                    CategoryType::Normal => gettext("Move to Rooms"),
                    CategoryType::LowPriority => gettext("Move to Low Priority"),
                    CategoryType::Left => gettext("Leave Room"),
                    _ => to_type.to_string(),
                },
                CategoryType::Normal => match to_type {
                    CategoryType::Favorite => gettext("Move to Favorites"),
                    CategoryType::LowPriority => gettext("Move to Low Priority"),
                    CategoryType::Left => gettext("Leave Room"),
                    _ => to_type.to_string(),
                },
                CategoryType::LowPriority => match to_type {
                    CategoryType::Favorite => gettext("Move to Favorites"),
                    CategoryType::Normal => gettext("Move to Rooms"),
                    CategoryType::Left => gettext("Leave Room"),
                    _ => to_type.to_string(),
                },
                CategoryType::Left => match to_type {
                    // Translators: This is an action to rejoin a room and put it in the "Favorites"
                    // section.
                    CategoryType::Favorite => gettext("Rejoin Room as Favorite"),
                    CategoryType::Normal => gettext("Rejoin Room"),
                    // Translators: This is an action to rejoin a room and put it in the "Low
                    // Priority" section.
                    CategoryType::LowPriority => gettext("Rejoin Room as Low Priority"),
                    _ => to_type.to_string(),
                },
                _ => to_type.to_string(),
            };

            Some(label)
        }

        /// Set the expanded state of this row.
        fn set_expanded(&self, expanded: bool) {
            if self.expanded.get() == expanded {
                return;
            }
            let obj = self.obj();

            if expanded {
                obj.set_state_flags(gtk::StateFlags::CHECKED, false);
            } else {
                obj.unset_state_flags(gtk::StateFlags::CHECKED);
            }

            self.expanded.set(expanded);
            obj.set_expanded_accessibility_state(expanded);
            obj.notify_expanded();
        }

        /// Set the `CategoryType` to show a label for.
        fn set_show_label_for_category(&self, category: CategoryType) {
            if category == self.show_label_for_category.get() {
                return;
            }

            self.show_label_for_category.set(category);

            let obj = self.obj();
            obj.notify_show_label_for_category();
            obj.notify_label();
        }
    }
}

glib::wrapper! {
    /// A sidebar row representing a category.
    pub struct CategoryRow(ObjectSubclass<imp::CategoryRow>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl CategoryRow {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Set the expanded state of this row for a11y.
    fn set_expanded_accessibility_state(&self, expanded: bool) {
        if let Some(row) = self.parent() {
            row.update_state(&[gtk::accessible::State::Expanded(Some(expanded))]);
        }
    }

    /// The descendant that labels this row for a11y.
    pub fn labelled_by(&self) -> &gtk::Accessible {
        self.imp().display_name.upcast_ref()
    }
}
