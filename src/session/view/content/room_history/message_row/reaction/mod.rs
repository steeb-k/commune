use adw::subclass::prelude::*;
use gtk::{gio, glib, glib::clone, prelude::*, CompositeTemplate};
use matrix_sdk_ui::timeline::ReactionSenderData as SdkReactionSenderData;

mod reaction_popover;

use self::reaction_popover::ReactionPopover;
use crate::{
    gettext_f, ngettext_f,
    prelude::*,
    session::{
        model::{Member, MemberList, ReactionGroup},
        view::content::room_history::member_timestamp::MemberTimestamp,
    },
    utils::{BoundObjectWeakRef, EMOJI_REGEX},
};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_history/message_row/reaction/mod.ui"
    )]
    #[properties(wrapper_type = super::MessageReaction)]
    pub struct MessageReaction {
        /// The reaction senders (group) to display.
        #[property(get, set = Self::set_group, construct_only)]
        pub group: BoundObjectWeakRef<ReactionGroup>,
        /// The list of reaction senders as room members.
        #[property(get)]
        pub list: gio::ListStore,
        /// The member list of the room of the reaction.
        #[property(get, set = Self::set_members, explicit_notify, nullable)]
        pub members: RefCell<Option<MemberList>>,
        #[template_child]
        pub button: TemplateChild<gtk::ToggleButton>,
        #[template_child]
        pub reaction_key: TemplateChild<gtk::Label>,
        #[template_child]
        pub reaction_count: TemplateChild<gtk::Label>,
        /// The displayed member if there is only one reaction sendr.
        pub reaction_member: BoundObjectWeakRef<Member>,
    }

    impl Default for MessageReaction {
        fn default() -> Self {
            Self {
                group: Default::default(),
                list: gio::ListStore::new::<MemberTimestamp>(),
                members: Default::default(),
                button: Default::default(),
                reaction_key: Default::default(),
                reaction_count: Default::default(),
                reaction_member: Default::default(),
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

    #[glib::derived_properties]
    impl ObjectImpl for MessageReaction {}

    impl WidgetImpl for MessageReaction {}
    impl FlowBoxChildImpl for MessageReaction {}

    impl MessageReaction {
        /// Set the reaction group to display.
        fn set_group(&self, group: ReactionGroup) {
            let obj = self.obj();
            let key = group.key();
            self.reaction_key.set_label(&key);

            if EMOJI_REGEX.is_match(&key) {
                self.reaction_key.add_css_class("emoji");
            } else {
                self.reaction_key.remove_css_class("emoji");
            }

            self.button.set_action_target_value(Some(&key.to_variant()));
            group
                .bind_property("has-user", &*self.button, "active")
                .sync_create()
                .build();
            group
                .bind_property("count", &*self.reaction_count, "label")
                .sync_create()
                .build();

            group
                .bind_property("count", &*self.reaction_count, "visible")
                .sync_create()
                .transform_to(|_, count: u32| Some(count > 1))
                .build();

            let items_changed_handler_id = group.connect_items_changed(clone!(
                #[weak]
                obj,
                move |group, pos, removed, added| obj.items_changed(group, pos, removed, added)
            ));
            obj.items_changed(&group, 0, self.list.n_items(), group.n_items());

            self.group.set(&group, vec![items_changed_handler_id]);
        }

        /// Set the members list of the room of the reaction.
        fn set_members(&self, members: Option<MemberList>) {
            if *self.members.borrow() == members {
                return;
            }
            let obj = self.obj();

            self.members.replace(members);
            obj.notify_members();

            if let Some(group) = self.group.obj() {
                obj.items_changed(&group, 0, self.list.n_items(), group.n_items());
            }
        }
    }
}

glib::wrapper! {
    /// A widget displaying a reaction of a message.
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
            let sender = MemberTimestamp::new(&member, Some(timestamp));

            new_senders.push(sender);
        }

        self.list().splice(pos, removed, &new_senders);
        self.update_tooltip();
    }

    /// Update the text of the tooltip.
    fn update_tooltip(&self) {
        let Some(group) = self.group() else {
            return;
        };

        let imp = self.imp();
        imp.reaction_member.disconnect_signals();
        let n_items = self.list().n_items();

        if n_items == 1 {
            if let Some(member) = self
                .list()
                .item(0)
                .and_downcast::<MemberTimestamp>()
                .and_then(|r| r.member())
            {
                // Listen to changes of the display name.
                let handler_id = member.connect_display_name_notify(clone!(
                    #[weak(rename_to = obj)]
                    self,
                    move |member| {
                        obj.update_member_tooltip(member);
                    }
                ));

                imp.reaction_member.set(&member, vec![handler_id]);
                self.update_member_tooltip(&member);
                return;
            }
        }

        let text = (n_items > 0).then(|| {
            ngettext_f(
                // Translators: Do NOT translate the content between '{' and '}', this is a
                // variable name.
                "1 member reacted with {reaction_key}",
                "{n} members reacted with {reaction_key}",
                n_items,
                &[("n", &n_items.to_string()), ("reaction_key", &group.key())],
            )
        });

        imp.button.set_tooltip_text(text.as_deref())
    }

    fn update_member_tooltip(&self, member: &Member) {
        let Some(group) = self.group() else {
            return;
        };

        // Translators: Do NOT translate the content between '{' and '}', this is a
        // variable name.
        let text = gettext_f(
            "{user} reacted with {reaction_key}",
            &[
                ("user", &member.disambiguated_name()),
                ("reaction_key", &group.key()),
            ],
        );

        self.imp().button.set_tooltip_text(Some(&text));
    }

    /// Handle a right click/long press on the reaction button.
    ///
    /// Shows a popover with the senders of that reaction, if there are any.
    #[template_callback]
    fn show_popover(&self) {
        let list = self.list();
        if list.n_items() == 0 {
            // No popover.
            return;
        };

        let button = &*self.imp().button;
        let popover = ReactionPopover::new(&list);
        popover.set_parent(button);
        popover.connect_closed(|popover| {
            popover.unparent();
        });
        popover.popup();
    }
}
