use adw::subclass::prelude::*;
use gtk::{glib, glib::clone, prelude::*, CompositeTemplate};

use super::reaction::MessageReaction;
use crate::session::model::{MemberList, ReactionList};

mod imp {
    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_history/message_row/reaction_list.ui"
    )]
    pub struct MessageReactionList {
        #[template_child]
        pub flow_box: TemplateChild<gtk::FlowBox>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MessageReactionList {
        const NAME: &'static str = "ContentMessageReactionList";
        type Type = super::MessageReactionList;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            klass.set_css_name("message-reactions");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for MessageReactionList {}

    impl WidgetImpl for MessageReactionList {}

    impl BinImpl for MessageReactionList {}
}

glib::wrapper! {
    /// A widget displaying the reactions of a message.
    pub struct MessageReactionList(ObjectSubclass<imp::MessageReactionList>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl MessageReactionList {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn set_reaction_list(&self, members: &MemberList, reaction_list: &ReactionList) {
        self.imp().flow_box.bind_model(
            Some(reaction_list),
            clone!(
                @weak members => @default-return { gtk::FlowBoxChild::new().upcast() },
                move |obj| {
                    MessageReaction::new(members, obj.clone().downcast().unwrap()).upcast()
                }
            ),
        );
    }
}
