use gtk::{glib, prelude::*, subclass::prelude::*, CompositeTemplate};

use crate::{
    components::Avatar,
    prelude::*,
    session::model::{AvatarData, Member},
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
                self.avatar.set_data(Some(member.avatar_data().to_owned()));
                self.display_name.set_label(&member.display_name());
                self.id.set_label(member.user_id().as_str());
            } else {
                self.avatar.set_data(None::<AvatarData>);
                self.display_name.set_label("");
                self.id.set_label("");
            }

            self.member.replace(member);
            self.obj().notify_member();
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
