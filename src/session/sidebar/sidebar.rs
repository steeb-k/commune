use adw::subclass::prelude::BinImpl;
use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*, CompositeTemplate};

use crate::session::{
    categories::Categories,
    room::Room,
    sidebar::{RoomRow, Row},
};

mod imp {
    use super::*;
    use glib::subclass::InitializingObject;
    use once_cell::sync::Lazy;
    use std::cell::Cell;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/sidebar.ui")]
    pub struct Sidebar {
        pub compact: Cell<bool>,
        #[template_child]
        pub headerbar: TemplateChild<adw::HeaderBar>,
        #[template_child]
        pub listview: TemplateChild<gtk::ListView>,
        #[template_child]
        pub room_search_entry: TemplateChild<gtk::SearchEntry>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Sidebar {
        const NAME: &'static str = "Sidebar";
        type Type = super::Sidebar;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            RoomRow::static_type();
            Row::static_type();
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for Sidebar {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpec::new_boolean(
                        "compact",
                        "Compact",
                        "Wheter a compact view is used or not",
                        false,
                        glib::ParamFlags::READWRITE,
                    ),
                    glib::ParamSpec::new_object(
                        "categories",
                        "Categories",
                        "A list of rooms grouped into categories",
                        Categories::static_type(),
                        glib::ParamFlags::WRITABLE,
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
                "compact" => {
                    let compact = value.get().unwrap();
                    self.compact.set(compact);
                }
                "categories" => {
                    let categories = value.get().unwrap();
                    obj.set_categories(categories);
                }
                _ => unimplemented!(),
            }
        }

        fn property(&self, _obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "compact" => self.compact.get().to_value(),
                _ => unimplemented!(),
            }
        }
    }

    impl WidgetImpl for Sidebar {}
    impl BinImpl for Sidebar {}
}

glib::wrapper! {
    pub struct Sidebar(ObjectSubclass<imp::Sidebar>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl Sidebar {
    pub fn new() -> Self {
        glib::Object::new(&[]).expect("Failed to create Sidebar")
    }

    pub fn set_categories(&self, categories: Option<Categories>) {
        let priv_ = imp::Sidebar::from_instance(self);

        if let Some(categories) = categories {
            // TODO: hide empty categories
            let tree_model = gtk::TreeListModel::new(&categories, false, true, |item| {
                item.clone().downcast::<gio::ListModel>().ok()
            });

            let room_expression = gtk::ClosureExpression::new(
                String::static_type(),
                |value| {
                    Some(
                        value[0]
                            .get::<gtk::TreeListRow>()
                            .unwrap()
                            .item()
                            .and_then(|o| o.downcast::<Room>().ok())
                            .map_or(String::new(), |o| o.display_name())
                            .to_value(),
                    )
                },
                &[],
            );
            let filter = gtk::StringFilterBuilder::new()
                .match_mode(gtk::StringFilterMatchMode::Substring)
                .expression(&room_expression)
                .ignore_case(true)
                .build();
            let filter_model = gtk::FilterListModel::new(Some(&tree_model), Some(&filter));
            priv_
                .room_search_entry
                .bind_property("text", &filter, "search")
                .flags(glib::BindingFlags::SYNC_CREATE)
                .build();

            let selection = gtk::SingleSelection::new(Some(&filter_model));
            selection.connect_notify_local(Some("selected-item"), clone!(@weak self as obj => move |model, _| {
                if let Some(room) = model.selected_item().and_then(|row| row.downcast_ref::<gtk::TreeListRow>().unwrap().item()).and_then(|o| o.downcast::<Room>().ok()) {
                        obj.activate_action("session.show-room", Some(&room.matrix_room().room_id().as_str().to_variant()));
                }
            }));

            priv_.listview.set_model(Some(&selection));
        } else {
            priv_.listview.set_model(gtk::NONE_SELECTION_MODEL);
        }
    }
}
