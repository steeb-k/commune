use adw::subclass::prelude::BinImpl;
use gtk::subclass::prelude::*;
use gtk::{self, prelude::*};
use gtk::{glib, CompositeTemplate};

use crate::session::sidebar::Category;

mod imp {
    use super::*;
    use glib::subclass::InitializingObject;
    use std::cell::{Cell, RefCell};

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/sidebar-category-row.ui")]
    pub struct CategoryRow {
        pub category: RefCell<Option<Category>>,
        pub expanded: Cell<bool>,
        pub binding: RefCell<Option<glib::Binding>>,
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
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for CategoryRow {
        fn properties() -> &'static [glib::ParamSpec] {
            use once_cell::sync::Lazy;
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpec::new_object(
                        "category",
                        "Category",
                        "The category of this row",
                        Category::static_type(),
                        glib::ParamFlags::READWRITE | glib::ParamFlags::EXPLICIT_NOTIFY,
                    ),
                    glib::ParamSpec::new_boolean(
                        "expanded",
                        "Expanded",
                        "The expanded state of this row",
                        true,
                        glib::ParamFlags::READWRITE | glib::ParamFlags::EXPLICIT_NOTIFY,
                    ),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(
            &self,
            obj: &Self::Type,
            _id: usize,
            value: &glib::Value,
            pspec: &glib::ParamSpec,
        ) {
            match pspec.name() {
                "category" => {
                    let category = value.get().unwrap();
                    obj.set_category(category);
                }
                "expanded" => {
                    let expanded = value.get().unwrap();
                    obj.set_expanded(expanded);
                }
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "category" => obj.category().to_value(),
                "expanded" => obj.expanded().to_value(),
                _ => unimplemented!(),
            }
        }
    }

    impl WidgetImpl for CategoryRow {}
    impl BinImpl for CategoryRow {}
}

glib::wrapper! {
    pub struct CategoryRow(ObjectSubclass<imp::CategoryRow>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl CategoryRow {
    pub fn new() -> Self {
        glib::Object::new(&[]).expect("Failed to create CategoryRow")
    }

    pub fn category(&self) -> Option<Category> {
        let priv_ = imp::CategoryRow::from_instance(&self);
        priv_.category.borrow().clone()
    }

    pub fn set_category(&self, category: Option<Category>) {
        let priv_ = imp::CategoryRow::from_instance(&self);

        if self.category() == category {
            return;
        }

        if let Some(binding) = priv_.binding.take() {
            binding.unbind();
        }

        if let Some(ref category) = category {
            let binding = category
                .bind_property("display-name", &priv_.display_name.get(), "label")
                .flags(glib::BindingFlags::SYNC_CREATE)
                .build()
                .unwrap();

            priv_.binding.replace(Some(binding));
        }

        priv_.category.replace(category);
        self.notify("category");
    }

    fn expanded(&self) -> bool {
        let priv_ = imp::CategoryRow::from_instance(&self);
        priv_.expanded.get()
    }

    fn set_expanded(&self, expanded: bool) {
        let priv_ = imp::CategoryRow::from_instance(&self);

        if self.expanded() == expanded {
            return;
        }

        if expanded {
            self.set_state_flags(gtk::StateFlags::CHECKED, false);
        } else {
            self.unset_state_flags(gtk::StateFlags::CHECKED);
        }

        priv_.expanded.set(expanded);
        self.notify("expanded");
    }
}
