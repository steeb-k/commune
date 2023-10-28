use gtk::{gio, glib, prelude::*, subclass::prelude::*, CompositeTemplate};

use crate::session::view::content::room_history::member_timestamp::row::MemberTimestampRow;

mod imp {
    use glib::subclass::InitializingObject;
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_history/read_receipts_list/read_receipts_popover.ui"
    )]
    pub struct ReadReceiptsPopover {
        #[template_child]
        pub list: TemplateChild<gtk::ListView>,
        /// The receipts to display.
        pub receipts: glib::WeakRef<gio::ListStore>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ReadReceiptsPopover {
        const NAME: &'static str = "ContentReadReceiptsPopover";
        type Type = super::ReadReceiptsPopover;
        type ParentType = gtk::Popover;

        fn class_init(klass: &mut Self::Class) {
            MemberTimestampRow::static_type();
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for ReadReceiptsPopover {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![glib::ParamSpecObject::builder::<gio::ListStore>("receipts")
                    .construct_only()
                    .build()]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            let obj = self.obj();

            match pspec.name() {
                "receipts" => obj.set_receipts(value.get().unwrap()),
                _ => unimplemented!(),
            }
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            let obj = self.obj();

            match pspec.name() {
                "receipts" => obj.receipts().to_value(),
                _ => unimplemented!(),
            }
        }

        fn constructed(&self) {
            self.parent_constructed();
        }
    }

    impl WidgetImpl for ReadReceiptsPopover {}
    impl PopoverImpl for ReadReceiptsPopover {}
}

glib::wrapper! {
    /// A popover to display the read receipts on an event.
    pub struct ReadReceiptsPopover(ObjectSubclass<imp::ReadReceiptsPopover>)
        @extends gtk::Widget, gtk::Popover;
}

impl ReadReceiptsPopover {
    /// Constructs a new `ReadReceiptsPopover` with the given receipts list.
    pub fn new(receipts: &gio::ListStore) -> Self {
        glib::Object::builder()
            .property("receipts", receipts)
            .build()
    }

    /// The receipts to display.
    pub fn receipts(&self) -> Option<gio::ListStore> {
        self.imp().receipts.upgrade()
    }

    /// Set the receipts to display.
    fn set_receipts(&self, receipts: Option<gio::ListStore>) {
        let Some(receipts) = receipts else {
            // Ignore missing receipts.
            return;
        };
        let imp = self.imp();

        imp.receipts.set(Some(&receipts));
        imp.list
            .set_model(Some(&gtk::NoSelection::new(Some(receipts))));
        self.notify("receipts");
    }
}
