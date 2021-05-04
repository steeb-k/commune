use adw::subclass::prelude::BinImpl;
use gtk::{gio, glib, prelude::*, subclass::prelude::*, CompositeTemplate};

use crate::session::{
    categories::{Categories, Category},
    room::Room,
    sidebar::{RoomRow, Row, Selection},
};

mod imp {
    use super::*;
    use glib::subclass::InitializingObject;
    use once_cell::sync::Lazy;
    use std::cell::{Cell, RefCell};

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/sidebar.ui")]
    pub struct Sidebar {
        pub compact: Cell<bool>,
        pub selected_room: RefCell<Option<Room>>,
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
                    glib::ParamSpec::new_object(
                        "selected-room",
                        "Selected Room",
                        "The selected room in this sidebar",
                        Room::static_type(),
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
                "compact" => {
                    let compact = value.get().unwrap();
                    self.compact.set(compact);
                }
                "categories" => {
                    let categories = value.get().unwrap();
                    obj.set_categories(categories);
                }
                "selected-room" => {
                    let selected_room = value.get().unwrap();
                    obj.set_selected_room(selected_room);
                }
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "compact" => self.compact.get().to_value(),
                "selected-room" => obj.selected_room().to_value(),
                _ => unimplemented!(),
            }
        }

        fn constructed(&self, obj: &Self::Type) {
            self.parent_constructed(obj);

            self.listview.get().connect_activate(move |listview, pos| {
                if let Some(model) = listview
                    .model()
                    .and_then(|m| m.downcast::<Selection>().ok())
                {
                    if let Some(row) = model
                        .item(pos)
                        .and_then(|o| o.downcast::<gtk::TreeListRow>().ok())
                    {
                        if row
                            .item()
                            .and_then(|o| o.downcast::<Category>().ok())
                            .is_some()
                        {
                            row.set_expanded(!row.is_expanded());
                        } else if row.item().and_then(|o| o.downcast::<Room>().ok()).is_some() {
                            model.set_selected(pos);
                        }
                    }
                }
            });
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

    pub fn selected_room(&self) -> Option<Room> {
        let priv_ = imp::Sidebar::from_instance(self);
        priv_.selected_room.borrow().clone()
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

            let selection = Selection::new(Some(&filter_model));
            self.bind_property("selected-room", &selection, "selected-room")
                .flags(glib::BindingFlags::SYNC_CREATE | glib::BindingFlags::BIDIRECTIONAL)
                .build();

            priv_.listview.set_model(Some(&selection));
        } else {
            priv_.listview.set_model(gtk::NONE_SELECTION_MODEL);
        }
    }

    fn set_selected_room(&self, selected_room: Option<Room>) {
        let priv_ = imp::Sidebar::from_instance(self);

        if self.selected_room() == selected_room {
            return;
        }

        priv_.selected_room.replace(selected_room);

        self.notify("selected-room");
    }
}
