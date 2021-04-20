use crate::session::sidebar::CategoryName;
use adw;
use adw::subclass::prelude::BinImpl;
use gtk::subclass::prelude::*;
use gtk::{self, prelude::*};
use gtk::{glib, CompositeTemplate};

mod imp {
    use super::*;
    use glib::subclass::InitializingObject;

    #[derive(Debug, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/sidebar-category-row.ui")]
    pub struct SidebarCategoryRow {
        #[template_child]
        pub display_name: TemplateChild<gtk::Label>,
        #[template_child]
        pub arrow: TemplateChild<gtk::Image>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SidebarCategoryRow {
        const NAME: &'static str = "SidebarCategoryRow";
        type Type = super::SidebarCategoryRow;
        type ParentType = adw::Bin;

        fn new() -> Self {
            Self {
                display_name: TemplateChild::default(),
                arrow: TemplateChild::default(),
            }
        }

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for SidebarCategoryRow {
        fn properties() -> &'static [glib::ParamSpec] {
            use once_cell::sync::Lazy;
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpec::new_enum(
                        "display-name",
                        "Display Name",
                        "The display name of this category",
                        CategoryName::static_type(),
                        CategoryName::default() as i32,
                        glib::ParamFlags::WRITABLE,
                    ),
                    glib::ParamSpec::new_boolean(
                        "expanded",
                        "Expanded",
                        "Wheter this category is expanded or not",
                        true,
                        glib::ParamFlags::WRITABLE,
                    ),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(
            &self,
            _obj: &Self::Type,
            _id: usize,
            value: &glib::Value,
            pspec: &glib::ParamSpec,
        ) {
            match pspec.name() {
                "display-name" => {
                    let display_name: CategoryName = value
                        .get()
                        .expect("type conformity checked by `Object::set_property`")
                        .expect("A room always needs a display name");
                    self.display_name.set_label(&display_name.to_string());
                }
                "expanded" => {
                    let expanded = value
                        .get()
                        .expect("type conformity checked by `Object::set_property`")
                        .unwrap();
                    if expanded {
                        //self.add_css_class("expanded");
                    } else {
                        //self.remove_css_class("expanded");
                    }
                }
                _ => unimplemented!(),
            }
        }

        fn constructed(&self, obj: &Self::Type) {
            self.parent_constructed(obj);
        }
    }

    impl WidgetImpl for SidebarCategoryRow {}
    impl BinImpl for SidebarCategoryRow {}
}

glib::wrapper! {
    pub struct SidebarCategoryRow(ObjectSubclass<imp::SidebarCategoryRow>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl SidebarCategoryRow {
    pub fn new() -> Self {
        glib::Object::new(&[]).expect("Failed to create SidebarCategoryRow")
    }
}
