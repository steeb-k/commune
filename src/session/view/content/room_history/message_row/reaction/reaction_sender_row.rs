use adw::{prelude::*, subclass::prelude::*};
use gtk::{glib, CompositeTemplate};

use super::member_reaction_sender::MemberReactionSender;

mod imp {
    use glib::subclass::InitializingObject;
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_history/message_row/reaction/reaction_sender_row.ui"
    )]
    pub struct ReactionSenderRow {
        /// The sender presented by this row.
        pub sender: glib::WeakRef<MemberReactionSender>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ReactionSenderRow {
        const NAME: &'static str = "ContentMessageReactionSenderRow";
        type Type = super::ReactionSenderRow;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for ReactionSenderRow {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpecObject::builder::<MemberReactionSender>("sender")
                        .explicit_notify()
                        .build(),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            match pspec.name() {
                "sender" => self.obj().set_sender(
                    value
                        .get::<Option<MemberReactionSender>>()
                        .unwrap()
                        .as_ref(),
                ),
                _ => unimplemented!(),
            }
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "sender" => self.obj().sender().to_value(),
                _ => unimplemented!(),
            }
        }
    }

    impl WidgetImpl for ReactionSenderRow {}

    impl BinImpl for ReactionSenderRow {}
}

glib::wrapper! {
    /// A row displaying a reaction sender.
    pub struct ReactionSenderRow(ObjectSubclass<imp::ReactionSenderRow>)
        @extends gtk::Widget, adw::Bin;
}

impl ReactionSenderRow {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// The reaction sender presented by this row.
    pub fn sender(&self) -> Option<MemberReactionSender> {
        self.imp().sender.upgrade()
    }

    /// Set the reaction sender presented by this row.
    pub fn set_sender(&self, sender: Option<&MemberReactionSender>) {
        if self.sender().as_ref() == sender {
            return;
        }

        self.imp().sender.set(sender);
        self.notify("sender");
    }
}

impl Default for ReactionSenderRow {
    fn default() -> Self {
        Self::new()
    }
}
