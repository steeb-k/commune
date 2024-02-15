use adw::{prelude::*, subclass::prelude::*};
use gtk::{
    gio,
    glib::{self, clone},
    CompositeTemplate,
};

use crate::{
    components::UserProfileDialog,
    session::view::content::room_history::member_timestamp::{
        row::MemberTimestampRow, MemberTimestamp,
    },
};

mod imp {
    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_history/message_row/reaction/reaction_popover.ui"
    )]
    #[properties(wrapper_type = super::ReactionPopover)]
    pub struct ReactionPopover {
        #[template_child]
        pub list: TemplateChild<gtk::ListView>,
        /// The reaction senders to display.
        #[property(get, set = Self::set_senders, construct_only)]
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

    #[glib::derived_properties]
    impl ObjectImpl for ReactionPopover {}

    impl WidgetImpl for ReactionPopover {}
    impl PopoverImpl for ReactionPopover {}

    impl ReactionPopover {
        /// Set the reaction senders to display.
        fn set_senders(&self, senders: gio::ListStore) {
            let obj = self.obj();

            self.senders.set(Some(&senders));
            self.list
                .set_model(Some(&gtk::NoSelection::new(Some(senders))));
            self.list
                .connect_activate(clone!(@weak obj => move |_, pos| {
                    let Some(member) = obj.senders()
                        .and_then(|list| list.item(pos))
                        .and_downcast::<MemberTimestamp>()
                        .and_then(|ts| ts.member())
                        else { return; };

                    let dialog = UserProfileDialog::new();
                    dialog.set_room_member(member);
                    dialog.present(&obj);
                    obj.popdown();
                }));
        }
    }
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
}
