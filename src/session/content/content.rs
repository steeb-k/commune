use adw::subclass::prelude::*;
use gtk::{glib, glib::clone, prelude::*, subclass::prelude::*, CompositeTemplate};

use crate::session::{
    content::ItemRow,
    room::{Room, Timeline},
};

mod imp {
    use super::*;
    use glib::subclass::InitializingObject;
    use std::cell::Cell;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/content.ui")]
    pub struct Content {
        pub compact: Cell<bool>,
        #[template_child]
        pub headerbar: TemplateChild<adw::HeaderBar>,
        #[template_child]
        pub listview: TemplateChild<gtk::ListView>,
        #[template_child]
        pub scrolled_window: TemplateChild<gtk::ScrolledWindow>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Content {
        const NAME: &'static str = "Content";
        type Type = super::Content;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            ItemRow::static_type();
            Self::bind_template(klass);
            klass.set_accessible_role(gtk::AccessibleRole::Group);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for Content {
        fn properties() -> &'static [glib::ParamSpec] {
            use once_cell::sync::Lazy;
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![glib::ParamSpec::new_boolean(
                    "compact",
                    "Compact",
                    "Wheter a compact view is used or not",
                    false,
                    glib::ParamFlags::READWRITE,
                )]
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
                "compact" => {
                    let compact = value.get().unwrap();
                    self.compact.set(compact);
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

        fn constructed(&self, obj: &Self::Type) {
            let adj = self.scrolled_window.vadjustment().unwrap();
            // TODO: make sure that we have enough messages to fill at least to scroll pages, if the room history is long enough

            adj.connect_value_changed(clone!(@weak obj => move |adj| {
                // Load more message when the user gets close to the end of the known room history
                // Use the page size twice to detect if the user gets close the end
                if adj.value() < adj.page_size() * 2.0 {
                    if let Some(room) = obj.room() {
                        room.load_previous_events();
                        }
                }
            }));

            self.parent_constructed(obj);
        }
    }

    impl WidgetImpl for Content {}
    impl BinImpl for Content {}
}

glib::wrapper! {
    pub struct Content(ObjectSubclass<imp::Content>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl Content {
    pub fn new() -> Self {
        glib::Object::new(&[]).expect("Failed to create Content")
    }

    pub fn set_room(&self, room: &Room) {
        let priv_ = imp::Content::from_instance(self);
        // TODO: use gtk::MultiSelection to allow selection
        priv_
            .listview
            .set_model(Some(&gtk::NoSelection::new(Some(room.timeline()))));
    }

    fn room(&self) -> Option<Room> {
        let priv_ = imp::Content::from_instance(self);
        priv_
            .listview
            .model()
            .and_then(|model| model.downcast::<gtk::NoSelection>().ok())
            .and_then(|model| model.model())
            .and_then(|model| model.downcast::<Timeline>().ok())
            .map(|timeline| timeline.room().to_owned())
    }
}
