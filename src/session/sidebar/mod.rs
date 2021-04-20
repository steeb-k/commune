mod category;
mod category_list;
mod category_row;
mod room;
mod room_row;

use self::category::{Category, CategoryName};
use self::category_list::CategoryList;
use self::category_row::SidebarCategoryRow;
use self::room::{HighlightFlags, Room};
use self::room_row::SidebarRoomRow;

use adw;
use adw::subclass::prelude::BinImpl;
use gtk::subclass::prelude::*;
use gtk::{self, prelude::*};
use gtk::{glib, glib::clone, glib::SyncSender, CompositeTemplate};
use matrix_sdk::{identifiers::RoomId, Client};

mod imp {
    use super::*;
    use glib::subclass::InitializingObject;
    use std::cell::Cell;

    #[derive(Debug, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/sidebar.ui")]
    pub struct Sidebar {
        pub compact: Cell<bool>,
        #[template_child]
        pub headerbar: TemplateChild<adw::HeaderBar>,
        #[template_child]
        pub listview: TemplateChild<gtk::ListView>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Sidebar {
        const NAME: &'static str = "Sidebar";
        type Type = super::Sidebar;
        type ParentType = adw::Bin;

        fn new() -> Self {
            Self {
                compact: Cell::new(false),
                listview: TemplateChild::default(),
                headerbar: TemplateChild::default(),
            }
        }

        fn class_init(klass: &mut Self::Class) {
            CategoryList::static_type();
            SidebarRoomRow::static_type();
            SidebarCategoryRow::static_type();
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for Sidebar {
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
                    let compact = value
                        .get()
                        .expect("type conformity checked by `Object::set_property`");
                    self.compact.set(compact.unwrap());
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

    /// Sets up the required channel to recive async updates from the `Client`
    pub fn setup_channel(&self) -> SyncSender<RoomId> {
        let (sender, receiver) = glib::MainContext::sync_channel::<RoomId>(Default::default(), 100);

        receiver.attach(
            None,
            clone!(@weak self as obj => @default-panic, move |room_id| {
                obj.get_list_model().update(&room_id);
                glib::Continue(true)
            }),
        );
        sender
    }

    /// Loads the state from the `Store`
    pub fn load(&self, client: &Client) {
        let list = self.get_list_model();
        // TODO: Add list for user defined categories e.g. favorite
        let invited = Category::new(client.clone(), CategoryName::Invited);
        let joined = Category::new(client.clone(), CategoryName::Normal);
        let left = Category::new(client.clone(), CategoryName::Left);

        invited.append_batch(client.invited_rooms().into_iter().map(Into::into).collect());
        joined.append_batch(client.joined_rooms().into_iter().map(Into::into).collect());
        left.append_batch(client.left_rooms().into_iter().map(Into::into).collect());

        list.append_batch(&[invited, joined, left]);
    }

    fn get_list_model(&self) -> CategoryList {
        imp::Sidebar::from_instance(self)
            .listview
            .model()
            .unwrap()
            .downcast::<gtk::NoSelection>()
            .unwrap()
            .model()
            .unwrap()
            .downcast::<CategoryList>()
            .unwrap()
    }
}
