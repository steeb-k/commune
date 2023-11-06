mod divider_row;
mod item_row;
mod message_row;
mod message_toolbar;
mod read_receipts_list;
mod state_row;
mod typing_row;
mod verification_info_bar;

use std::time::Duration;

use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{
    gdk, gio,
    glib::{self, clone, FromVariant},
    CompositeTemplate,
};
use matrix_sdk::ruma::EventId;
use ruma::{
    api::client::receipt::create_receipt::v3::ReceiptType,
    events::{receipt::ReceiptThread, room::power_levels::PowerLevelAction},
    OwnedEventId,
};
use tracing::{error, warn};

use self::{
    divider_row::DividerRow, item_row::ItemRow, message_row::MessageRow,
    message_toolbar::MessageToolbar, read_receipts_list::ReadReceiptsList, state_row::StateRow,
    typing_row::TypingRow, verification_info_bar::VerificationInfoBar,
};
use super::{room_details, RoomDetails};
use crate::{
    components::{DragOverlay, ReactionChooser, RoomTitle, Spinner},
    session::model::{Event, EventKey, MemberList, Room, RoomType, Timeline, TimelineState},
    spawn, spawn_tokio, toast,
    utils::{message_dialog, template_callbacks::TemplateCallbacks},
    Window,
};

/// The time to wait before considering that scrolling has ended.
const SCROLL_TIMEOUT: Duration = Duration::from_millis(500);
/// The time to wait before considering that messages on a screen where read.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

mod imp {
    use std::{
        cell::{Cell, RefCell},
        collections::HashMap,
    };

    use glib::{signal::SignalHandlerId, subclass::InitializingObject};
    use once_cell::unsync::OnceCell;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/Fractal/ui/session/view/content/room_history/mod.ui")]
    pub struct RoomHistory {
        pub room: RefCell<Option<Room>>,
        /// Whether this is the only view visible, i.e. there is no sidebar.
        pub only_view: Cell<bool>,
        pub room_members: RefCell<Option<MemberList>>,
        pub room_handlers: RefCell<Vec<SignalHandlerId>>,
        pub timeline_handlers: RefCell<Vec<SignalHandlerId>>,
        pub is_auto_scrolling: Cell<bool>,
        pub sticky: Cell<bool>,
        pub item_context_menu: OnceCell<gtk::PopoverMenu>,
        pub item_reaction_chooser: ReactionChooser,
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
        pub room_expr_watches: RefCell<HashMap<&'static str, gtk::ExpressionWatch>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RoomHistory {
        const NAME: &'static str = "ContentRoomHistory";
        type Type = super::RoomHistory;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            ItemRow::static_type();
            VerificationInfoBar::static_type();
            Timeline::static_type();

            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);
            TemplateCallbacks::bind_template_callbacks(klass);

            klass.set_accessible_role(gtk::AccessibleRole::Group);

            klass.install_action("room-history.leave", None, move |obj, _, _| {
                spawn!(clone!(@weak obj => async move {
                    obj.leave().await;
                }));
            });

            klass.install_action("room-history.try-again", None, move |widget, _, _| {
                widget.try_again();
            });

            klass.install_action("room-history.permalink", None, move |widget, _, _| {
                spawn!(clone!(@weak widget => async move {
                    widget.permalink().await;
                }));
            });

            klass.install_action("room-history.details", None, move |widget, _, _| {
                widget.open_room_details(None);
            });
            klass.install_action("room-history.invite-members", None, move |widget, _, _| {
                widget.open_room_details(Some(room_details::SubpageName::Invite));
            });

            klass.install_action("room-history.scroll-down", None, move |widget, _, _| {
                widget.scroll_down();
            });
            klass.install_action(
                "room-history.scroll-to-event",
                Some(EventKey::static_variant_type().as_str()),
                move |widget, _, v| {
                    if let Some(event_key) = v.and_then(EventKey::from_variant) {
                        widget.scroll_to_event(&event_key);
                    }
                },
            );

            klass.install_action("room-history.reply", Some("s"), move |widget, _, v| {
                if let Some(event_id) = v
                    .and_then(String::from_variant)
                    .and_then(|s| EventId::parse(s).ok())
                {
                    if let Some(event) = widget
                        .room()
                        .and_then(|room| room.timeline().event_by_key(&EventKey::EventId(event_id)))
                        .and_downcast()
                    {
                        widget.message_toolbar().set_reply_to(event);
                    }
                }
            });

            klass.install_action("room-history.edit", Some("s"), move |widget, _, v| {
                if let Some(event_id) = v
                    .and_then(String::from_variant)
                    .and_then(|s| EventId::parse(s).ok())
                {
                    if let Some(event) = widget
                        .room()
                        .and_then(|room| room.timeline().event_by_key(&EventKey::EventId(event_id)))
                        .and_downcast()
                    {
                        widget.message_toolbar().set_edit(event);
                    }
                }
            });
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for RoomHistory {
        fn properties() -> &'static [glib::ParamSpec] {
            use once_cell::sync::Lazy;
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpecObject::builder::<Room>("room")
                        .explicit_notify()
                        .build(),
                    glib::ParamSpecBoolean::builder("only-view").build(),
                    glib::ParamSpecBoolean::builder("empty")
                        .explicit_notify()
                        .build(),
                    glib::ParamSpecBoolean::builder("sticky")
                        .explicit_notify()
                        .build(),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            let obj = self.obj();

            match pspec.name() {
                "room" => obj.set_room(value.get().unwrap()),
                "only-view" => self.only_view.set(value.get().unwrap()),
                "sticky" => obj.set_sticky(value.get().unwrap()),
                _ => unimplemented!(),
            }
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            let obj = self.obj();

            match pspec.name() {
                "room" => obj.room().to_value(),
                "only-view" => self.only_view.get().to_value(),
                "empty" => obj.is_empty().to_value(),
                "sticky" => obj.sticky().to_value(),
                _ => unimplemented!(),
            }
        }

        fn constructed(&self) {
            self.setup_listview();
            self.setup_drop_target();

            self.parent_constructed();
        }

        fn dispose(&self) {
            if let Some(room) = self.room.take() {
                for handler in self.room_handlers.take() {
                    room.disconnect(handler);
                }

                for handler in self.timeline_handlers.take() {
                    room.timeline().disconnect(handler);
                }
            }

            for (_, expr_watch) in self.room_expr_watches.take() {
                expr_watch.unwatch();
            }
        }
    }

    impl WidgetImpl for RoomHistory {}
    impl BinImpl for RoomHistory {}

    impl RoomHistory {
        fn setup_listview(&self) {
            let obj = self.obj();

            let factory = gtk::SignalListItemFactory::new();
            factory.connect_setup(clone!(@weak obj => move |_, item| {
                let item = match item.downcast_ref::<gtk::ListItem>() {
                    Some(item) => item,
                    None => {
                        error!("List item factory did not receive a list item: {item:?}");
                        return;
                    }
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

            adj.connect_value_changed(clone!(@weak obj => move |adj| {
                let imp = obj.imp();

                obj.trigger_read_receipts_update();

                let is_at_bottom = adj.value() + adj.page_size() == adj.upper();
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
                            spawn!(clone!(@weak obj => async move {
                                obj.message_toolbar().send_file(file).await;
                            }));
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
}

glib::wrapper! {
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

    /// Set the room currently displayed.
    pub fn set_room(&self, room: Option<Room>) {
        let imp = self.imp();

        if self.room() == room {
            return;
        }

        if let Some(room) = self.room() {
            for handler in imp.room_handlers.take() {
                room.disconnect(handler);
            }

            for handler in imp.timeline_handlers.take() {
                room.timeline().disconnect(handler);
            }

            for (_, expr_watch) in imp.room_expr_watches.take() {
                expr_watch.unwatch();
            }
        }

        if let Some(source_id) = imp.scroll_timeout.take() {
            source_id.remove();
        }
        if let Some(source_id) = imp.read_timeout.take() {
            source_id.remove();
        }

        if let Some(ref room) = room {
            let timeline = room.timeline();

            let category_handler = room.connect_notify_local(
                Some("category"),
                clone!(@weak self as obj => move |_, _| {
                    obj.update_room_state();
                }),
            );

            let tombstoned_handler = room.connect_notify_local(
                Some("tombstoned"),
                clone!(@weak self as obj => move |_, _| {
                    obj.update_tombstoned_banner();
                }),
            );

            let successor_handler = room.connect_notify_local(
                Some("successor"),
                clone!(@weak self as obj => move |_, _| {
                    obj.update_tombstoned_banner();
                }),
            );

            let successor_room_handler = room.connect_notify_local(
                Some("successor-room"),
                clone!(@weak self as obj => move |_, _| {
                    obj.update_tombstoned_banner();
                }),
            );

            imp.room_handlers.replace(vec![
                category_handler,
                tombstoned_handler,
                successor_handler,
                successor_room_handler,
            ]);

            let empty_handler = timeline.connect_notify_local(
                Some("empty"),
                clone!(@weak self as obj => move |_, _| {
                    obj.update_view();
                }),
            );

            let state_handler = timeline.connect_notify_local(
                Some("state"),
                clone!(@weak self as obj => move |timeline, _| {
                    obj.update_view();

                    // Always test if we need to load more when timeline is ready.
                    if timeline.state() == TimelineState::Ready {
                        obj.start_loading();
                    }
                }),
            );

            imp.timeline_handlers
                .replace(vec![empty_handler, state_handler]);

            timeline.remove_empty_typing_row();
            self.trigger_read_receipts_update();

            self.init_invite_action(room);
            self.scroll_down();
        }

        // Keep a strong reference to the members list before changing the model, so all
        // events use the same list.
        imp.room_members
            .replace(room.as_ref().map(|r| r.get_or_create_members()));

        let model = room.as_ref().map(|room| room.timeline().items());
        self.selection_model().set_model(model);

        imp.is_loading.set(false);
        imp.room.replace(room);
        self.update_view();
        self.start_loading();
        self.update_room_state();
        self.update_tombstoned_banner();

        self.notify("room");
        self.notify("empty");
    }

    /// The room currently displayed.
    pub fn room(&self) -> Option<Room> {
        self.imp().room.borrow().clone()
    }

    /// The members of the room currently displayed.
    pub fn room_members(&self) -> Option<MemberList> {
        self.imp().room_members.borrow().clone()
    }

    /// Whether this `RoomHistory` is empty, aka no room is currently displayed.
    pub fn is_empty(&self) -> bool {
        self.imp().room.borrow().is_none()
    }

    fn selection_model(&self) -> &gtk::NoSelection {
        self.imp()
            .selection_model
            .get_or_init(|| gtk::NoSelection::new(gio::ListModel::NONE.cloned()))
    }

    /// Leave the room.
    pub async fn leave(&self) {
        let Some(window) = self.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let Some(room) = self.room() else {
            return;
        };

        if !message_dialog::confirm_leave_room(&room, &window).await {
            return;
        }

        if room.set_category(RoomType::Left).await.is_err() {
            toast!(
                self,
                gettext(
                    // Translators: Do NOT translate the content between '{' and '}', this is a variable name.
                    "Failed to leave {room}",
                ),
                @room,
            );
        }
    }

    pub async fn permalink(&self) {
        if let Some(room) = self.room() {
            let room = room.matrix_room();
            let handle = spawn_tokio!(async move { room.matrix_to_permalink().await });
            match handle.await.unwrap() {
                Ok(permalink) => {
                    self.clipboard().set_text(&permalink.to_string());
                    toast!(self, gettext("Permalink copied to clipboard"));
                }
                Err(error) => {
                    error!("Could not get permalink: {error}");
                    toast!(self, gettext("Failed to copy the permalink"));
                }
            }
        }
    }

    fn init_invite_action(&self, room: &Room) {
        let invite_possible = room.own_user_is_allowed_to_expr(PowerLevelAction::Invite);

        let watch = invite_possible.watch(
            glib::Object::NONE,
            clone!(@weak self as obj => move || {
                obj.update_invite_action();
            }),
        );

        self.imp()
            .room_expr_watches
            .borrow_mut()
            .insert("invite-action", watch);
        self.update_invite_action();
    }

    fn update_invite_action(&self) {
        if let Some(invite_action) = self.imp().room_expr_watches.borrow().get("invite-action") {
            let allow_invite = invite_action
                .evaluate_as::<bool>()
                .expect("Created expression needs to be valid and a boolean");
            self.action_set_enabled("room-history.invite-members", allow_invite);
        };
    }

    /// Opens the room details.
    ///
    /// If `subpage_name` is set, the room details will be opened on the given
    /// subpage.
    pub fn open_room_details(&self, subpage_name: Option<room_details::SubpageName>) {
        if let Some(room) = self.room() {
            let window = RoomDetails::new(&self.parent_window(), &room);
            if let Some(subpage_name) = subpage_name {
                window.show_initial_subpage(subpage_name);
            }
            window.present();
        }
    }

    fn update_room_state(&self) {
        let imp = self.imp();

        if let Some(room) = &*imp.room.borrow() {
            let menu_visible = if room.category() == RoomType::Left {
                self.action_set_enabled("room-history.leave", false);
                false
            } else {
                self.action_set_enabled("room-history.leave", true);
                true
            };
            imp.room_menu.set_visible(menu_visible);
        }
    }

    fn update_view(&self) {
        let imp = self.imp();

        if let Some(room) = &*imp.room.borrow() {
            if room.timeline().is_empty() {
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

        if timeline.is_empty() {
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

    /// Returns the parent GtkWindow containing this widget.
    fn parent_window(&self) -> Option<gtk::Window> {
        self.root().and_downcast()
    }

    /// Whether the room history should stick to the newest message in the
    /// timeline.
    pub fn sticky(&self) -> bool {
        self.imp().sticky.get()
    }

    /// Set whether the room history should stick to the newest message in the
    /// timeline.
    pub fn set_sticky(&self, sticky: bool) {
        let imp = self.imp();

        if self.sticky() == sticky {
            return;
        }

        imp.scroll_btn_revealer.set_reveal_child(!sticky);

        imp.sticky.set(sticky);
        self.notify("sticky");
    }

    /// Scroll to the newest message in the timeline
    pub fn scroll_down(&self) {
        let imp = self.imp();

        imp.is_auto_scrolling.set(true);

        imp.scrolled_window
            .emit_scroll_child(gtk::ScrollType::End, false);
    }

    /// Set `RoomHistory` to stick to the bottom based on scrollbar position
    pub fn enable_sticky_mode(&self) {
        let imp = self.imp();
        let adj = imp.listview.vadjustment().unwrap();
        let is_at_bottom = adj.value() + adj.page_size() == adj.upper();
        self.set_sticky(is_at_bottom);
    }

    fn try_again(&self) {
        self.start_loading();
    }

    pub fn handle_paste_action(&self) {
        self.message_toolbar().handle_paste_action();
    }

    pub fn item_context_menu(&self) -> &gtk::PopoverMenu {
        self.imp()
            .item_context_menu
            .get_or_init(|| gtk::PopoverMenu::from_model(gio::MenuModel::NONE))
    }

    pub fn item_reaction_chooser(&self) -> &ReactionChooser {
        &self.imp().item_reaction_chooser
    }

    fn scroll_to_event(&self, key: &EventKey) {
        let room = match self.room() {
            Some(room) => room,
            None => return,
        };

        if let Some(pos) = room.timeline().find_event_position(key) {
            let pos = pos as u32;
            let _ = self
                .imp()
                .listview
                .activate_action("list.scroll-to-item", Some(&pos.to_variant()));
        }
    }

    /// Trigger the process to update read receipts.
    fn trigger_read_receipts_update(&self) {
        let Some(room) = self.room() else {
            return;
        };

        let timeline = room.timeline();
        if !timeline.is_empty() {
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

        let last_event_id = self.last_visible_event_id();

        if let Some(event_id) = last_event_id {
            spawn!(clone!(@weak self as obj => async move {
                obj.send_receipt(ReceiptType::Read, event_id).await;
            }));
        }
    }

    /// Update the read marker.
    fn update_read_marker(&self) {
        let imp = self.imp();
        imp.read_timeout.take();

        let last_event_id = self.last_visible_event_id();

        if let Some(event_id) = last_event_id {
            spawn!(clone!(@weak self as obj => async move {
                obj.send_receipt(ReceiptType::FullyRead, event_id).await;
            }));
        }
    }

    /// Get the ID of the last visible event in the room history.
    fn last_visible_event_id(&self) -> Option<OwnedEventId> {
        let listview = &*self.imp().listview;
        let mut child = listview.last_child();
        // The visible part of the listview spans between 0 and max.
        let max = listview.height() as f64;

        while let Some(item) = child {
            // Vertical position of the top of the item.
            let (_, top_pos) = item.translate_coordinates(listview, 0.0, 0.0).unwrap();
            // Vertical position of the bottom of the item.
            let (_, bottom_pos) = item
                .translate_coordinates(listview, 0.0, item.height() as f64)
                .unwrap();

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
    async fn send_receipt(&self, receipt_type: ReceiptType, event_id: OwnedEventId) {
        let Some(room) = self.room() else {
            return;
        };

        let matrix_timeline = room.timeline().matrix_timeline();
        let handle = spawn_tokio!(async move {
            matrix_timeline
                .send_single_receipt(receipt_type, ReceiptThread::Unthreaded, event_id)
                .await
        });

        if let Err(error) = handle.await.unwrap() {
            error!("Failed to send read receipt: {error}");
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
    fn join_or_view_successor(&self) {
        let Some(room) = self.room() else {
            return;
        };

        if !room.is_joined() || !room.is_tombstoned() {
            return;
        }

        if let Some(successor) = room.successor() {
            let Some(window) = self.root().and_downcast::<Window>() else {
                return;
            };

            let session = room.session();
            window.show_room(session.session_id(), successor.room_id());
        } else if let Some(successor_id) = room.successor_id().map(ToOwned::to_owned) {
            spawn!(clone!(@weak self as obj, @weak room => async move {
                if let Err(error) = room.session()
                    .room_list()
                    .join_by_id_or_alias(successor_id.into(), vec![]).await
                {
                    toast!(obj, error);
                }
            }));
        }
    }
}
