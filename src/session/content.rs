use adw;
use adw::subclass::prelude::BinImpl;
use gtk::subclass::prelude::*;
use gtk::{self, prelude::*};
use gtk::{glib, glib::SyncSender, CompositeTemplate};
use matrix_sdk::identifiers::RoomId;

mod imp {
    use super::*;
    use glib::subclass::InitializingObject;
    use std::cell::Cell;

    #[derive(Debug, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/content.ui")]
    pub struct FrctlContent {
        pub compact: Cell<bool>,
        #[template_child]
        pub headerbar: TemplateChild<adw::HeaderBar>,
        #[template_child]
        pub room_history: TemplateChild<gtk::ListView>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FrctlContent {
        const NAME: &'static str = "FrctlContent";
        type Type = super::FrctlContent;
        type ParentType = adw::Bin;

        fn new() -> Self {
            Self {
                compact: Cell::new(false),
                headerbar: TemplateChild::default(),
                room_history: TemplateChild::default(),
            }
        }

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for FrctlContent {
        fn properties() -> &'static [glib::ParamSpec] {
            use once_cell::sync::Lazy;
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![glib::ParamSpec::boolean(
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
            match pspec.get_name() {
                "compact" => {
                    let compact = value
                        .get()
                        .expect("type conformity checked by `Object::set_property`");
                    self.compact.set(compact.unwrap());
                }
                _ => unimplemented!(),
            }
        }

        fn get_property(
            &self,
            _obj: &Self::Type,
            _id: usize,
            pspec: &glib::ParamSpec,
        ) -> glib::Value {
            match pspec.get_name() {
                "compact" => self.compact.get().to_value(),
                _ => unimplemented!(),
            }
        }
    }

    impl WidgetImpl for FrctlContent {}
    impl BinImpl for FrctlContent {}
}

glib::wrapper! {
    pub struct FrctlContent(ObjectSubclass<imp::FrctlContent>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl FrctlContent {
    pub fn new() -> Self {
        glib::Object::new(&[]).expect("Failed to create FrctlContent")
    }

    /// Sets up the required channel to recive async updates from the `Client`
    pub fn setup_channel(&self) -> SyncSender<RoomId> {
        let (sender, receiver) = glib::MainContext::sync_channel::<RoomId>(Default::default(), 100);
        receiver.attach(None, move |_room_id| {
            //TODO: actually do something: update the message GListModel
            glib::Continue(true)
        });
        sender
    }
}
