use adw::{prelude::*, subclass::prelude::*};
use gtk::{glib, prelude::*, subclass::prelude::*, CompositeTemplate};
use matrix_sdk::events::{AnyStateEvent, AnyStateEventContent};

mod imp {
    use super::*;
    use glib::subclass::InitializingObject;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/content-state-row.ui")]
    pub struct StateRow {
        #[template_child]
        pub timestamp: TemplateChild<gtk::Label>,
        #[template_child]
        pub content: TemplateChild<adw::Bin>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for StateRow {
        const NAME: &'static str = "ContentStateRow";
        type Type = super::StateRow;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for StateRow {}
    impl WidgetImpl for StateRow {}
    impl BinImpl for StateRow {}
}

glib::wrapper! {
    pub struct StateRow(ObjectSubclass<imp::StateRow>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

//TODO
// - [] Implement widgets to show state events
impl StateRow {
    pub fn new() -> Self {
        glib::Object::new(&[]).expect("Failed to create StateRow")
    }

    pub fn update(&self, state: &AnyStateEvent) {
        let _priv_ = imp::StateRow::from_instance(self);
        // We may want to show more state events in the future
        // For a full list of state events see:
        // https://matrix-org.github.io/matrix-rust-sdk/matrix_sdk/events/enum.AnyStateEventContent.html
        let message = match state.content() {
            AnyStateEventContent::RoomCreate(_event) => format!("The beginning of this room."),
            AnyStateEventContent::RoomEncryption(_event) => format!("This room is now encrypted."),
            AnyStateEventContent::RoomMember(_event) => {
                // TODO: fully implement this state event
                format!("A member did change something: state, avatar, name ...")
            }
            AnyStateEventContent::RoomThirdPartyInvite(event) => {
                format!("{} was invited.", event.display_name)
            }
            AnyStateEventContent::RoomTombstone(event) => {
                format!("The room was upgraded: {}", event.body)
                // Todo: add button for new room with acction session.show_room::room_id
            }
            _ => {
                format!("Unsupported Event: this shouldn't be shown.")
            }
        };
        if let Some(Ok(child)) = self.child().map(|w| w.downcast::<gtk::Label>()) {
            child.set_text(&message);
        } else {
            let child = gtk::Label::new(Some(&message));
            self.set_child(Some(&child));
        };
    }
}
