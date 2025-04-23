use std::time::Duration;

use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{gdk, gio, glib, glib::clone, graphene::Point, CompositeTemplate};
use matrix_sdk::ruma::EventId;
use matrix_sdk_ui::timeline::TimelineEventItemId;
use ruma::{
    api::client::receipt::create_receipt::v3::ReceiptType, events::room::message::MessageType,
    OwnedEventId,
};
use tracing::{error, warn};

mod divider_row;
mod event_actions;
mod event_row;
mod member_timestamp;
mod message_row;
mod message_toolbar;
mod read_receipts_list;
mod sender_avatar;
mod state;
mod title;
mod typing_row;
mod verification_info_bar;

use self::{
    divider_row::DividerRow,
    event_actions::*,
    event_row::EventRow,
    message_row::MessageRow,
    message_toolbar::MessageToolbar,
    read_receipts_list::ReadReceiptsList,
    sender_avatar::SenderAvatar,
    state::{StateGroupRow, StateRow},
    title::RoomHistoryTitle,
    typing_row::TypingRow,
    verification_info_bar::VerificationInfoBar,
};
use super::{room_details, RoomDetails};
use crate::{
    components::{confirm_leave_room_dialog, DragOverlay},
    prelude::*,
    session::model::{
        Event, MemberList, Membership, ReceiptPosition, Room, TargetRoomCategory, Timeline,
        VirtualItem, VirtualItemKind,
    },
    spawn, toast,
    utils::{BoundObject, GroupingListGroup, GroupingListModel, LoadingState, TemplateCallbacks},
    Window,
};

/// The time to wait before considering that scrolling has ended.
const SCROLL_TIMEOUT: Duration = Duration::from_millis(500);
/// The time to wait before considering that messages on a screen where read.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

mod imp {
    use std::{
        cell::{Cell, OnceCell, RefCell},
        ops::ControlFlow,
    };

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session/view/content/room_history/mod.ui")]
    #[properties(wrapper_type = super::RoomHistory)]
    pub struct RoomHistory {
        #[template_child]
        sender_menu_model: TemplateChild<gio::Menu>,
        #[template_child]
        pub(super) header_bar: TemplateChild<adw::HeaderBar>,
        #[template_child]
        room_title: TemplateChild<RoomHistoryTitle>,
        #[template_child]
        room_menu: TemplateChild<gtk::MenuButton>,
        #[template_child]
        listview: TemplateChild<gtk::ListView>,
        #[template_child]
        content: TemplateChild<gtk::Widget>,
        #[template_child]
        scrolled_window: TemplateChild<gtk::ScrolledWindow>,
        #[template_child]
        scroll_btn: TemplateChild<gtk::Button>,
        #[template_child]
        scroll_btn_revealer: TemplateChild<gtk::Revealer>,
        #[template_child]
        pub(super) message_toolbar: TemplateChild<MessageToolbar>,
        #[template_child]
        loading: TemplateChild<adw::Spinner>,
        #[template_child]
        error: TemplateChild<adw::StatusPage>,
        #[template_child]
        stack: TemplateChild<gtk::Stack>,
        #[template_child]
        tombstoned_banner: TemplateChild<adw::Banner>,
        #[template_child]
        drag_overlay: TemplateChild<DragOverlay>,
        /// The context menu for rows presenting an [`Event`].
        event_context_menu: OnceCell<EventActionsContextMenu>,
        sender_context_menu: OnceCell<gtk::PopoverMenu>,
        /// The timeline currently displayed.
        #[property(get, set = Self::set_timeline, explicit_notify, nullable)]
        timeline: BoundObject<Timeline>,
        /// Whether this is the only view visible, i.e. there is no sidebar.
        #[property(get, set)]
        is_only_view: Cell<bool>,
        /// The members of the current room.
        ///
        /// We hold a strong reference here to keep the list in memory as long
        /// as the room is opened.
        room_members: RefCell<Option<MemberList>>,
        /// Whether the current room history scrolling is automatic.
        is_auto_scrolling: Cell<bool>,
        /// Whether the room history should stick to the newest message in the
        /// timeline.
        #[property(get)]
        is_sticky: Cell<bool>,
        /// The `GroupingListModel` used in the list view.
        grouping_model: OnceCell<GroupingListModel>,
        scroll_timeout: RefCell<Option<glib::SourceId>>,
        read_timeout: RefCell<Option<glib::SourceId>>,
        room_handlers: RefCell<Vec<glib::SignalHandlerId>>,
        can_invite_handler: RefCell<Option<glib::SignalHandlerId>>,
        membership_handler: RefCell<Option<glib::SignalHandlerId>>,
        join_rule_handler: RefCell<Option<glib::SignalHandlerId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RoomHistory {
        const NAME: &'static str = "ContentRoomHistory";
        type Type = super::RoomHistory;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            VerificationInfoBar::ensure_type();

            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);
            TemplateCallbacks::bind_template_callbacks(klass);

            klass.set_accessible_role(gtk::AccessibleRole::Group);

            klass.install_action_async("room-history.leave", None, |obj, _, _| async move {
                obj.imp().leave().await;
            });
            klass.install_action_async("room-history.join", None, |obj, _, _| async move {
                obj.imp().join().await;
            });
            klass.install_action_async("room-history.forget", None, |obj, _, _| async move {
                obj.imp().forget().await;
            });

            klass.install_action("room-history.details", None, |obj, _, _| {
                obj.open_room_details(None);
            });
            klass.install_action("room-history.invite-members", None, |obj, _, _| {
                obj.open_room_details(Some(room_details::SubpageName::Invite));
            });

            klass.install_action(
                "room-history.scroll-to-event",
                Some(&TimelineEventItemId::static_variant_type()),
                |obj, _, v| {
                    let Some(event_key) = v.and_then(TimelineEventItemId::from_variant) else {
                        error!("Could not parse event identifier to scroll to");
                        return;
                    };

                    obj.imp().scroll_to_event(&event_key);
                },
            );

            klass.install_action(
                "room-history.reply",
                Some(&String::static_variant_type()),
                |obj, _, v| {
                    let Some(event_id) = v
                        .and_then(String::from_variant)
                        .and_then(|s| EventId::parse(s).ok())
                    else {
                        error!("Could not parse event ID to reply to");
                        return;
                    };

                    let Some(event) = obj.timeline().and_then(|timeline| {
                        timeline.event_by_identifier(&TimelineEventItemId::EventId(event_id))
                    }) else {
                        warn!("Could not find event to reply to");
                        return;
                    };

                    obj.imp().message_toolbar.set_reply_to(event);
                },
            );

            klass.install_action(
                "room-history.edit",
                Some(&String::static_variant_type()),
                |obj, _, v| {
                    let Some(event_id) = v
                        .and_then(String::from_variant)
                        .and_then(|s| EventId::parse(s).ok())
                    else {
                        error!("Could not parse event ID to edit");
                        return;
                    };

                    let Some(event) = obj.timeline().and_then(|timeline| {
                        timeline.event_by_identifier(&TimelineEventItemId::EventId(event_id))
                    }) else {
                        warn!("Could not find event to edit");
                        return;
                    };

                    obj.imp().message_toolbar.set_edit(&event);
                },
            );

            klass.install_action("room-history.edit-latest-message", None, |obj, _, _| {
                let Some(timeline) = obj.timeline() else {
                    return;
                };

                let own_member = timeline.room().own_member();
                let own_user_id = own_member.user_id();

                // Find the latest editable message that was sent by our user.
                let Some(event) = timeline
                    .items()
                    .iter::<glib::Object>()
                    .rev()
                    .find_map(|item| {
                        item.ok().and_downcast::<Event>().filter(|event| {
                            event.sender_id() == *own_user_id
                                && event.event_id().is_some()
                                && event.message().is_some_and(|message| {
                                    matches!(
                                        message.msgtype(),
                                        MessageType::Text(_) | MessageType::Emote(_)
                                    )
                                })
                        })
                    })
                else {
                    warn!("Could not find latest event to edit");
                    return;
                };

                obj.imp().message_toolbar.set_edit(&event);
            });
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for RoomHistory {
        fn constructed(&self) {
            self.parent_constructed();

            self.init_listview();
            self.init_drop_target();

            self.scroll_btn_revealer
                .connect_child_revealed_notify(|revealer| {
                    // Hide the revealer when we don't want to show the child and the animation is
                    // finished.
                    if !revealer.reveals_child() && !revealer.is_child_revealed() {
                        revealer.set_visible(false);
                    }
                });
        }

        fn dispose(&self) {
            self.disconnect_all();
        }
    }

    impl WidgetImpl for RoomHistory {
        fn grab_focus(&self) -> bool {
            if self.message_toolbar.grab_focus() {
                true
            } else {
                self.room_title.grab_focus()
            }
        }
    }

    impl BinImpl for RoomHistory {}

    #[gtk::template_callbacks]
    impl RoomHistory {
        /// Initialize the list view.
        fn init_listview(&self) {
            let factory = gtk::SignalListItemFactory::new();
            factory.connect_setup(move |_, list_item| {
                let Some(list_item) = list_item.downcast_ref::<gtk::ListItem>() else {
                    error!("List item factory did not receive a list item: {list_item:?}");
                    return;
                };

                list_item.set_activatable(false);
                list_item.set_selectable(false);
            });
            factory.connect_bind(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_, list_item| {
                    let Some(list_item) = list_item.downcast_ref::<gtk::ListItem>() else {
                        error!("List item factory did not receive a list item: {list_item:?}");
                        return;
                    };

                    imp.bind_list_item_to_item(list_item);
                }
            ));
            self.listview.set_factory(Some(&factory));

            // Needed to use the natural height of GtkPictures
            self.listview
                .set_vscroll_policy(gtk::ScrollablePolicy::Natural);

            let selection_model = gtk::NoSelection::new(Some(self.grouping_model().clone()));
            self.listview.set_model(Some(&selection_model));

            self.set_sticky(true);
            let adj = self.listview.vadjustment().unwrap();

            adj.connect_value_changed(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.scroll_value_changed();
                }
            ));
            adj.connect_upper_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.scroll_max_value_changed();
                }
            ));
            adj.connect_page_size_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.scroll_max_value_changed();
                }
            ));
        }

        /// Initialize the drop target.
        fn init_drop_target(&self) {
            let target = gtk::DropTarget::new(
                gio::File::static_type(),
                gdk::DragAction::COPY | gdk::DragAction::MOVE,
            );

            target.connect_drop(clone!(
                #[weak(rename_to = imp)]
                self,
                #[upgrade_or]
                false,
                move |_, value, _, _| {
                    match value.get::<gio::File>() {
                        Ok(file) => {
                            spawn!(async move {
                                imp.message_toolbar.send_file(file).await;
                            });
                            true
                        }
                        Err(error) => {
                            warn!("Could not get file from drop: {error:?}");
                            toast!(imp.obj(), gettext("Error getting file from drop"));

                            false
                        }
                    }
                }
            ));

            self.drag_overlay.set_drop_target(target);
        }

        /// Disconnect all the signals.
        fn disconnect_all(&self) {
            if let Some(room) = self.room() {
                for handler in self.room_handlers.take() {
                    room.disconnect(handler);
                }

                if let Some(handler) = self.can_invite_handler.take() {
                    room.permissions().disconnect(handler);
                }
                if let Some(handler) = self.membership_handler.take() {
                    room.own_member().disconnect(handler);
                }
                if let Some(handler) = self.join_rule_handler.take() {
                    room.join_rule().disconnect(handler);
                }
            }

            self.timeline.disconnect_signals();
        }

        /// Set the timeline currently displayed.
        #[allow(clippy::too_many_lines)]
        fn set_timeline(&self, timeline: Option<Timeline>) {
            if self.timeline.obj() == timeline {
                return;
            }

            self.disconnect_all();
            if let Some(source_id) = self.scroll_timeout.take() {
                source_id.remove();
            }
            if let Some(source_id) = self.read_timeout.take() {
                source_id.remove();
            }

            if let Some(timeline) = timeline {
                let room = timeline.room();

                // Keep a strong reference to the members list before changing the model, so all
                // events use the same list.
                self.room_members
                    .replace(Some(room.get_or_create_members()));

                let membership_handler = room.own_member().connect_membership_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_room_menu();
                    }
                ));
                self.membership_handler.replace(Some(membership_handler));

                let join_rule_handler = room.join_rule().connect_we_can_join_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_room_menu();
                    }
                ));
                self.join_rule_handler.replace(Some(join_rule_handler));

                let can_invite_handler = room.permissions().connect_can_invite_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_invite_action();
                    }
                ));
                self.can_invite_handler.replace(Some(can_invite_handler));

                let is_direct_handler = room.connect_is_direct_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_invite_action();
                    }
                ));
                let tombstoned_handler = room.connect_is_tombstoned_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_tombstoned_banner();
                    }
                ));
                let successor_id_handler = room.connect_successor_id_string_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_tombstoned_banner();
                    }
                ));
                let successor_handler = room.connect_successor_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_tombstoned_banner();
                    }
                ));

                self.room_handlers.replace(vec![
                    is_direct_handler,
                    tombstoned_handler,
                    successor_id_handler,
                    successor_handler,
                ]);

                let empty_handler = timeline.connect_is_empty_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_view();
                    }
                ));

                let state_handler = timeline.connect_state_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |timeline| {
                        imp.update_view();

                        // Always test if we need to load more when the timeline is ready.
                        // This is mostly to make sure that we load events if the timeline was not
                        // initialized when the room was opened.
                        if timeline.state() == LoadingState::Ready {
                            imp.load_more_events_if_needed();
                        }
                    }
                ));

                self.timeline
                    .set(timeline.clone(), vec![empty_handler, state_handler]);

                timeline.remove_empty_typing_row();
                self.grouping_model().set_model(Some(timeline.items()));

                self.trigger_read_receipts_update();
                self.scroll_down();
            } else {
                self.grouping_model().set_model(None::<gio::ListModel>);
            }

            self.update_view();
            self.load_more_events_if_needed();
            self.update_room_menu();
            self.update_tombstoned_banner();
            self.update_invite_action();

            self.obj().notify_timeline();
        }

        /// The room of the current timeline, if any.
        pub(super) fn room(&self) -> Option<Room> {
            self.timeline.obj().map(|timeline| timeline.room())
        }

        /// The `GroupingListModel` used in the list view.
        fn grouping_model(&self) -> &GroupingListModel {
            self.grouping_model.get_or_init(|| {
                GroupingListModel::new(|lhs, rhs| {
                    lhs.downcast_ref::<Event>()
                        .is_some_and(Event::is_state_group_event)
                        && rhs
                            .downcast_ref::<Event>()
                            .is_some_and(Event::is_state_group_event)
                })
            })
        }

        /// Bind the given `GtkListItem` to its item.
        fn bind_list_item_to_item(&self, list_item: &gtk::ListItem) {
            let Some(item) = list_item.item() else {
                error!("List item does not have an item",);
                list_item.set_child(None::<&gtk::Widget>);
                return;
            };

            if let Some(event) = item.downcast_ref::<Event>() {
                let child = list_item.child_or_else::<EventRow>(|| EventRow::new(&self.obj()));
                child.set_event(Some(event.clone()));
            } else if let Some(virtual_item) = item.downcast_ref::<VirtualItem>() {
                set_virtual_item_child(list_item, virtual_item);
            } else if let Some(group) = item.downcast_ref::<GroupingListGroup>() {
                let child = list_item.child_or_default::<StateGroupRow>();
                child.set_group(Some(group.clone()));
            } else {
                error!("Could not build widget for unsupported room history item: {item:?}");
            }
        }

        /// Handle when the scroll value changed.
        fn scroll_value_changed(&self) {
            let is_at_bottom = self.is_at_bottom();

            if self.is_auto_scrolling.get() && !is_at_bottom {
                // Force to scroll to the very bottom.
                self.scrolled_window
                    .emit_scroll_child(gtk::ScrollType::End, false);
            } else {
                self.set_is_auto_scrolling(false);
                self.set_sticky(is_at_bottom);
                self.update_scroll_btn();

                // Remove the typing row if the user scrolls up.
                if !is_at_bottom {
                    if let Some(timeline) = self.timeline.obj() {
                        timeline.remove_empty_typing_row();
                    }
                }

                self.trigger_read_receipts_update();
                self.load_more_events_if_needed();
            }
        }

        /// Handle when the maximum scroll value changed.
        fn scroll_max_value_changed(&self) {
            if self.is_auto_scrolling.get() {
                // We are handling it.
                return;
            }

            if self.is_sticky.get() {
                self.scroll_down();
            } else {
                self.update_scroll_btn();
            }

            self.load_more_events_if_needed();
        }

        /// Set whether the room history should stick to the newest message in
        /// the timeline.
        pub(super) fn set_sticky(&self, is_sticky: bool) {
            if self.is_sticky.get() == is_sticky {
                return;
            }

            self.is_sticky.set(is_sticky);
            self.obj().notify_is_sticky();
        }

        /// Set whether the current room history scrolling is automatic.
        fn set_is_auto_scrolling(&self, is_auto: bool) {
            if self.is_auto_scrolling.get() == is_auto {
                return;
            }

            self.is_auto_scrolling.set(is_auto);
        }

        /// Scroll to the bottom of the timeline.
        #[template_callback]
        fn scroll_down(&self) {
            if self.is_at_bottom() {
                // Nothing to do.
                return;
            }

            self.set_is_auto_scrolling(true);

            let n_items = self.grouping_model().n_items();

            if n_items > 0 {
                // Wait until the next tick, to make sure that the GtkListView has created the
                // item before focusing it.
                glib::idle_add_local_once(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move || {
                        imp.listview
                            .scroll_to(n_items - 1, gtk::ListScrollFlags::FOCUS, None);
                    }
                ));
            }
        }

        /// Whether the list view is scrolled at the bottom.
        pub(super) fn is_at_bottom(&self) -> bool {
            let adj = self
                .listview
                .vadjustment()
                .expect("GtkListView has a vadjustment");
            (adj.value() + adj.page_size() - adj.upper()).abs() < 0.0001
        }

        /// Update the visibility of the scroll button.
        fn update_scroll_btn(&self) {
            let is_at_bottom = self.is_at_bottom();

            if !is_at_bottom {
                // Show the revealer so we can reveal the button.
                self.scroll_btn_revealer.set_visible(true);
            }

            self.scroll_btn_revealer.set_reveal_child(!is_at_bottom);
        }

        /// Update the room menu for the current state.
        fn update_room_menu(&self) {
            let Some(room) = self.room() else {
                self.room_menu.set_visible(false);
                return;
            };

            let obj = self.obj();
            let membership = room.own_member().membership();
            obj.action_set_enabled("room-history.leave", membership == Membership::Join);
            obj.action_set_enabled(
                "room-history.join",
                membership == Membership::Leave && room.join_rule().we_can_join(),
            );
            obj.action_set_enabled(
                "room-history.forget",
                matches!(membership, Membership::Leave | Membership::Ban),
            );

            self.room_menu.set_visible(true);
        }

        /// Update the view for the current state.
        fn update_view(&self) {
            let Some(timeline) = self.timeline.obj() else {
                return;
            };

            let visible_child_name = if timeline.is_empty() {
                if timeline.state() == LoadingState::Error {
                    "error"
                } else {
                    "loading"
                }
            } else {
                "content"
            };
            self.stack.set_visible_child_name(visible_child_name);
        }

        /// Whether we need to load more events at the start of the timeline.
        fn needs_more_events_at_the_start(&self) -> bool {
            if self.grouping_model().n_items() == 0 {
                // We definitely want events if the history is empty.
                return true;
            }

            // Load more messages when the user gets close to the top of the known room
            // history. Use the page size twice to detect if the user gets close to
            // the top.
            let adj = self
                .listview
                .vadjustment()
                .expect("GtkListView has a vadjustment");
            adj.value() < adj.page_size() * 2.0
        }

        /// Load more events in the history if needed.
        fn load_more_events_if_needed(&self) {
            if self.needs_more_events_at_the_start() {
                self.load_more_events_at_the_start();
            }
        }

        /// Load more events at the beginning of the history.
        fn load_more_events_at_the_start(&self) {
            let Some(timeline) = self.timeline.obj() else {
                return;
            };

            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    timeline
                        .paginate_backwards(clone!(
                            #[weak]
                            imp,
                            #[upgrade_or]
                            ControlFlow::Break(()),
                            move || {
                                if imp.needs_more_events_at_the_start() {
                                    ControlFlow::Continue(())
                                } else {
                                    ControlFlow::Break(())
                                }
                            }
                        ))
                        .await;
                }
            ));
        }

        /// Load more events in the history, regardless of if we need them.
        ///
        /// This should only be used to try to fix timeline loading errors.
        #[template_callback]
        fn load_more_events(&self) {
            self.load_more_events_at_the_start();
        }

        /// Scroll to the event with the given identifier.
        fn scroll_to_event(&self, key: &TimelineEventItemId) {
            let Some(timeline) = self.timeline.obj() else {
                return;
            };

            if let Some(pos) = timeline.find_event_position(key) {
                let pos = pos as u32;
                self.listview
                    .scroll_to(pos, gtk::ListScrollFlags::FOCUS, None);
            }
        }

        /// Trigger the process to update read receipts.
        fn trigger_read_receipts_update(&self) {
            let Some(timeline) = self.timeline.obj() else {
                return;
            };

            if !timeline.is_empty() {
                if let Some(source_id) = self.scroll_timeout.take() {
                    source_id.remove();
                }
                if let Some(source_id) = self.read_timeout.take() {
                    source_id.remove();
                }

                // Only send read receipt when scrolling stopped.
                self.scroll_timeout
                    .replace(Some(glib::timeout_add_local_once(
                        SCROLL_TIMEOUT,
                        clone!(
                            #[weak(rename_to = imp)]
                            self,
                            move || {
                                imp.update_read_receipts();
                            }
                        ),
                    )));
            }
        }

        /// Update the read receipts.
        fn update_read_receipts(&self) {
            self.scroll_timeout.take();

            if let Some(source_id) = self.read_timeout.take() {
                source_id.remove();
            }

            self.read_timeout.replace(Some(glib::timeout_add_local_once(
                READ_TIMEOUT,
                clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move || {
                        imp.update_read_marker();
                    }
                ),
            )));

            let Some(position) = self.receipt_position() else {
                return;
            };

            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    let Some(room) = imp.room() else { return };
                    room.send_receipt(ReceiptType::Read, position).await;
                }
            ));
        }

        /// Update the read marker.
        fn update_read_marker(&self) {
            self.read_timeout.take();

            let Some(position) = self.receipt_position() else {
                return;
            };

            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    let Some(room) = imp.room() else { return };
                    room.send_receipt(ReceiptType::FullyRead, position).await;
                }
            ));
        }

        /// The position where a receipt should point to.
        fn receipt_position(&self) -> Option<ReceiptPosition> {
            let position = if self.is_at_bottom() {
                ReceiptPosition::End
            } else {
                ReceiptPosition::Event(self.last_visible_event_id()?)
            };

            Some(position)
        }

        /// Get the ID of the last visible event in the room history.
        fn last_visible_event_id(&self) -> Option<OwnedEventId> {
            let listview = &*self.listview;
            let mut child = listview.last_child();
            // The visible part of the listview spans between 0 and max.
            let max = listview.height() as f32;

            while let Some(item) = child {
                // Vertical position of the top of the item.
                let top_pos = item
                    .compute_point(listview, &Point::new(0.0, 0.0))
                    .unwrap()
                    .y();
                // Vertical position of the bottom of the item.
                let bottom_pos = item
                    .compute_point(listview, &Point::new(0.0, item.height() as f32))
                    .unwrap()
                    .y();

                let top_in_view = top_pos > 0.0 && top_pos <= max;
                let bottom_in_view = bottom_pos > 0.0 && bottom_pos <= max;
                // If a message is too big and takes more space than the current view.
                let content_in_view = top_pos <= max && bottom_pos > 0.0;
                if top_in_view || bottom_in_view || content_in_view {
                    if let Some(event_id) = item
                        .first_child()
                        .and_downcast::<EventRow>()
                        .and_then(|row| row.event())
                        .and_then(|event| event.event_id())
                    {
                        return Some(event_id);
                    }
                }

                child = item.prev_sibling();
            }

            None
        }

        /// Update the tombstoned banner according to the state of the current
        /// room.
        fn update_tombstoned_banner(&self) {
            let banner = &self.tombstoned_banner;

            let Some(room) = self.room() else {
                banner.set_revealed(false);
                return;
            };

            if !room.is_tombstoned() {
                banner.set_revealed(false);
                return;
            }

            if room.successor().is_some() {
                banner.set_title(&gettext("There is a newer version of this room"));
                // Translators: This is a verb, as in 'View Room'.
                banner.set_button_label(Some(&gettext("View")));
            } else if room.successor_id().is_some() {
                banner.set_title(&gettext("There is a newer version of this room"));
                banner.set_button_label(Some(&gettext("Join")));
            } else {
                banner.set_title(&gettext("This room was closed"));
                banner.set_button_label(None);
            }

            banner.set_revealed(true);
        }

        /// Leave the room.
        async fn leave(&self) {
            let Some(room) = self.room() else {
                return;
            };

            if confirm_leave_room_dialog(&room, &*self.obj())
                .await
                .is_none()
            {
                return;
            }

            if room
                .change_category(TargetRoomCategory::Left)
                .await
                .is_err()
            {
                toast!(
                    self.obj(),
                    gettext(
                        // Translators: Do NOT translate the content between '{' and '}', this is a variable name.
                        "Could not leave {room}",
                    ),
                    @room,
                );
            }
        }

        /// Join the room.
        async fn join(&self) {
            let Some(room) = self.room() else {
                return;
            };

            if room
                .change_category(TargetRoomCategory::Normal)
                .await
                .is_err()
            {
                toast!(
                    self.obj(),
                    gettext(
                        // Translators: Do NOT translate the content between '{' and '}', this is a
                        // variable name.
                        "Could not join {room}",
                    ),
                    @room,
                );
            }
        }

        /// Forget the room.
        async fn forget(&self) {
            let Some(room) = self.room() else {
                return;
            };

            if room.forget().await.is_err() {
                toast!(
                    self.obj(),
                    // Translators: Do NOT translate the content between '{' and '}', this is a variable name.
                    gettext("Could not forget {room}"),
                    @room,
                );
            }
        }

        // Update the invite action according to the current state.
        fn update_invite_action(&self) {
            let Some(room) = self.room() else {
                return;
            };

            // Enable the invite action when we can invite but it is not a direct room.
            let can_invite = !room.is_direct() && room.permissions().can_invite();

            self.obj()
                .action_set_enabled("room-history.invite-members", can_invite);
        }

        /// Join or view the successor of the room, if possible.
        #[template_callback]
        async fn join_or_view_successor(&self) {
            let Some(room) = self.room() else {
                return;
            };
            let Some(session) = room.session() else {
                return;
            };

            if !room.is_joined() || !room.is_tombstoned() {
                return;
            }
            let obj = self.obj();

            if let Some(successor) = room.successor() {
                let Some(window) = obj.root().and_downcast::<Window>() else {
                    return;
                };

                window.session_view().select_room(successor);
            } else if let Some(successor_id) = room.successor_id().map(ToOwned::to_owned) {
                let via = successor_id
                    .server_name()
                    .map(ToOwned::to_owned)
                    .into_iter()
                    .collect();

                if let Err(error) = session
                    .room_list()
                    .join_by_id_or_alias(successor_id.into(), via)
                    .await
                {
                    toast!(obj, error);
                }
            }
        }

        /// The context menu for rows presenting an [`Event`].
        pub(super) fn event_context_menu(&self) -> &EventActionsContextMenu {
            self.event_context_menu.get_or_init(Default::default)
        }

        /// The context menu for the sender avatars.
        pub(super) fn sender_context_menu(&self) -> &gtk::PopoverMenu {
            self.sender_context_menu.get_or_init(|| {
                let popover = gtk::PopoverMenu::builder()
                    .has_arrow(false)
                    .halign(gtk::Align::Start)
                    .menu_model(&*self.sender_menu_model)
                    .build();
                popover.update_property(&[gtk::accessible::Property::Label(&gettext(
                    "Sender Context Menu",
                ))]);
                popover
            })
        }
    }
}

glib::wrapper! {
    /// A view that displays the timeline of a room and ways to send new messages.
    pub struct RoomHistory(ObjectSubclass<imp::RoomHistory>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl RoomHistory {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// The header bar of the room history.
    pub(crate) fn header_bar(&self) -> &adw::HeaderBar {
        &self.imp().header_bar
    }

    /// The message toolbar of the room history.
    pub(super) fn message_toolbar(&self) -> &MessageToolbar {
        &self.imp().message_toolbar
    }

    /// Opens the room details.
    ///
    /// If `subpage_name` is set, the room details will be opened on the given
    /// subpage.
    pub(crate) fn open_room_details(&self, subpage_name: Option<room_details::SubpageName>) {
        let Some(room) = self.imp().room() else {
            return;
        };

        let window = RoomDetails::new(self.root().and_downcast_ref(), &room);
        if let Some(subpage_name) = subpage_name {
            window.show_initial_subpage(subpage_name);
        }
        window.present();
    }

    /// Enable or disable the mode allowing the room history to stick to the
    /// bottom based on scrollbar position.
    pub(crate) fn enable_sticky_mode(&self, enable: bool) {
        let imp = self.imp();
        if enable {
            imp.set_sticky(imp.is_at_bottom());
        } else {
            imp.set_sticky(false);
        }
    }

    /// Handle a paste action.
    pub(crate) fn handle_paste_action(&self) {
        self.imp().message_toolbar.handle_paste_action();
    }

    /// The context menu for rows presenting an [`Event`].
    fn event_context_menu(&self) -> &EventActionsContextMenu {
        self.imp().event_context_menu()
    }

    /// The context menu for the sender avatars.
    fn sender_context_menu(&self) -> &gtk::PopoverMenu {
        self.imp().sender_context_menu()
    }
}

/// Set the proper child of the given `GtkListItem` for the given
/// [`VirtualItem`].
///
/// Constructs or reuses the child widget as necessary.
fn set_virtual_item_child(list_item: &gtk::ListItem, virtual_item: &VirtualItem) {
    let kind = &virtual_item.kind();

    match kind {
        VirtualItemKind::Spinner => {
            if !list_item
                .child()
                .is_some_and(|widget| widget.is::<adw::Spinner>())
            {
                let spinner = adw::Spinner::builder()
                    .margin_top(12)
                    .margin_bottom(12)
                    .height_request(24)
                    .width_request(24)
                    .build();
                spinner.add_css_class("room-history-row");
                spinner.set_accessible_role(gtk::AccessibleRole::ListItem);
                list_item.set_child(Some(&spinner));
            }
        }
        VirtualItemKind::Typing => {
            let child = list_item.child_or_default::<TypingRow>();
            let typing_list = virtual_item.room().typing_list();
            child.set_list(Some(typing_list));
        }
        VirtualItemKind::TimelineStart
        | VirtualItemKind::DayDivider(_)
        | VirtualItemKind::NewMessages => {
            let divider = list_item.child_or_default::<DividerRow>();
            divider.set_virtual_item(Some(virtual_item));
        }
    }
}
