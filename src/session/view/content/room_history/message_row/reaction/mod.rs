use adw::subclass::prelude::*;
use gtk::{gio, glib, glib::clone, prelude::*, CompositeTemplate};
use matrix_sdk_ui::timeline::ReactionSenderData as SdkReactionSenderData;

mod member_reaction_sender;
mod reaction_popover;
mod reaction_sender_row;

use self::{
    member_reaction_sender::MemberReactionSender, reaction_popover::ReactionPopover,
    reaction_sender_row::ReactionSenderRow,
};
use crate::{
    session::model::{MemberList, ReactionGroup},
    utils::{BoundObjectWeakRef, EMOJI_REGEX},
};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, CompositeTemplate)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_history/message_row/reaction/mod.ui"
    )]
    pub struct MessageReaction {
        /// The reaction senders (group) to display.
        pub group: BoundObjectWeakRef<ReactionGroup>,
        /// The list of reaction senders as room members.
        pub list: gio::ListStore,
        /// The member list of the room of the reaction.
        pub members: RefCell<Option<MemberList>>,
        #[template_child]
        pub button: TemplateChild<gtk::ToggleButton>,
        #[template_child]
        pub reaction_key: TemplateChild<gtk::Label>,
        #[template_child]
        pub reaction_count: TemplateChild<gtk::Label>,
    }

    impl Default for MessageReaction {
        fn default() -> Self {
            Self {
                group: Default::default(),
                list: gio::ListStore::new::<MemberReactionSender>(),
                members: Default::default(),
                button: Default::default(),
                reaction_key: Default::default(),
                reaction_count: Default::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MessageReaction {
        const NAME: &'static str = "ContentMessageReaction";
        type Type = super::MessageReaction;
        type ParentType = gtk::FlowBoxChild;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for MessageReaction {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpecObject::builder::<ReactionGroup>("group")
                        .construct_only()
                        .build(),
                    glib::ParamSpecObject::builder::<gio::ListStore>("list")
                        .read_only()
                        .build(),
                    glib::ParamSpecObject::builder::<MemberList>("members").build(),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            match pspec.name() {
                "group" => {
                    self.obj().set_group(value.get().unwrap());
                }
                "members" => self.obj().set_members(value.get().unwrap()),
                _ => unimplemented!(),
            }
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "group" => self.obj().group().to_value(),
                "list" => self.obj().list().to_value(),
                "members" => self.obj().members().to_value(),
                _ => unimplemented!(),
            }
        }

        fn dispose(&self) {
            self.group.disconnect_signals();
        }
    }

    impl WidgetImpl for MessageReaction {}

    impl FlowBoxChildImpl for MessageReaction {}
}

glib::wrapper! {
    /// A widget displaying the reactions of a message.
    pub struct MessageReaction(ObjectSubclass<imp::MessageReaction>)
        @extends gtk::Widget, gtk::FlowBoxChild, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl MessageReaction {
    pub fn new(members: MemberList, reaction_group: ReactionGroup) -> Self {
        glib::Object::builder()
            .property("group", reaction_group)
            .property("members", members)
            .build()
    }

    // The reaction group to display.
    pub fn group(&self) -> Option<ReactionGroup> {
        self.imp().group.obj()
    }

    /// Set the reaction group to display.
    fn set_group(&self, group: ReactionGroup) {
        let imp = self.imp();
        let key = group.key();
        imp.reaction_key.set_label(key);

        if EMOJI_REGEX.is_match(key) {
            imp.reaction_key.add_css_class("emoji");
        } else {
            imp.reaction_key.remove_css_class("emoji");
        }

        imp.button.set_action_target_value(Some(&key.to_variant()));
        group
            .bind_property("has-user", &*imp.button, "active")
            .sync_create()
            .build();
        group
            .bind_property("count", &*imp.reaction_count, "label")
            .sync_create()
            .build();

        let items_changed_handler_id = group.connect_items_changed(
            clone!(@weak self as obj => move |group, pos, removed, added|
                   obj.items_changed(group, pos, removed, added)
            ),
        );
        self.items_changed(&group, 0, self.list().n_items(), group.n_items());

        imp.group.set(&group, vec![items_changed_handler_id]);
    }

    /// The list of reaction senders as room members.
    pub fn list(&self) -> &gio::ListStore {
        &self.imp().list
    }

    /// The member list of the room of the reaction.
    pub fn members(&self) -> Option<MemberList> {
        self.imp().members.borrow().clone()
    }

    /// Set the members list of the room of the reaction.
    pub fn set_members(&self, members: Option<MemberList>) {
        let imp = self.imp();

        if imp.members.borrow().as_ref() == members.as_ref() {
            return;
        }

        imp.members.replace(members);
        self.notify("members");

        if let Some(group) = imp.group.obj() {
            self.items_changed(&group, 0, self.list().n_items(), group.n_items());
        }
    }

    fn items_changed(&self, group: &ReactionGroup, pos: u32, removed: u32, added: u32) {
        let Some(members) = &*self.imp().members.borrow() else {
            return;
        };

        let mut new_senders = Vec::with_capacity(added as usize);
        for i in pos..pos + added {
            let Some(boxed) = group.item(i).and_downcast::<glib::BoxedAnyObject>() else {
                break;
            };

            let sender_data = boxed.borrow::<SdkReactionSenderData>();
            let member = members.get_or_create(sender_data.sender_id.clone());
            let timestamp = sender_data.timestamp.as_secs().into();
            let sender = MemberReactionSender::new(&member, timestamp);

            new_senders.push(sender);
        }

        self.list().splice(pos, removed, &new_senders);
    }

    /// Handle a right click/long press on the reaction button.
    ///
    /// Shows a popover with the senders of that reaction, if there are any.
    #[template_callback]
    fn show_popover(&self) {
        if self.list().n_items() == 0 {
            // No popover.
            return;
        };

        let button = &*self.imp().button;
        let popover = ReactionPopover::new(self.list());
        popover.set_parent(button);
        popover.connect_closed(clone!(@weak button => move |popover| {
            popover.unparent();
        }));
        popover.popup();
    }
}
