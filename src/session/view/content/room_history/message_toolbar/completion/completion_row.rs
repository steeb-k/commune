use gtk::{glib, prelude::*, subclass::prelude::*, CompositeTemplate};

use crate::{
    components::Avatar,
    prelude::*,
    session::model::{Member, Room},
};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_history/message_toolbar/completion/completion_row.ui"
    )]
    #[properties(wrapper_type = super::CompletionRow)]
    pub struct CompletionRow {
        #[template_child]
        pub avatar: TemplateChild<Avatar>,
        #[template_child]
        pub display_name: TemplateChild<gtk::Label>,
        #[template_child]
        pub id: TemplateChild<gtk::Label>,
        /// The room member presented by this row.
        #[property(get, set = Self::set_member, explicit_notify, nullable)]
        pub member: RefCell<Option<Member>>,
        /// The room presented by this row.
        #[property(get, set = Self::set_room, explicit_notify, nullable)]
        pub room: RefCell<Option<Room>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CompletionRow {
        const NAME: &'static str = "ContentCompletionRow";
        type Type = super::CompletionRow;
        type ParentType = gtk::ListBoxRow;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for CompletionRow {}

    impl WidgetImpl for CompletionRow {}
    impl ListBoxRowImpl for CompletionRow {}

    impl CompletionRow {
        /// Set the room member displayed by this row.
        fn set_member(&self, member: Option<Member>) {
            if *self.member.borrow() == member {
                return;
            }

            if let Some(member) = &member {
                self.avatar.set_data(Some(member.avatar_data()));
                self.display_name.set_label(&member.display_name());
                self.id.set_label(member.user_id().as_str());
            }

            self.member.replace(member);
            self.room.replace(None);

            let obj = self.obj();
            obj.notify_member();
            obj.notify_room();
        }

        /// Set the room displayed by this row.
        fn set_room(&self, room: Option<Room>) {
            if *self.room.borrow() == room {
                return;
            }

            if let Some(room) = &room {
                self.avatar.set_data(Some(room.avatar_data()));
                self.display_name.set_label(&room.display_name());
                self.id.set_label(
                    room.alias()
                        .as_ref()
                        .map(|a| a.as_str())
                        .unwrap_or_else(|| room.room_id().as_str()),
                );
            }

            self.room.replace(room);
            self.member.replace(None);

            let obj = self.obj();
            obj.notify_member();
            obj.notify_room();
        }
    }
}

glib::wrapper! {
    /// A popover to allow completion for a given text buffer.
    pub struct CompletionRow(ObjectSubclass<imp::CompletionRow>)
        @extends gtk::Widget, gtk::ListBoxRow;
}

impl CompletionRow {
    pub fn new() -> Self {
        glib::Object::new()
    }
}

impl Default for CompletionRow {
    fn default() -> Self {
        Self::new()
    }
}
