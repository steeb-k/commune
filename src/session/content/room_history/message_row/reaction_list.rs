use adw::subclass::prelude::*;
use gtk::{glib, prelude::*, subclass::prelude::*, CompositeTemplate};

use crate::session::room::ReactionList;

use super::reaction::MessageReaction;

mod imp {
    use super::*;
    use glib::subclass::InitializingObject;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/content-message-reaction-list.ui")]
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
        glib::Object::new(&[]).expect("Failed to create MessageReactionList")
    }

    pub fn set_reaction_list(&self, reaction_list: &ReactionList) {
        let priv_ = imp::MessageReactionList::from_instance(self);

        priv_.flow_box.bind_model(Some(reaction_list), |obj| {
            MessageReaction::new(obj.clone().downcast().unwrap()).upcast()
        });
    }
}

impl Default for MessageReactionList {
    fn default() -> Self {
        Self::new()
    }
}
