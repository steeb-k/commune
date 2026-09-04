use adw::{prelude::*, subclass::prelude::*};
use gtk::{gdk, glib, glib::clone};
use tracing::error;

mod audio;
mod caption;
mod content;
mod file;
mod info;
#[cfg(not(target_os = "android"))]
mod location;
mod message_state_stack;
mod reaction;
mod reaction_list;
mod reply;
mod sender_name;
mod text;
mod url_preview;
mod visual_media;

use gettextrs::gettext;
use matrix_sdk_ui::timeline::{TimelineEventShieldState, TimelineEventShieldStateCode};

pub use self::content::{ContentFormat, MessageContent};
use self::{
    message_state_stack::MessageStateStack, reaction_list::MessageReactionList,
    sender_name::MessageSenderName,
};
use super::{EventTimestamp, ReadReceiptsList};
use crate::{
    Application,
    components::UserProfileDialog,
    ngettext_f,
    prelude::*,
    session::{Event, EventHeaderState, Member},
    utils::BoundObject,
};

/// The setting that says whether messages are presented as chat bubbles.
const CHAT_BUBBLES_SETTING: &str = "chat-bubbles-enabled";

mod imp {
    use std::{
        cell::{Cell, RefCell},
        marker::PhantomData,
    };

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session_view/room_history/message_row/mod.ui")]
    #[properties(wrapper_type = super::MessageRow)]
    pub struct MessageRow {
        #[template_child]
        avatar_button: TemplateChild<gtk::Button>,
        #[template_child]
        header: TemplateChild<gtk::Box>,
        #[template_child]
        display_name: TemplateChild<MessageSenderName>,
        #[template_child]
        timestamp: TemplateChild<EventTimestamp>,
        #[template_child]
        content: TemplateChild<MessageContent>,
        #[template_child]
        message_state: TemplateChild<MessageStateStack>,
        #[template_child]
        shield_icon: TemplateChild<gtk::Image>,
        #[template_child]
        state_box: TemplateChild<gtk::Box>,
        #[template_child]
        reactions: TemplateChild<MessageReactionList>,
        #[template_child]
        thread_chip: TemplateChild<gtk::Button>,
        #[template_child]
        thread_replies_label: TemplateChild<gtk::Label>,
        binding: RefCell<Option<glib::Binding>>,
        /// Whether messages are presented as chat bubbles.
        bubbles_enabled: Cell<bool>,
        /// The handler watching the chat bubbles setting.
        settings_handler: RefCell<Option<glib::SignalHandlerId>>,
        /// The event that is presented.
        #[property(get, set = Self::set_event, explicit_notify)]
        event: BoundObject<Event>,
        /// The sender of the event that is presented.
        #[property(get = Self::sender)]
        sender: PhantomData<Option<Member>>,
        /// The texture of the image preview displayed by the descendant of this
        /// widget, if any.
        #[property(get = Self::texture)]
        texture: PhantomData<Option<gdk::Texture>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MessageRow {
        const NAME: &'static str = "ContentMessageRow";
        type Type = super::MessageRow;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            EventTimestamp::ensure_type();
            ReadReceiptsList::ensure_type();

            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);
            klass.set_css_name("message-row");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for MessageRow {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            self.content.connect_format_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |content| {
                    imp.reactions.set_visible(!matches!(
                        content.format(),
                        ContentFormat::Compact | ContentFormat::Ellipsized
                    ));
                    imp.update_thread_chip();
                }
            ));
            self.content.connect_texture_notify(clone!(
                #[weak]
                obj,
                move |_| {
                    obj.notify_texture();
                }
            ));

            let settings = Application::default().settings();
            self.bubbles_enabled
                .set(settings.boolean(CHAT_BUBBLES_SETTING));
            let settings_handler = settings.connect_changed(
                Some(CHAT_BUBBLES_SETTING),
                clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |settings, _| {
                        imp.bubbles_enabled
                            .set(settings.boolean(CHAT_BUBBLES_SETTING));
                        imp.update_bubbles();
                        imp.update_header();
                    }
                ),
            );
            self.settings_handler.replace(Some(settings_handler));
        }

        fn dispose(&self) {
            if let Some(binding) = self.binding.take() {
                binding.unbind();
            }
            if let Some(handler) = self.settings_handler.take() {
                Application::default().settings().disconnect(handler);
            }
        }
    }

    impl WidgetImpl for MessageRow {}
    impl BinImpl for MessageRow {}

    #[gtk::template_callbacks]
    impl MessageRow {
        /// Set the event that is presented.
        fn set_event(&self, event: Event) {
            let obj = self.obj();

            // Remove signals and bindings from the previous event.
            self.event.disconnect_signals();
            if let Some(binding) = self.binding.take() {
                binding.unbind();
            }

            let sender = event.sender();
            self.display_name.set_sender(Some(sender));

            let state_binding = event
                .bind_property("state", &*self.message_state, "state")
                .sync_create()
                .build();

            self.binding.replace(Some(state_binding));

            let header_state_handler = event.connect_header_state_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.update_header();
                }
            ));

            // Listening to changes in the source might not be enough, there are changes
            // that we display that do not affect the source, like related events.
            let item_changed_handler = event.connect_item_changed(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.update_content();
                    imp.update_thread_chip();
                    imp.update_shield();
                }
            ));

            self.reactions
                .set_reaction_list(&event.room().get_or_create_members(), &event.reactions());
            self.event
                .set(event, vec![header_state_handler, item_changed_handler]);

            obj.notify_event();
            obj.notify_sender();

            self.update_content();
            self.update_bubbles();
            self.update_header();
            self.update_thread_chip();
            self.update_shield();
        }

        /// The sender of the event that is presented.
        fn sender(&self) -> Option<Member> {
            self.event.obj().map(|event| event.sender())
        }

        /// Update the header for the current event.
        fn update_header(&self) {
            let Some(event) = self.event.obj() else {
                return;
            };

            let header_state = event.header_state();
            let avatar_visible = header_state == EventHeaderState::Full;
            let header_visible = header_state != EventHeaderState::Hidden;

            self.avatar_button.set_visible(avatar_visible);
            self.display_name.set_visible(avatar_visible);
            self.header.set_visible(header_visible);

            if let Some(row) = self.obj().parent() {
                if avatar_visible {
                    row.add_css_class("has-avatar");
                } else {
                    row.remove_css_class("has-avatar");
                }
            }
        }

        /// Whether this row is a bubble of the account's own user.
        fn is_own_bubble(&self) -> bool {
            self.bubbles_enabled.get() && self.sender().is_some_and(|sender| sender.is_own_user())
        }

        /// Update the presentation for the chat bubbles setting.
        ///
        /// The classes carry the drawing, but a bubble also hugs its content
        /// and an own bubble sits at the end of the line, which only the
        /// alignments can say.
        fn update_bubbles(&self) {
            let enabled = self.bubbles_enabled.get();
            let own = self.is_own_bubble();
            let obj = self.obj();

            if enabled {
                obj.add_css_class("bubble");
            } else {
                obj.remove_css_class("bubble");
            }
            if own {
                obj.add_css_class("bubble-own");
            } else {
                obj.remove_css_class("bubble-own");
            }
            if enabled {
                self.content.add_css_class("bubble-surface");
            } else {
                self.content.remove_css_class("bubble-surface");
            }

            let content_halign = if !enabled {
                gtk::Align::Fill
            } else if own {
                gtk::Align::End
            } else {
                gtk::Align::Start
            };
            self.content.set_halign(content_halign);

            let trailing = if own {
                gtk::Align::End
            } else {
                gtk::Align::Start
            };
            self.reactions
                .set_halign(if enabled { trailing } else { gtk::Align::Fill });
            self.thread_chip.set_halign(trailing);

            // In bubbles, everything about a sender clusters on the
            // message's side of the line: the name and the timestamp stop
            // spanning the row and sit together over the bubble's edge,
            // beside the avatar. The flat view keeps its two corners.
            self.header
                .set_halign(if enabled { trailing } else { gtk::Align::Fill });
            self.display_name.set_hexpand(!enabled);
            self.timestamp.set_hexpand(!enabled);

            // The name is always the innermost piece, against the avatar,
            // and the timestamp always on the outside — which on an own
            // bubble means the time comes first.
            if enabled && own {
                self.header
                    .reorder_child_after(&*self.timestamp, gtk::Widget::NONE);
            } else {
                self.header
                    .reorder_child_after(&*self.display_name, gtk::Widget::NONE);
            }

            // The avatar of an own bubble sits on the bubble's side of the
            // line: the far column of the grid instead of the first. The
            // delivery state and shield swap the other way — between the
            // bubble and the avatar they would wedge the bubble's edge away
            // from the line the name and the avatar draw, which is exactly
            // the misalignment the left side does not have.
            if let Some(grid) = self.avatar_button.parent().and_downcast::<gtk::Grid>()
                && let Some(manager) = grid.layout_manager()
            {
                if let Ok(layout_child) = manager
                    .layout_child(&*self.avatar_button)
                    .downcast::<gtk::GridLayoutChild>()
                {
                    layout_child.set_column(if own { 3 } else { 0 });
                }
                if let Ok(layout_child) = manager
                    .layout_child(&*self.state_box)
                    .downcast::<gtk::GridLayoutChild>()
                {
                    layout_child.set_column(if own { 0 } else { 2 });
                }
            }
        }

        /// Update the authenticity shield for the current event.
        ///
        /// A red shield is a warning — an unverified or mismatched sender, a
        /// message sent in the clear — and a grey one is a caveat. Most
        /// messages draw neither, which is what keeps the two readable.
        fn update_shield(&self) {
            let Some(event) = self.event.obj() else {
                return;
            };

            let (visible, code) = match event.shield() {
                TimelineEventShieldState::Red { code } => {
                    self.shield_icon
                        .set_icon_name(Some("verified-danger-symbolic"));
                    self.shield_icon.add_css_class("error");
                    self.shield_icon.remove_css_class("dim-label");
                    (true, Some(code))
                }
                TimelineEventShieldState::Grey { code } => {
                    self.shield_icon
                        .set_icon_name(Some("verified-warning-symbolic"));
                    self.shield_icon.add_css_class("dim-label");
                    self.shield_icon.remove_css_class("error");
                    (true, Some(code))
                }
                TimelineEventShieldState::None => (false, None),
            };

            self.shield_icon.set_visible(visible);
            self.shield_icon
                .set_tooltip_text(code.map(shield_message).as_deref());
        }

        /// Update the content for the current event.
        fn update_content(&self) {
            let Some(event) = self.event.obj() else {
                return;
            };

            self.content.update_for_event(&event);
        }

        /// Update the thread chip for the current event.
        fn update_thread_chip(&self) {
            let Some(event) = self.event.obj() else {
                return;
            };

            // Inside the thread's own view the chip would open what is
            // already open, and the thread-focused timeline does not keep the
            // summary current: the chip belongs to the room's history.
            if event.timeline().is_thread() {
                self.thread_chip.set_visible(false);
                return;
            }

            // The chip is also the way into the thread, so it needs the
            // root's event ID as the action target.
            let event_id = event.event_id();

            // The count can be zero when every reply in the thread has been
            // redacted; a chip announcing a thread with nothing to read is
            // worse than none.
            let num_replies = event
                .thread_summary()
                .map(|summary| summary.num_replies)
                .filter(|count| *count > 0);

            let compact = matches!(
                self.content.format(),
                ContentFormat::Compact | ContentFormat::Ellipsized
            );

            // The action takes the root's ID, so the chip carries the action
            // only together with its target: a button naming a parameterised
            // action with no target makes GTK warn at every draw — thousands
            // of lines a minute on a busy room — even while it is hidden.
            if let (Some(count), Some(event_id)) = (num_replies, &event_id) {
                self.thread_replies_label.set_label(&ngettext_f(
                    // Translators: Do NOT translate the content between '{' and
                    // '}', this is a variable name.
                    "1 reply",
                    "{n} replies",
                    count,
                    &[("n", &count.to_string())],
                ));
                self.thread_chip
                    .set_action_target_value(Some(&event_id.as_str().to_variant()));
                self.thread_chip
                    .set_action_name(Some("room-history.show-thread"));
            } else {
                self.thread_chip.set_action_name(None);
                self.thread_chip.set_action_target_value(None);
            }

            self.thread_chip
                .set_visible(num_replies.is_some() && event_id.is_some() && !compact);
        }

        /// Get the texture displayed by this widget, if any.
        pub(super) fn texture(&self) -> Option<gdk::Texture> {
            self.content.texture()
        }

        /// View the profile of the sender.
        #[template_callback]
        fn view_sender_profile(&self) {
            let Some(sender) = self.sender() else {
                error!("Could not open profile for missing sender");
                return;
            };

            let dialog = UserProfileDialog::new();
            dialog.set_room_member(sender);
            dialog.present(Some(&*self.obj()));
        }
    }
}

/// The sentence for the given shield code.
fn shield_message(code: TimelineEventShieldStateCode) -> String {
    match code {
        TimelineEventShieldStateCode::AuthenticityNotGuaranteed => {
            gettext("The authenticity of this message cannot be guaranteed on this device.")
        }
        TimelineEventShieldStateCode::UnknownDevice => {
            gettext("The device that sent this message is not known.")
        }
        TimelineEventShieldStateCode::UnsignedDevice => {
            gettext("The device that sent this message has not been verified by its owner.")
        }
        TimelineEventShieldStateCode::UnverifiedIdentity => {
            gettext("The sender of this message has not been verified.")
        }
        TimelineEventShieldStateCode::VerificationViolation => {
            gettext("The sender of this message was verified once, and has changed identity since.")
        }
        TimelineEventShieldStateCode::MismatchedSender => {
            gettext("The sender of this message does not match the device that encrypted it.")
        }
        TimelineEventShieldStateCode::SentInClear => {
            gettext("This message was not encrypted, in a room that is.")
        }
    }
}

glib::wrapper! {
    /// A row displaying a message in the timeline.
    pub struct MessageRow(ObjectSubclass<imp::MessageRow>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl MessageRow {
    pub fn new() -> Self {
        glib::Object::new()
    }
}

impl Default for MessageRow {
    fn default() -> Self {
        Self::new()
    }
}
