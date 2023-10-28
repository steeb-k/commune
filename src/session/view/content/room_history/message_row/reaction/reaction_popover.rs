use adw::subclass::prelude::*;
use gtk::{gio, glib, prelude::*, CompositeTemplate};

use crate::session::view::content::room_history::member_timestamp::row::MemberTimestampRow;

mod imp {
    use glib::subclass::InitializingObject;
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_history/message_row/reaction/reaction_popover.ui"
    )]
    pub struct ReactionPopover {
        #[template_child]
        pub list: TemplateChild<gtk::ListView>,
        /// The reaction senders to display.
        pub senders: glib::WeakRef<gio::ListStore>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ReactionPopover {
        const NAME: &'static str = "ContentMessageReactionPopover";
        type Type = super::ReactionPopover;
        type ParentType = gtk::Popover;

        fn class_init(klass: &mut Self::Class) {
            MemberTimestampRow::static_type();
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for ReactionPopover {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![glib::ParamSpecObject::builder::<gio::ListStore>("senders")
                    .construct_only()
                    .build()]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            let obj = self.obj();

            match pspec.name() {
                "senders" => obj.set_senders(value.get().unwrap()),
                _ => unimplemented!(),
            }
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            let obj = self.obj();

            match pspec.name() {
                "senders" => obj.senders().to_value(),
                _ => unimplemented!(),
            }
        }

        fn constructed(&self) {
            self.parent_constructed();
        }
    }

    impl WidgetImpl for ReactionPopover {}

    impl PopoverImpl for ReactionPopover {}
}

glib::wrapper! {
    /// A popover to display the senders of a reaction.
    pub struct ReactionPopover(ObjectSubclass<imp::ReactionPopover>)
        @extends gtk::Widget, gtk::Popover;
}

impl ReactionPopover {
    /// Constructs a new `ReactionPopover` with the given reaction senders.
    pub fn new(senders: &gio::ListStore) -> Self {
        glib::Object::builder().property("senders", senders).build()
    }

    /// The reaction senders to display.
    pub fn senders(&self) -> Option<gio::ListStore> {
        self.imp().senders.upgrade()
    }

    /// Set the reaction senders to display.
    fn set_senders(&self, senders: Option<gio::ListStore>) {
        let Some(senders) = senders else {
            // Ignore missing reaction senders.
            return;
        };
        let imp = self.imp();

        imp.senders.set(Some(&senders));
        imp.list
            .set_model(Some(&gtk::NoSelection::new(Some(senders))));
        self.notify("senders");
    }
}
