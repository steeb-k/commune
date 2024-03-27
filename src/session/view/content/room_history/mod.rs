mod divider_row;
mod item_row;
mod member_timestamp;
mod message_row;
mod message_toolbar;
mod read_receipts_list;
mod sender_avatar;
mod state_row;
mod typing_row;
mod verification_info_bar;

use std::time::Duration;

use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{gdk, gio, glib, glib::clone, graphene::Point, CompositeTemplate};
use matrix_sdk::ruma::EventId;
use ruma::{
    api::client::receipt::create_receipt::v3::ReceiptType, events::receipt::ReceiptThread,
    OwnedEventId,
};
use tracing::{error, warn};

use self::{
    divider_row::DividerRow, item_row::ItemRow, message_row::MessageRow,
    message_toolbar::MessageToolbar, read_receipts_list::ReadReceiptsList,
    sender_avatar::SenderAvatar, state_row::StateRow, typing_row::TypingRow,
    verification_info_bar::VerificationInfoBar,
};
use super::{room_details, RoomDetails};
use crate::{
    components::{DragOverlay, ReactionChooser, RoomTitle, Spinner},
    i18n::gettext_f,
    prelude::*,
    session::model::{
        Event, EventKey, MemberList, Membership, Room, RoomType, Timeline, TimelineState,
    },
    spawn, spawn_tokio, toast,
    utils::{message_dialog, template_callbacks::TemplateCallbacks, BoundObject},
    Window,
};

/// The time to wait before considering that scrolling has ended.
const SCROLL_TIMEOUT: Duration = Duration::from_millis(500);
/// The time to wait before considering that messages on a screen where read.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

mod imp {
    use std::{
        cell::{Cell, OnceCell, RefCell},
        marker::PhantomData,
    };

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session/view/content/room_history/mod.ui")]
    #[properties(wrapper_type = super::RoomHistory)]
    pub struct RoomHistory {
        /// The room currently displayed.
        #[property(get, set = Self::set_room, explicit_notify, nullable)]
        pub room: BoundObject<Room>,
        /// Whether this is the only view visible, i.e. there is no sidebar.
        #[property(get, set)]
        pub only_view: Cell<bool>,
        /// Whether this `RoomHistory` is empty, aka no room is currently
        /// displayed.
        #[property(get = Self::empty)]
        empty: PhantomData<bool>,
        pub room_members: RefCell<Option<MemberList>>,
        pub timeline_handlers: RefCell<Vec<glib::SignalHandlerId>>,
        pub is_auto_scrolling: Cell<bool>,
        /// Whether the room history should stick to the newest message in the
        /// timeline.
        #[property(get, set = Self::set_sticky, explicit_notify)]
        pub sticky: Cell<bool>,
        pub item_context_menu: OnceCell<gtk::PopoverMenu>,
        pub item_reaction_chooser: ReactionChooser,
        pub sender_context_menu: OnceCell<gtk::PopoverMenu>,
        #[template_child]
        pub sender_menu_model: TemplateChild<gio::Menu>,
        #[template_child]
        pub room_title: TemplateChild<RoomTitle>,
        #[template_child]
        pub room_menu: TemplateChild<gtk::MenuButton>,
        #[template_child]
        pub listview: TemplateChild<gtk::ListView>,
        #[template_child]
        pub content: TemplateChild<gtk::Widget>,
        #[template_child]
        pub scrolled_window: TemplateChild<gtk::ScrolledWindow>,
        #[template_child]
        pub scroll_btn: TemplateChild<gtk::Button>,
        #[template_child]
        pub scroll_btn_revealer: TemplateChild<gtk::Revealer>,
        #[template_child]
        pub message_toolbar: TemplateChild<MessageToolbar>,
        #[template_child]
        pub loading: TemplateChild<Spinner>,
        #[template_child]
        pub error: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub tombstoned_banner: TemplateChild<adw::Banner>,
        pub is_loading: Cell<bool>,
        #[template_child]
        pub drag_overlay: TemplateChild<DragOverlay>,
        pub scroll_timeout: RefCell<Option<glib::SourceId>>,
        pub read_timeout: RefCell<Option<glib::SourceId>>,
        /// The GtkSelectionModel used in the listview.
        // TODO: use gtk::MultiSelection to allow selection
        pub selection_model: OnceCell<gtk::NoSelection>,
        pub can_invite_handler: RefCell<Option<glib::SignalHandlerId>>,
        pub membership_handler: RefCell<Option<glib::SignalHandlerId>>,
        pub join_rule_handler: RefCell<Option<glib::SignalHandlerId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RoomHistory {
        const NAME: &'static str = "ContentRoomHistory";
        type Type = super::RoomHistory;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            ItemRow::ensure_type();
            VerificationInfoBar::ensure_type();
            Timeline::ensure_type();

            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);
            TemplateCallbacks::bind_template_callbacks(klass);

            klass.set_accessible_role(gtk::AccessibleRole::Group);

            klass.install_action_async("room-history.leave", None, |obj, _, _| async move {
                obj.leave().await;
            });
            klass.install_action_async("room-history.join", None, |obj, _, _| async move {
                obj.join().await;
            });
            klass.install_action_async("room-history.forget", None, |obj, _, _| async move {
                obj.forget().await;
            });

            klass.install_action("room-history.try-again", None, |obj, _, _| {
                obj.try_again();
            });

            klass.install_action_async("room-history.permalink", None, |obj, _, _| async move {
                obj.permalink().await;
            });

            klass.install_action("room-history.details", None, |obj, _, _| {
                obj.open_room_details(None);
            });
            klass.install_action("room-history.invite-members", None, |obj, _, _| {
                obj.open_room_details(Some(room_details::SubpageName::Invite));
            });

            klass.install_action("room-history.scroll-down", None, |obj, _, _| {
                obj.scroll_down();
            });
            klass.install_action(
                "room-history.scroll-to-event",
                Some(&EventKey::static_variant_type()),
                |obj, _, v| {
                    if let Some(event_key) = v.and_then(EventKey::from_variant) {
                        obj.scroll_to_event(&event_key);
                    }
                },
            );

            klass.install_action(
                "room-history.reply",
                Some(&String::static_variant_type()),
                |obj, _, v| {
                    if let Some(event_id) = v
                        .and_then(String::from_variant)
                        .and_then(|s| EventId::parse(s).ok())
                    {
                        if let Some(event) = obj
                            .room()
                            .and_then(|room| {
                                room.timeline().event_by_key(&EventKey::EventId(event_id))
                            })
                            .and_downcast()
                        {
                            obj.message_toolbar().set_reply_to(event);
                        }
                    }
                },
            );

            klass.install_action(
                "room-history.edit",
                Some(&String::static_variant_type()),
                |obj, _, v| {
                    if let Some(event_id) = v
                        .and_then(String::from_variant)
                        .and_then(|s| EventId::parse(s).ok())
                    {
                        if let Some(event) = obj
                            .room()
                            .and_then(|room| {
                                room.timeline().event_by_key(&EventKey::EventId(event_id))
                            })
                            .and_downcast()
                        {
                            obj.message_toolbar().set_edit(event);
                        }
                    }
                },
            );
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for RoomHistory {
        fn constructed(&self) {
            self.setup_listview();
            self.setup_drop_target();

            self.scroll_btn_revealer
                .connect_child_revealed_notify(|revealer| {
                    // Hide the revealer when we don't want to show the child and the animation is
                    // finished.
                    if !revealer.reveals_child() && !revealer.is_child_revealed() {
                        revealer.set_visible(false);
                    }
                });

            self.parent_constructed();
        }

        fn dispose(&self) {
            self.disconnect_all();
        }
    }

    impl WidgetImpl for RoomHistory {}
    impl BinImpl for RoomHistory {}

    impl RoomHistory {
        fn setup_listview(&self) {
            let obj = self.obj();

            let factory = gtk::SignalListItemFactory::new();
            factory.connect_setup(clone!(@weak obj => move |_, item| {
                let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                    error!("List item factory did not receive a list item: {item:?}");
                    return;
                };
                let row = ItemRow::new(&obj);
                item.set_child(Some(&row));
                item.bind_property("item", &row, "item").build();
                item.set_activatable(false);
                item.set_selectable(false);
            }));
            self.listview.set_factory(Some(&factory));

            // Needed to use the natural height of GtkPictures
            self.listview
                .set_vscroll_policy(gtk::ScrollablePolicy::Natural);

            self.listview.set_model(Some(obj.selection_model()));

            obj.set_sticky(true);
            let adj = self.listview.vadjustment().unwrap();

            adj.connect_value_changed(clone!(@weak obj => move |_| {
                let imp = obj.imp();

                obj.trigger_read_receipts_update();

                let is_at_bottom = obj.is_at_bottom();
                if imp.is_auto_scrolling.get() {
                    if is_at_bottom {
                        imp.is_auto_scrolling.set(false);
                        obj.set_sticky(true);
                    } else {
                        obj.scroll_down();
                    }
                } else {
                    obj.set_sticky(is_at_bottom);
                }

                // Remove the typing row if we scroll up.
                if !is_at_bottom {
                    if let Some(room) = obj.room() {
                        room.timeline().remove_empty_typing_row();
                    }
                }

                obj.start_loading();
            }));
            adj.connect_upper_notify(clone!(@weak obj => move |_| {
                if obj.sticky() {
                    obj.scroll_down();
                }
                obj.start_loading();
            }));
            adj.connect_page_size_notify(clone!(@weak obj => move |_| {
                if obj.sticky() {
                    obj.scroll_down();
                }
                obj.start_loading();
            }));
        }

        fn setup_drop_target(&self) {
            let obj = self.obj();

            let target = gtk::DropTarget::new(
                gio::File::static_type(),
                gdk::DragAction::COPY | gdk::DragAction::MOVE,
            );

            target.connect_drop(
                clone!(@weak obj => @default-return false, move |_, value, _, _| {
                    match value.get::<gio::File>() {
                        Ok(file) => {
                            spawn!(async move {
                                obj.message_toolbar().send_file(file).await;
                            });
                            true
                        }
                        Err(error) => {
                            warn!("Could not get file from drop: {error:?}");
                            toast!(
                                obj,
                                gettext("Error getting file from drop")
                            );

                            false
                        }
                    }
                }),
            );

            self.drag_overlay.set_drop_target(target);
        }
    }

    impl RoomHistory {
        fn disconnect_all(&self) {
            if let Some(room) = self.room.obj() {
                for handler in self.timeline_handlers.take() {
                    room.timeline().disconnect(handler);
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

            self.room.disconnect_signals();
        }

        /// Set the room currently displayed.
        fn set_room(&self, room: Option<Room>) {
            if self.room.obj() == room {
                return;
            }
            let obj = self.obj();

            self.disconnect_all();

            if let Some(source_id) = self.scroll_timeout.take() {
                source_id.remove();
            }
            if let Some(source_id) = self.read_timeout.take() {
                source_id.remove();
            }

            if let Some(room) = room {
                let timeline = room.timeline();

                // Keep a strong reference to the members list before changing the model, so all
                // events use the same list.
                self.room_members
                    .replace(Some(room.get_or_create_members()));

                let membership_handler =
                    room.own_member()
                        .connect_membership_notify(clone!(@weak obj => move |_| {
                            obj.update_room_menu();
                        }));
                self.membership_handler.replace(Some(membership_handler));

                let join_rule_handler =
                    room.join_rule()
                        .connect_we_can_join_notify(clone!(@weak obj => move |_| {
                            obj.update_room_menu();
                        }));
                self.join_rule_handler.replace(Some(join_rule_handler));

                let tombstoned_handler =
                    room.connect_is_tombstoned_notify(clone!(@weak obj => move |_| {
                        obj.update_tombstoned_banner();
                    }));

                let successor_handler =
                    room.connect_successor_id_string_notify(clone!(@weak obj => move |_| {
                        obj.update_tombstoned_banner();
                    }));

                let successor_room_handler =
                    room.connect_successor_notify(clone!(@weak obj => move |_| {
                        obj.update_tombstoned_banner();
                    }));

                self.room.set(
                    room,
                    vec![
                        tombstoned_handler,
                        successor_handler,
                        successor_room_handler,
                    ],
                );

                let empty_handler = timeline.connect_empty_notify(clone!(@weak obj => move |_| {
                    obj.update_view();
                }));

                let state_handler =
                    timeline.connect_state_notify(clone!(@weak obj => move |timeline| {
                        obj.update_view();

                        // Always test if we need to load more when timeline is ready.
                        if timeline.state() == TimelineState::Ready {
                            obj.start_loading();
                        }
                    }));

                self.timeline_handlers
                    .replace(vec![empty_handler, state_handler]);

                timeline.remove_empty_typing_row();
                obj.selection_model().set_model(Some(&timeline.items()));

                obj.trigger_read_receipts_update();
                obj.init_invite_action();
                obj.scroll_down();
            } else {
                obj.selection_model().set_model(None::<&gio::ListModel>);
            }

            self.is_loading.set(false);
            obj.update_view();
            obj.start_loading();
            obj.update_room_menu();
            obj.update_tombstoned_banner();

            obj.notify_room();
            obj.notify_empty();
        }

        /// Whether this `RoomHistory` is empty, aka no room is currently
        /// displayed.
        fn empty(&self) -> bool {
            self.room.obj().is_none()
        }

        /// Set whether the room history should stick to the newest message in
        /// the timeline.
        fn set_sticky(&self, sticky: bool) {
            if self.sticky.get() == sticky {
                return;
            }

            if !sticky {
                self.scroll_btn_revealer.set_visible(true);
            }
            self.scroll_btn_revealer.set_reveal_child(!sticky);

            self.sticky.set(sticky);
            self.obj().notify_sticky();
        }
    }
}

glib::wrapper! {
    /// A view that displays the timeline of a room and ways to send new messages.
    pub struct RoomHistory(ObjectSubclass<imp::RoomHistory>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl RoomHistory {
    pub fn new() -> Self {
        glib::Object::new()
    }

    fn message_toolbar(&self) -> &MessageToolbar {
        &self.imp().message_toolbar
    }

    /// The members of the room currently displayed.
    pub fn room_members(&self) -> Option<MemberList> {
        self.imp().room_members.borrow().clone()
    }

    fn selection_model(&self) -> &gtk::NoSelection {
        self.imp()
            .selection_model
            .get_or_init(|| gtk::NoSelection::new(gio::ListModel::NONE.cloned()))
    }

    /// Leave the room.
    async fn leave(&self) {
        let Some(room) = self.room() else {
            return;
        };

        if !message_dialog::confirm_leave_room(&room, self).await {
            return;
        }

        if room.set_category(RoomType::Left).await.is_err() {
            toast!(
                self,
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

        if room.set_category(RoomType::Normal).await.is_err() {
            toast!(
                self,
                gettext_f(
                    // Translators: Do NOT translate the content between '{' and '}', this is a
                    // variable name.
                    "Could not join room {room_name}. Try again later.",
                    &[("room_name", &room.display_name())],
                )
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
                self,
                // Translators: Do NOT translate the content between '{' and '}', this is a variable name.
                gettext("Could not forget {room}"),
                @room,
            );
        }
    }

    pub async fn permalink(&self) {
        let Some(room) = self.room() else {
            return;
        };

        let permalink = room.matrix_to_uri().await;
        self.clipboard().set_text(&permalink.to_string());
        toast!(self, gettext("Permalink copied to clipboard"));
    }

    fn init_invite_action(&self) {
        let Some(room) = self.room() else {
            return;
        };
        let permissions = room.permissions();

        let can_invite_handler =
            permissions.connect_can_invite_notify(clone!(@weak self as obj => move |permissions| {
                obj.action_set_enabled("room-history.invite-members", permissions.can_invite());
            }));
        self.imp()
            .can_invite_handler
            .replace(Some(can_invite_handler));

        self.action_set_enabled("room-history.invite-members", permissions.can_invite());
    }

    /// Opens the room details.
    ///
    /// If `subpage_name` is set, the room details will be opened on the given
    /// subpage.
    pub fn open_room_details(&self, subpage_name: Option<room_details::SubpageName>) {
        let Some(room) = self.room() else {
            return;
        };

        let window = RoomDetails::new(self.root().and_downcast_ref(), &room);
        if let Some(subpage_name) = subpage_name {
            window.show_initial_subpage(subpage_name);
        }
        window.present();
    }

    fn update_room_menu(&self) {
        let imp = self.imp();
        let Some(room) = self.room() else {
            imp.room_menu.set_visible(false);
            return;
        };

        let membership = room.own_member().membership();
        self.action_set_enabled("room-history.leave", membership == Membership::Join);
        self.action_set_enabled(
            "room-history.join",
            membership == Membership::Leave && room.join_rule().we_can_join(),
        );
        self.action_set_enabled(
            "room-history.forget",
            matches!(membership, Membership::Leave | Membership::Ban),
        );

        imp.room_menu.set_visible(true);
    }

    fn update_view(&self) {
        let imp = self.imp();

        if let Some(room) = self.room() {
            if room.timeline().empty() {
                if room.timeline().state() == TimelineState::Error {
                    imp.stack.set_visible_child(&*imp.error);
                } else {
                    imp.stack.set_visible_child(&*imp.loading);
                }
            } else {
                imp.stack.set_visible_child(&*imp.content);
            }
        }
    }

    /// Whether we need to load more messages.
    fn need_messages(&self) -> bool {
        let Some(room) = self.room() else {
            return false;
        };
        let timeline = room.timeline();

        if !timeline.can_load() {
            // We will retry when timeline is ready.
            return false;
        }

        if timeline.empty() {
            // We definitely want messages if the timeline is ready but empty.
            return true;
        };

        // Load more messages when the user gets close to the top of the known room
        // history. Use the page size twice to detect if the user gets close to
        // the top.
        let adj = self.imp().listview.vadjustment().unwrap();
        adj.value() < adj.page_size() * 2.0 || adj.upper() <= adj.page_size() / 2.0
    }

    fn start_loading(&self) {
        let imp = self.imp();

        if imp.is_loading.get() {
            return;
        }

        if !self.need_messages() {
            return;
        }

        let Some(room) = self.room() else {
            return;
        };

        imp.is_loading.set(true);

        let obj_weak = self.downgrade();
        spawn!(glib::Priority::DEFAULT_IDLE, async move {
            room.timeline().load().await;

            // Remove the task
            if let Some(obj) = obj_weak.upgrade() {
                obj.imp().is_loading.set(false);
            }
        });
    }

    /// Scroll to the newest message in the timeline
    pub fn scroll_down(&self) {
        let imp = self.imp();

        imp.is_auto_scrolling.set(true);

        let n_items = self.selection_model().n_items();

        if n_items > 0 {
            imp.listview
                .scroll_to(n_items - 1, gtk::ListScrollFlags::FOCUS, None);
        }
    }

    /// Whether the GtkListView is scrolled at the bottom.
    fn is_at_bottom(&self) -> bool {
        let adj = self.imp().listview.vadjustment().unwrap();
        adj.value() + adj.page_size() == adj.upper()
    }

    /// Set `RoomHistory` to stick to the bottom based on scrollbar position
    pub fn enable_sticky_mode(&self) {
        self.set_sticky(self.is_at_bottom());
    }

    fn try_again(&self) {
        self.start_loading();
    }

    pub fn handle_paste_action(&self) {
        self.message_toolbar().handle_paste_action();
    }

    /// The context menu for the item rows.
    pub fn item_context_menu(&self) -> &gtk::PopoverMenu {
        self.imp().item_context_menu.get_or_init(|| {
            let popover = gtk::PopoverMenu::builder()
                .has_arrow(false)
                .halign(gtk::Align::Start)
                .build();
            popover.update_property(&[gtk::accessible::Property::Label(&gettext("Context Menu"))]);
            popover
        })
    }

    /// The reaction chooser for the item rows.
    pub fn item_reaction_chooser(&self) -> &ReactionChooser {
        &self.imp().item_reaction_chooser
    }

    /// The context menu for the sender avatars.
    pub fn sender_context_menu(&self) -> &gtk::PopoverMenu {
        let imp = self.imp();
        imp.sender_context_menu.get_or_init(|| {
            let popover = gtk::PopoverMenu::builder()
                .has_arrow(false)
                .halign(gtk::Align::Start)
                .menu_model(&*imp.sender_menu_model)
                .build();
            popover.update_property(&[gtk::accessible::Property::Label(&gettext(
                "Sender Context Menu",
            ))]);
            popover
        })
    }

    fn scroll_to_event(&self, key: &EventKey) {
        let room = match self.room() {
            Some(room) => room,
            None => return,
        };

        if let Some(pos) = room.timeline().find_event_position(key) {
            let pos = pos as u32;
            self.imp()
                .listview
                .scroll_to(pos, gtk::ListScrollFlags::FOCUS, None);
        }
    }

    /// Trigger the process to update read receipts.
    fn trigger_read_receipts_update(&self) {
        let Some(room) = self.room() else {
            return;
        };

        let timeline = room.timeline();
        if !timeline.empty() {
            let imp = self.imp();

            if let Some(source_id) = imp.scroll_timeout.take() {
                source_id.remove();
            }
            if let Some(source_id) = imp.read_timeout.take() {
                source_id.remove();
            }

            // Only send read receipt when scrolling stopped.
            imp.scroll_timeout
                .replace(Some(glib::timeout_add_local_once(
                    SCROLL_TIMEOUT,
                    clone!(@weak self as obj => move || {
                        obj.update_read_receipts();
                    }),
                )));
        }
    }

    /// Update the read receipts.
    fn update_read_receipts(&self) {
        let imp = self.imp();
        imp.scroll_timeout.take();

        if let Some(source_id) = imp.read_timeout.take() {
            source_id.remove();
        }

        imp.read_timeout.replace(Some(glib::timeout_add_local_once(
            READ_TIMEOUT,
            clone!(@weak self as obj => move || {
                obj.update_read_marker();
            }),
        )));

        let Some(position) = self.receipt_position() else {
            return;
        };

        spawn!(clone!(@weak self as obj => async move {
            obj.send_receipt(ReceiptType::Read, position).await;
        }));
    }

    /// Update the read marker.
    fn update_read_marker(&self) {
        let imp = self.imp();
        imp.read_timeout.take();

        let Some(position) = self.receipt_position() else {
            return;
        };

        spawn!(clone!(@weak self as obj => async move {
            obj.send_receipt(ReceiptType::FullyRead, position).await;
        }));
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
        let listview = &*self.imp().listview;
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
                    .and_downcast::<ItemRow>()
                    .and_then(|row| row.item())
                    .and_downcast::<Event>()
                    .and_then(|event| event.event_id())
                {
                    return Some(event_id);
                }
            }

            child = item.prev_sibling();
        }

        None
    }

    /// Send the given receipt.
    async fn send_receipt(&self, receipt_type: ReceiptType, position: ReceiptPosition) {
        let Some(room) = self.room() else {
            return;
        };
        let Some(session) = room.session() else {
            return;
        };
        let send_public_receipt = session.settings().public_read_receipts_enabled();

        let receipt_type = match receipt_type {
            ReceiptType::Read if !send_public_receipt => ReceiptType::ReadPrivate,
            t => t,
        };

        let matrix_timeline = room.timeline().matrix_timeline();
        let handle = spawn_tokio!(async move {
            match position {
                ReceiptPosition::End => matrix_timeline.mark_as_read(receipt_type).await,
                ReceiptPosition::Event(event_id) => {
                    matrix_timeline
                        .send_single_receipt(receipt_type, ReceiptThread::Unthreaded, event_id)
                        .await
                }
            }
        });

        if let Err(error) = handle.await.unwrap() {
            error!("Could not send read receipt: {error}");
        }
    }

    /// Update the tombstoned banner according to the state of the current room.
    fn update_tombstoned_banner(&self) {
        let banner = &self.imp().tombstoned_banner;

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

    /// Join or view the room's successor, if possible.
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

        if let Some(successor) = room.successor() {
            let Some(window) = self.root().and_downcast::<Window>() else {
                return;
            };

            window.show_room(session.session_id(), successor.room_id());
        } else if let Some(successor_id) = room.successor_id().map(ToOwned::to_owned) {
            if let Err(error) = session
                .room_list()
                .join_by_id_or_alias(successor_id.into(), vec![])
                .await
            {
                toast!(self, error);
            }
        }
    }
}

/// The position of the receipt to send.
enum ReceiptPosition {
    /// We are at the end of the timeline (bottom of the view).
    End,
    /// We are at the event with the given ID.
    Event(OwnedEventId),
}
