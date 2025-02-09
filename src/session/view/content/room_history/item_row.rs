use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{gio, glib, glib::clone};
use matrix_sdk_ui::timeline::{TimelineEventItemId, TimelineItemContent};
use ruma::events::room::message::MessageType;
use tracing::error;

use super::{DividerRow, MessageRow, RoomHistory, StateRow, TypingRow};
use crate::{
    components::ContextMenuBin,
    prelude::*,
    session::{
        model::{Event, MessageState, Room, TimelineItem, VirtualItem, VirtualItemKind},
        view::{content::room_history::message_toolbar::ComposerState, EventDetailsDialog},
    },
    spawn, spawn_tokio, toast,
    utils::BoundObjectWeakRef,
};

mod imp {
    use std::{cell::RefCell, rc::Rc};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::ItemRow)]
    pub struct ItemRow {
        /// The ancestor room history of this row.
        #[property(get, set = Self::set_room_history, construct_only)]
        room_history: glib::WeakRef<RoomHistory>,
        message_toolbar_handler: RefCell<Option<glib::SignalHandlerId>>,
        composer_state: BoundObjectWeakRef<ComposerState>,
        /// The [`TimelineItem`] presented by this row.
        #[property(get, set = Self::set_item, explicit_notify, nullable)]
        item: RefCell<Option<TimelineItem>>,
        item_handlers: RefCell<Vec<glib::SignalHandlerId>>,
        /// The event action group of this row.
        #[property(get, set = Self::set_action_group)]
        action_group: RefCell<Option<gio::SimpleActionGroup>>,
        permissions_handler: RefCell<Option<glib::SignalHandlerId>>,
        binding: RefCell<Option<glib::Binding>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ItemRow {
        const NAME: &'static str = "RoomHistoryItemRow";
        type Type = super::ItemRow;
        type ParentType = ContextMenuBin;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("room-history-row");
            klass.set_accessible_role(gtk::AccessibleRole::ListItem);

            klass.install_action(
                "room-history-row.enable-copy-image",
                Some(&bool::static_variant_type()),
                |obj, _, param| {
                    let enable = param
                        .and_then(glib::Variant::get::<bool>)
                        .expect("The parameter should be a boolean");
                    let imp = obj.imp();

                    let Some(action_group) = imp.action_group.borrow().clone() else {
                        error!("Could not change state of copy-image action: no action group");
                        return;
                    };
                    let Some(action) = action_group.lookup_action("copy-image") else {
                        error!("Could not change state of copy-image action: action not found");
                        return;
                    };
                    let Some(action) = action.downcast_ref::<gio::SimpleAction>() else {
                        error!("Could not change state of copy-image action: not a GSimpleAction");
                        return;
                    };
                    action.set_enabled(enable);
                },
            );
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for ItemRow {
        fn constructed(&self) {
            self.parent_constructed();

            self.obj().connect_parent_notify(|obj| {
                obj.imp().update_highlight();
            });
        }

        fn dispose(&self) {
            self.disconnect_item_signals();

            if let Some(handler) = self.message_toolbar_handler.take() {
                if let Some(room_history) = self.room_history.upgrade() {
                    room_history.message_toolbar().disconnect(handler);
                }
            }
        }
    }

    impl WidgetImpl for ItemRow {}

    impl ContextMenuBinImpl for ItemRow {
        fn menu_opened(&self) {
            let Some(room_history) = self.room_history.upgrade() else {
                return;
            };

            let obj = self.obj();
            let Some(event) = self.item.borrow().clone().and_downcast::<Event>() else {
                obj.set_popover(None);
                return;
            };
            if self.action_group.borrow().is_none() {
                // There are no possible actions.
                obj.set_popover(None);
                return;
            };

            let menu = room_history.item_context_menu();

            // Reset the state when the popover is closed.
            let closed_handler_cell: Rc<RefCell<Option<glib::signal::SignalHandlerId>>> =
                Rc::default();
            let closed_handler = menu.popover.connect_closed(clone!(
                #[weak]
                obj,
                #[weak]
                room_history,
                #[strong]
                closed_handler_cell,
                move |popover| {
                    room_history.enable_sticky_mode(true);
                    obj.remove_css_class("has-open-popup");

                    if let Some(handler) = closed_handler_cell.take() {
                        popover.disconnect(handler);
                    }
                }
            ));
            closed_handler_cell.replace(Some(closed_handler));

            if event.can_be_reacted_to() {
                menu.add_quick_reaction_chooser(event.reactions());
            } else {
                menu.remove_quick_reaction_chooser();
            }

            room_history.enable_sticky_mode(false);
            obj.add_css_class("has-open-popup");

            obj.set_popover(Some(menu.popover.clone()));
        }
    }

    impl ItemRow {
        /// Set the ancestor room history of this row.
        fn set_room_history(&self, room_history: &RoomHistory) {
            self.room_history.set(Some(room_history));

            let message_toolbar = room_history.message_toolbar();
            let message_toolbar_handler =
                message_toolbar.connect_current_composer_state_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |message_toolbar| {
                        imp.watch_related_event(&message_toolbar.current_composer_state());
                    }
                ));
            self.message_toolbar_handler
                .replace(Some(message_toolbar_handler));

            self.watch_related_event(&message_toolbar.current_composer_state());
        }

        /// Watch the related event for given current composer state of the
        /// toolbar.
        fn watch_related_event(&self, composer_state: &ComposerState) {
            self.composer_state.disconnect_signals();

            let composer_state_handler = composer_state.connect_related_to_changed(clone!(
                #[weak(rename_to = imp)]
                self,
                move |composer_state| {
                    imp.update_for_related_event(
                        composer_state
                            .related_to()
                            .map(|info| info.identifier())
                            .as_ref(),
                    );
                }
            ));
            self.composer_state
                .set(composer_state, vec![composer_state_handler]);

            self.update_for_related_event(
                composer_state
                    .related_to()
                    .map(|info| info.identifier())
                    .as_ref(),
            );
        }

        /// Disconnect the signal handlers depending on the item.
        fn disconnect_item_signals(&self) {
            if let Some(item) = self.item.borrow().clone() {
                for handler in self.item_handlers.borrow_mut().drain(..) {
                    item.disconnect(handler);
                }

                if let Some(event) = item.downcast_ref::<Event>() {
                    if let Some(handler) = self.permissions_handler.take() {
                        event.room().permissions().disconnect(handler);
                    }
                }
            }

            if let Some(binding) = self.binding.take() {
                binding.unbind();
            }
        }

        /// Set the [`TimelineItem`] presented by this row.
        ///
        /// This tries to reuse the widget and only update the content whenever
        /// possible, but it will create a new widget and drop the old one if it
        /// has to.
        fn set_item(&self, item: Option<TimelineItem>) {
            // Reinitialize the header.
            self.obj().remove_css_class("has-header");

            self.disconnect_item_signals();

            if let Some(item) = &item {
                if let Some(event) = item.downcast_ref::<Event>() {
                    self.set_event(event);
                } else if let Some(item) = item.downcast_ref::<VirtualItem>() {
                    self.set_virtual_item(item);
                }
            }
            self.item.replace(item);

            self.update_highlight();
        }

        /// The event displayed by this row, if any.
        fn event(&self) -> Option<Event> {
            self.item.borrow().clone().and_downcast()
        }

        /// Set the event to display.
        fn set_event(&self, event: &Event) {
            let state_notify_handler = event.connect_state_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |event| {
                    imp.update_event_actions(Some(event.upcast_ref()));
                }
            ));

            let source_notify_handler = event.connect_source_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |event| {
                    imp.build_event_widget(event.clone());
                    imp.update_event_actions(Some(event.upcast_ref()));
                }
            ));

            let edit_source_notify_handler = event.connect_latest_edit_source_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |event| {
                    imp.build_event_widget(event.clone());
                    imp.update_event_actions(Some(event.upcast_ref()));
                }
            ));

            let is_highlighted_notify_handler = event.connect_is_highlighted_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.update_highlight();
                }
            ));

            self.item_handlers.borrow_mut().extend([
                state_notify_handler,
                source_notify_handler,
                edit_source_notify_handler,
                is_highlighted_notify_handler,
            ]);

            let permissions_handler = event.room().permissions().connect_changed(clone!(
                #[weak(rename_to = imp)]
                self,
                #[weak]
                event,
                move |_| {
                    imp.update_event_actions(Some(event.upcast_ref()));
                }
            ));
            self.permissions_handler.replace(Some(permissions_handler));

            self.build_event_widget(event.clone());
            self.update_event_actions(Some(event.upcast_ref()));
        }

        /// Set the virtual item to display.
        fn set_virtual_item(&self, virtual_item: &VirtualItem) {
            self.obj().set_popover(None);
            self.update_event_actions(None);

            let kind_handler = virtual_item.connect_kind_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |virtual_item| {
                    imp.build_virtual_item(virtual_item);
                }
            ));
            self.item_handlers.borrow_mut().push(kind_handler);

            self.build_virtual_item(virtual_item);
        }

        /// Construct the widget for the given virtual item.
        fn build_virtual_item(&self, virtual_item: &VirtualItem) {
            let obj = self.obj();
            let kind = &*virtual_item.kind();

            match kind {
                VirtualItemKind::Spinner => {
                    if !obj
                        .child()
                        .is_some_and(|widget| widget.is::<adw::Spinner>())
                    {
                        obj.set_child(Some(&spinner()));
                    }
                }
                VirtualItemKind::Typing => {
                    let child = if let Some(child) = obj.child().and_downcast::<TypingRow>() {
                        child
                    } else {
                        let child = TypingRow::new();
                        obj.set_child(Some(&child));
                        child
                    };

                    let typing_list = self
                        .room_history
                        .upgrade()
                        .and_then(|h| h.room())
                        .map(|room| room.typing_list());
                    child.set_list(typing_list);
                }
                VirtualItemKind::TimelineStart
                | VirtualItemKind::DayDivider(_)
                | VirtualItemKind::NewMessages => {
                    let divider = if let Some(divider) = obj.child().and_downcast::<DividerRow>() {
                        divider
                    } else {
                        let divider = DividerRow::new();
                        obj.set_child(Some(&divider));
                        divider
                    };
                    divider.set_kind(kind);
                }
            }
        }

        /// Set the event action group of this row.
        fn set_action_group(&self, action_group: Option<gio::SimpleActionGroup>) {
            if *self.action_group.borrow() == action_group {
                return;
            }

            self.action_group.replace(action_group);
        }

        /// Construct the widget for the given event
        fn build_event_widget(&self, event: Event) {
            let obj = self.obj();

            match event.content() {
                TimelineItemContent::MembershipChange(_)
                | TimelineItemContent::ProfileChange(_)
                | TimelineItemContent::OtherState(_) => {
                    let child = if let Some(child) = obj.child().and_downcast::<StateRow>() {
                        child
                    } else {
                        let child = StateRow::new();
                        obj.set_child(Some(&child));
                        child
                    };
                    child.set_event(event);
                }
                _ => {
                    let child = if let Some(child) = obj.child().and_downcast::<MessageRow>() {
                        child
                    } else {
                        let child = MessageRow::new();
                        obj.set_child(Some(&child));
                        child
                    };
                    child.set_event(event);
                }
            }
        }

        /// Update the highlight state of this row.
        fn update_highlight(&self) {
            let obj = self.obj();

            let highlight = self.event().is_some_and(|event| event.is_highlighted());
            if highlight {
                obj.add_css_class("highlight");
            } else {
                obj.remove_css_class("highlight");
            }
        }

        /// Replace the context menu with an emoji chooser for reactions.
        fn show_reactions_chooser(&self) {
            let obj = self.obj();

            let Some(popover) = obj.popover() else {
                return;
            };

            let (_, rectangle) = popover.pointing_to();

            let emoji_chooser = gtk::EmojiChooser::builder()
                .has_arrow(false)
                .pointing_to(&rectangle)
                .build();

            emoji_chooser.connect_emoji_picked(clone!(
                #[weak]
                obj,
                move |_, emoji| {
                    let _ = obj.activate_action("event.toggle-reaction", Some(&emoji.to_variant()));
                }
            ));
            emoji_chooser.connect_closed(|emoji_chooser| {
                emoji_chooser.unparent();
            });
            emoji_chooser.set_parent(&*obj);

            popover.popdown();
            emoji_chooser.popup();
        }

        /// Update this row for the related event with the given identifier.
        fn update_for_related_event(&self, related_event_id: Option<&TimelineEventItemId>) {
            let obj = self.obj();

            if related_event_id.is_some_and(|identifier| {
                self.event()
                    .is_some_and(|event| event.matches_identifier(identifier))
            }) {
                obj.add_css_class("selected");
            } else {
                obj.remove_css_class("selected");
            }
        }

        /// Update the actions available for the given event.
        ///
        /// Unsets the actions if `event` is `None`.
        fn update_event_actions(&self, event: Option<&Event>) {
            let obj = self.obj();

            let Some(event) = event else {
                obj.insert_action_group("event", None::<&gio::ActionGroup>);
                self.set_action_group(None);
                obj.set_has_context_menu(false);
                return;
            };

            let action_group = gio::SimpleActionGroup::new();
            let room = event.room();
            let has_event_id = event.event_id().is_some();

            if has_event_id {
                action_group.add_action_entries([
                    // Create a permalink.
                    gio::ActionEntry::builder("permalink")
                        .activate(clone!(
                            #[weak]
                            obj,
                            move |_, _, _| {
                                spawn!(async move {
                                    let Some(event) = obj.imp().event() else {
                                        return;
                                    };
                                    let Some(permalink) = event.matrix_to_uri().await else {
                                        return;
                                    };

                                    obj.clipboard().set_text(&permalink.to_string());
                                    toast!(obj, gettext("Message link copied to clipboard"));
                                });
                            }
                        ))
                        .build(),
                    // View event details.
                    gio::ActionEntry::builder("view-details")
                        .activate(clone!(
                            #[weak]
                            obj,
                            move |_, _, _| {
                                let Some(event) = obj.imp().event() else {
                                    return;
                                };

                                let dialog = EventDetailsDialog::new(&event);
                                dialog.present(Some(&obj));
                            }
                        ))
                        .build(),
                ]);

                if room.is_joined() {
                    action_group.add_action_entries([
                        // Report the event.
                        gio::ActionEntry::builder("report")
                            .activate(clone!(
                                #[weak(rename_to = imp)]
                                self,
                                move |_, _, _| {
                                    spawn!(async move {
                                        imp.report_event().await;
                                    });
                                }
                            ))
                            .build(),
                    ]);
                }
            } else {
                let state = event.state();

                if matches!(
                    state,
                    MessageState::Sending
                        | MessageState::RecoverableError
                        | MessageState::PermanentError
                ) {
                    // Cancel the event.
                    action_group.add_action_entries([gio::ActionEntry::builder("cancel-send")
                        .activate(clone!(
                            #[weak(rename_to = imp)]
                            self,
                            move |_, _, _| {
                                spawn!(async move {
                                    imp.cancel_send().await;
                                });
                            }
                        ))
                        .build()]);
                }
            }

            self.add_message_actions(&action_group, &room, event);

            obj.insert_action_group("event", Some(&action_group));
            self.set_action_group(Some(action_group));
            obj.set_has_context_menu(true);
        }

        /// Add actions to the given action group for the given event, if it is
        /// a message.
        ///
        /// See [`Event::is_message`] for the definition of a message-like
        /// event.
        fn add_message_actions(
            &self,
            action_group: &gio::SimpleActionGroup,
            room: &Room,
            event: &Event,
        ) {
            if !event.is_message() {
                return;
            }

            let own_member = room.own_member();
            let own_user_id = own_member.user_id();
            let is_from_own_user = event.sender_id() == *own_user_id;
            let permissions = room.permissions();
            let has_event_id = event.event_id().is_some();

            // Redact/remove the event.
            if has_event_id
                && ((is_from_own_user && permissions.can_redact_own())
                    || permissions.can_redact_other())
            {
                action_group.add_action_entries([gio::ActionEntry::builder("remove")
                    .activate(clone!(
                        #[weak(rename_to = imp)]
                        self,
                        move |_, _, _| {
                            spawn!(async move {
                                imp.redact_message().await;
                            });
                        }
                    ))
                    .build()]);
            };

            // Send/redact a reaction.
            if event.can_be_reacted_to() {
                action_group.add_action_entries([
                    gio::ActionEntry::builder("toggle-reaction")
                        .parameter_type(Some(&String::static_variant_type()))
                        .activate(clone!(
                            #[weak(rename_to = imp)]
                            self,
                            move |_, _, variant| {
                                let Some(key) = variant.unwrap().get::<String>() else {
                                    error!("Could not parse reaction to toggle");
                                    return;
                                };

                                spawn!(async move {
                                    imp.toggle_reaction(key).await;
                                });
                            }
                        ))
                        .build(),
                    gio::ActionEntry::builder("show-reactions-chooser")
                        .activate(clone!(
                            #[weak(rename_to = imp)]
                            self,
                            move |_, _, _| {
                                imp.show_reactions_chooser();
                            }
                        ))
                        .build(),
                ]);
            }

            // Reply.
            if event.can_be_replied_to() {
                action_group.add_action_entries([gio::ActionEntry::builder("reply")
                    .activate(clone!(
                        #[weak(rename_to = imp)]
                        self,
                        move |_, _, _| {
                            let Some(event) = imp.event() else {
                                error!("Could not reply to timeline item that is not an event");
                                return;
                            };
                            let Some(event_id) = event.event_id() else {
                                error!("Event to reply to does not have an event ID");
                                return;
                            };

                            if imp
                                .obj()
                                .activate_action(
                                    "room-history.reply",
                                    Some(&event_id.as_str().to_variant()),
                                )
                                .is_err()
                            {
                                error!("Could not activate `room-history.reply` action");
                            };
                        }
                    ))
                    .build()]);
            }

            self.add_message_content_actions(action_group, room, event);
        }

        /// Add actions to the given action group for the given event, if it
        /// includes message content.
        #[allow(clippy::too_many_lines)]
        fn add_message_content_actions(
            &self,
            action_group: &gio::SimpleActionGroup,
            room: &Room,
            event: &Event,
        ) {
            let TimelineItemContent::Message(message) = event.content() else {
                return;
            };

            let obj = self.obj();
            let own_member = room.own_member();
            let own_user_id = own_member.user_id();
            let is_from_own_user = event.sender_id() == *own_user_id;
            let permissions = room.permissions();
            let has_event_id = event.event_id().is_some();

            match message.msgtype() {
                MessageType::Text(_) | MessageType::Emote(_) => {
                    // Copy text.
                    action_group.add_action_entries([gio::ActionEntry::builder("copy-text")
                        .activate(clone!(
                            #[weak(rename_to = imp)]
                            self,
                            move |_, _, _| {
                                imp.copy_text();
                            }
                        ))
                        .build()]);

                    // Edit message.
                    if has_event_id && is_from_own_user && permissions.can_send_message() {
                        action_group.add_action_entries([gio::ActionEntry::builder("edit")
                            .activate(clone!(
                                #[weak(rename_to = imp)]
                                self,
                                move |_, _, _| {
                                    imp.edit_message();
                                }
                            ))
                            .build()]);
                    }
                }
                MessageType::File(_) => {
                    // Save message's file.
                    action_group.add_action_entries([gio::ActionEntry::builder("file-save")
                        .activate(clone!(
                            #[weak(rename_to = imp)]
                            self,
                            move |_, _, _| {
                                imp.save_file();
                            }
                        ))
                        .build()]);
                }
                MessageType::Notice(_) => {
                    // Copy text.
                    action_group.add_action_entries([gio::ActionEntry::builder("copy-text")
                        .activate(clone!(
                            #[weak(rename_to = imp)]
                            self,
                            move |_, _, _| {
                                imp.copy_text();
                            }
                        ))
                        .build()]);
                }
                MessageType::Image(_) => {
                    action_group.add_action_entries([
                        // Copy the texture to the clipboard.
                        gio::ActionEntry::builder("copy-image")
                            .activate(clone!(
                                #[weak]
                                obj,
                                move |_, _, _| {
                                    let texture = obj
                                        .child()
                                        .and_downcast::<MessageRow>()
                                        .and_then(|r| r.texture())
                                        .expect("An ItemRow with an image should have a texture");

                                    obj.clipboard().set_texture(&texture);
                                    toast!(obj, gettext("Thumbnail copied to clipboard"));
                                }
                            ))
                            .build(),
                        // Save the image to a file.
                        gio::ActionEntry::builder("save-image")
                            .activate(clone!(
                                #[weak(rename_to = imp)]
                                self,
                                move |_, _, _| {
                                    imp.save_file();
                                }
                            ))
                            .build(),
                    ]);
                }
                MessageType::Video(_) => {
                    // Save the video to a file.
                    action_group.add_action_entries([gio::ActionEntry::builder("save-video")
                        .activate(clone!(
                            #[weak(rename_to = imp)]
                            self,
                            move |_, _, _| {
                                imp.save_file();
                            }
                        ))
                        .build()]);
                }
                MessageType::Audio(_) => {
                    // Save the audio to a file.
                    action_group.add_action_entries([gio::ActionEntry::builder("save-audio")
                        .activate(clone!(
                            #[weak(rename_to = imp)]
                            self,
                            move |_, _, _| {
                                imp.save_file();
                            }
                        ))
                        .build()]);
                }
                _ => {}
            }

            if let Some(media_message) = event.media_message() {
                if media_message.caption().is_some() {
                    // Copy caption.
                    action_group.add_action_entries([gio::ActionEntry::builder("copy-text")
                        .activate(clone!(
                            #[weak(rename_to = imp)]
                            self,
                            move |_, _, _| {
                                imp.copy_text();
                            }
                        ))
                        .build()]);
                }
            }
        }

        /// Copy the text of this row.
        fn copy_text(&self) {
            let Some(event) = self.event() else {
                error!("Could not copy text of timeline item that is not an event");
                return;
            };
            let TimelineItemContent::Message(message) = event.content() else {
                error!("Could not copy text of event that is not a textual message");
                return;
            };

            let text = match message.msgtype() {
                MessageType::Text(text_message) => text_message.body.clone(),
                MessageType::Emote(emote_message) => {
                    let display_name = event.sender().display_name();
                    format!("{display_name} {}", emote_message.body)
                }
                MessageType::Notice(notice_message) => notice_message.body.clone(),
                _ => {
                    if let Some(caption) = event
                        .media_message()
                        .and_then(|m| m.caption().map(|(caption, _)| caption.to_owned()))
                    {
                        caption
                    } else {
                        error!("Could not copy text of event that is not a textual message");
                        return;
                    }
                }
            };

            let obj = self.obj();
            obj.clipboard().set_text(&text);
            toast!(obj, gettext("Text copied to clipboard"));
        }

        /// Edit the message of this row.
        fn edit_message(&self) {
            let Some(event) = self.event() else {
                error!("Could not edit timeline item that is not an event");
                return;
            };
            let Some(event_id) = event.event_id() else {
                error!("Event to edit does not have an event ID");
                return;
            };

            if self
                .obj()
                .activate_action("room-history.edit", Some(&event_id.as_str().to_variant()))
                .is_err()
            {
                error!("Could not activate `room-history.edit` action");
            };
        }

        /// Save the media file of this row.
        fn save_file(&self) {
            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    let Some(event) = imp.event() else {
                        error!("Could not save file of timeline item that is not an event");
                        return;
                    };
                    let Some(session) = event.room().session() else {
                        // Should only happen if the process is being closed.
                        return;
                    };
                    let Some(media_message) = event.media_message() else {
                        error!("Could not save file for non-media event");
                        return;
                    };

                    let client = session.client();
                    media_message.save_to_file(&client, &*imp.obj()).await;
                }
            ));
        }

        /// Redact the event of this row.
        async fn redact_message(&self) {
            let Some(event) = self.event() else {
                error!("Could not redact timeline item that is not an event");
                return;
            };
            let Some(event_id) = event.event_id() else {
                error!("Event to redact does not have an event ID");
                return;
            };
            let obj = self.obj();

            let confirm_dialog = adw::AlertDialog::builder()
                .default_response("cancel")
                .heading(gettext("Remove Message?"))
                .body(gettext(
                    "Do you really want to remove this message? This cannot be undone.",
                ))
                .build();
            confirm_dialog.add_responses(&[
                ("cancel", &gettext("Cancel")),
                ("remove", &gettext("Remove")),
            ]);
            confirm_dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);

            if confirm_dialog.choose_future(&*obj).await != "remove" {
                return;
            }

            if event.room().redact(&[event_id], None).await.is_err() {
                toast!(obj, gettext("Could not remove message"));
            }
        }

        /// Toggle the reaction with the given key for the event of this row.
        async fn toggle_reaction(&self, key: String) {
            let Some(event) = self.event() else {
                error!("Could not toggle reaction on timeline item that is not an event");
                return;
            };

            if event.room().toggle_reaction(key, &event).await.is_err() {
                let obj = self.obj();
                toast!(obj, gettext("Could not toggle reaction"));
            }
        }

        /// Report the current event.
        async fn report_event(&self) {
            let Some(event) = self.event() else {
                error!("Could not report timeline item that is not an event");
                return;
            };
            let Some(event_id) = event.event_id() else {
                error!("Event to report does not have an event ID");
                return;
            };
            let obj = self.obj();

            // Ask the user to confirm, and provide optional reason.
            let reason_entry = adw::EntryRow::builder()
                .title(gettext("Reason (optional)"))
                .build();
            let list_box = gtk::ListBox::builder()
                .css_classes(["boxed-list"])
                .margin_top(6)
                .accessible_role(gtk::AccessibleRole::Group)
                .build();
            list_box.append(&reason_entry);

            let confirm_dialog = adw::AlertDialog::builder()
            .default_response("cancel")
            .heading(gettext("Report Event?"))
            .body(gettext(
                "Reporting an event will send its unique ID to the administrator of your homeserver. The administrator will not be able to see the content of the event if it is encrypted or redacted.",
            ))
            .extra_child(&list_box)
            .build();
            confirm_dialog.add_responses(&[
                ("cancel", &gettext("Cancel")),
                // Translators: This is a verb, as in 'Report Event'.
                ("report", &gettext("Report")),
            ]);
            confirm_dialog.set_response_appearance("report", adw::ResponseAppearance::Destructive);

            if confirm_dialog.choose_future(&*obj).await != "report" {
                return;
            }

            let reason = Some(reason_entry.text())
                .filter(|s| !s.is_empty())
                .map(Into::into);

            if event
                .room()
                .report_events(&[(event_id, reason)])
                .await
                .is_err()
            {
                toast!(obj, gettext("Could not report event"));
            }
        }

        /// Cancel sending the event of this row.
        async fn cancel_send(&self) {
            let Some(event) = self.event() else {
                error!("Could not discard timeline item that is not an event");
                return;
            };

            let matrix_timeline = event.room().timeline().matrix_timeline();
            let identifier = event.identifier();
            let handle =
                spawn_tokio!(async move { matrix_timeline.redact(&identifier, None).await });

            if let Err(error) = handle.await.unwrap() {
                error!("Could not discard local event: {error}");
                let obj = self.obj();
                toast!(obj, gettext("Could not discard message"));
            }
        }
    }
}

glib::wrapper! {
    /// A row presenting an item in the room history.
    pub struct ItemRow(ObjectSubclass<imp::ItemRow>)
        @extends gtk::Widget, ContextMenuBin, @implements gtk::Accessible;
}

impl ItemRow {
    pub fn new(room_history: &RoomHistory) -> Self {
        glib::Object::builder()
            .property("room-history", room_history)
            .build()
    }
}

/// Create a spinner widget.
fn spinner() -> adw::Spinner {
    adw::Spinner::builder()
        .margin_top(12)
        .margin_bottom(12)
        .height_request(24)
        .width_request(24)
        .build()
}
