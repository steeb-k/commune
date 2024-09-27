use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{accessible::Relation, gdk, gio, glib, glib::clone};
use ruma::api::client::receipt::create_receipt::v3::ReceiptType;
use tracing::error;

use super::{CategoryRow, IconItemRow, RoomRow, Sidebar, VerificationRow};
use crate::{
    components::{
        confirm_leave_room_dialog, ContextMenuBin, ContextMenuBinExt, ContextMenuBinImpl,
    },
    session::model::{
        Category, CategoryType, IdentityVerification, ReceiptPosition, Room, RoomCategory,
        SidebarIconItem, SidebarIconItemType, User,
    },
    spawn, spawn_tokio, toast,
    utils::BoundObjectWeakRef,
};

mod imp {
    use std::cell::RefCell;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::Row)]
    pub struct Row {
        /// The ancestor sidebar of this row.
        #[property(get, set = Self::set_sidebar, construct_only)]
        pub sidebar: BoundObjectWeakRef<Sidebar>,
        /// The item of this row.
        #[property(get, set = Self::set_item, explicit_notify, nullable)]
        pub item: RefCell<Option<glib::Object>>,
        room_handler: RefCell<Option<glib::SignalHandlerId>>,
        room_join_rule_handler: RefCell<Option<glib::SignalHandlerId>>,
        room_is_read_handler: RefCell<Option<glib::SignalHandlerId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Row {
        const NAME: &'static str = "SidebarRow";
        type Type = super::Row;
        type ParentType = ContextMenuBin;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("sidebar-row");
            klass.set_accessible_role(gtk::AccessibleRole::ListItem);
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for Row {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            // Set up drop controller
            let drop = gtk::DropTarget::builder()
                .actions(gdk::DragAction::MOVE)
                .formats(&gdk::ContentFormats::for_type(Room::static_type()))
                .build();
            drop.connect_accept(clone!(
                #[weak]
                obj,
                #[upgrade_or]
                false,
                move |_, drop| obj.drop_accept(drop)
            ));
            drop.connect_leave(clone!(
                #[weak]
                obj,
                move |_| {
                    obj.drop_leave();
                }
            ));
            drop.connect_drop(clone!(
                #[weak]
                obj,
                #[upgrade_or]
                false,
                move |_, v, _, _| obj.drop_end(v)
            ));
            obj.add_controller(drop);
        }

        fn dispose(&self) {
            if let Some(room) = self.room() {
                if let Some(handler) = self.room_join_rule_handler.take() {
                    room.join_rule().disconnect(handler);
                }
                if let Some(handler) = self.room_is_read_handler.take() {
                    room.disconnect(handler);
                }
            }
        }
    }

    impl WidgetImpl for Row {}

    impl ContextMenuBinImpl for Row {
        fn menu_opened(&self) {
            if !self.item.borrow().as_ref().is_some_and(|i| i.is::<Room>()) {
                // No context menu.
                return;
            }

            let obj = self.obj();
            if let Some(sidebar) = obj.sidebar() {
                let popover = sidebar.room_row_popover();
                obj.set_popover(Some(popover.clone()));
            }
        }
    }

    impl Row {
        /// Set the ancestor sidebar of this row.
        fn set_sidebar(&self, sidebar: Sidebar) {
            let obj = self.obj();

            let drop_source_type_handler =
                sidebar.connect_drop_source_category_type_notify(clone!(
                    #[weak]
                    obj,
                    move |_| {
                        obj.update_for_drop_source_type();
                    }
                ));

            let drop_active_target_type_handler = sidebar
                .connect_drop_active_target_category_type_notify(clone!(
                    #[weak]
                    obj,
                    move |_| {
                        obj.update_for_drop_active_target_type();
                    }
                ));

            self.sidebar.set(
                &sidebar,
                vec![drop_source_type_handler, drop_active_target_type_handler],
            );
        }

        /// Set the item of this row.
        fn set_item(&self, item: Option<glib::Object>) {
            if *self.item.borrow() == item {
                return;
            }
            let obj = self.obj();

            if let Some(room) = self.room() {
                if let Some(handler) = self.room_handler.take() {
                    room.disconnect(handler);
                }
                if let Some(handler) = self.room_join_rule_handler.take() {
                    room.join_rule().disconnect(handler);
                }
                if let Some(handler) = self.room_is_read_handler.take() {
                    room.disconnect(handler);
                }
            }

            self.item.replace(item.clone());

            self.update_context_menu();

            if let Some(item) = item {
                if let Some(category) = item.downcast_ref::<Category>() {
                    let child = if let Some(child) = obj.child().and_downcast::<CategoryRow>() {
                        child
                    } else {
                        let child = CategoryRow::new();
                        obj.set_child(Some(&child));
                        obj.update_relation(&[Relation::LabelledBy(&[child.labelled_by()])]);
                        child
                    };
                    child.set_category(Some(category.clone()));
                } else if let Some(room) = item.downcast_ref::<Room>() {
                    let child = if let Some(child) = obj.child().and_downcast::<RoomRow>() {
                        child
                    } else {
                        let child = RoomRow::new();
                        obj.set_child(Some(&child));
                        child
                    };

                    let room_is_direct_handler = room.connect_is_direct_notify(clone!(
                        #[weak(rename_to = imp)]
                        self,
                        move |_| {
                            imp.update_context_menu();
                        }
                    ));
                    self.room_handler.replace(Some(room_is_direct_handler));
                    let room_join_rule_handler =
                        room.join_rule().connect_we_can_join_notify(clone!(
                            #[weak(rename_to = imp)]
                            self,
                            move |_| {
                                imp.update_context_menu();
                            }
                        ));
                    self.room_join_rule_handler
                        .replace(Some(room_join_rule_handler));

                    let room_is_read_handler = room.connect_is_read_notify(clone!(
                        #[weak(rename_to = imp)]
                        self,
                        move |_| {
                            imp.update_context_menu();
                        }
                    ));
                    self.room_is_read_handler
                        .replace(Some(room_is_read_handler));

                    child.set_room(Some(room.clone()));
                } else if let Some(icon_item) = item.downcast_ref::<SidebarIconItem>() {
                    let child = if let Some(child) = obj.child().and_downcast::<IconItemRow>() {
                        child
                    } else {
                        let child = IconItemRow::new();
                        obj.set_child(Some(&child));
                        child
                    };

                    child.set_icon_item(Some(icon_item.clone()));
                } else if let Some(verification) = item.downcast_ref::<IdentityVerification>() {
                    let child = if let Some(child) = obj.child().and_downcast::<VerificationRow>() {
                        child
                    } else {
                        let child = VerificationRow::new();
                        obj.set_child(Some(&child));
                        child
                    };

                    child.set_identity_verification(Some(verification.clone()));
                } else {
                    panic!("Wrong row item: {item:?}");
                }

                obj.update_for_drop_source_type();
            }

            self.update_context_menu();
            obj.notify_item();
        }

        /// Get the `Room` of this item, if this is a room row.
        pub(super) fn room(&self) -> Option<Room> {
            self.item.borrow().clone().and_downcast()
        }

        /// Whether this has a room context menu.
        fn has_room_context_menu(&self) -> bool {
            self.room().is_some_and(|r| {
                matches!(
                    r.category(),
                    RoomCategory::Invited
                        | RoomCategory::Favorite
                        | RoomCategory::Normal
                        | RoomCategory::LowPriority
                        | RoomCategory::Left
                )
            })
        }

        /// Update the context menu according to the current state.
        fn update_context_menu(&self) {
            let obj = self.obj();

            if !self.has_room_context_menu() {
                obj.insert_action_group("room-row", None::<&gio::ActionGroup>);
                obj.set_has_context_menu(false);
                return;
            }

            obj.insert_action_group("room-row", self.room_actions().as_ref());
            obj.set_has_context_menu(true);
        }

        /// An action group with the available room actions.
        fn room_actions(&self) -> Option<gio::SimpleActionGroup> {
            let room = self.room()?;

            let obj = self.obj();
            let action_group = gio::SimpleActionGroup::new();
            let category = room.category();

            match category {
                RoomCategory::Invited => {
                    action_group.add_action_entries([
                        gio::ActionEntry::builder("accept-invite")
                            .activate(clone!(
                                #[weak]
                                obj,
                                move |_, _, _| {
                                    if let Some(room) = obj.room() {
                                        spawn!(async move {
                                            obj.set_room_category(&room, RoomCategory::Normal)
                                                .await;
                                        });
                                    }
                                }
                            ))
                            .build(),
                        gio::ActionEntry::builder("reject-invite")
                            .activate(clone!(
                                #[weak]
                                obj,
                                move |_, _, _| {
                                    if let Some(room) = obj.room() {
                                        spawn!(async move {
                                            obj.set_room_category(&room, RoomCategory::Left).await;
                                        });
                                    }
                                }
                            ))
                            .build(),
                    ]);
                }
                RoomCategory::Favorite | RoomCategory::Normal | RoomCategory::LowPriority => {
                    if matches!(category, RoomCategory::Favorite | RoomCategory::LowPriority) {
                        action_group.add_action_entries([gio::ActionEntry::builder("set-normal")
                            .activate(clone!(
                                #[weak]
                                obj,
                                move |_, _, _| {
                                    if let Some(room) = obj.room() {
                                        spawn!(async move {
                                            obj.set_room_category(&room, RoomCategory::Normal)
                                                .await;
                                        });
                                    }
                                }
                            ))
                            .build()]);
                    }

                    if matches!(category, RoomCategory::Normal | RoomCategory::LowPriority) {
                        action_group.add_action_entries([gio::ActionEntry::builder(
                            "set-favorite",
                        )
                        .activate(clone!(
                            #[weak]
                            obj,
                            move |_, _, _| {
                                if let Some(room) = obj.room() {
                                    spawn!(async move {
                                        obj.set_room_category(&room, RoomCategory::Favorite).await;
                                    });
                                }
                            }
                        ))
                        .build()]);
                    }

                    if matches!(category, RoomCategory::Favorite | RoomCategory::Normal) {
                        action_group.add_action_entries([gio::ActionEntry::builder(
                            "set-lowpriority",
                        )
                        .activate(clone!(
                            #[weak]
                            obj,
                            move |_, _, _| {
                                if let Some(room) = obj.room() {
                                    spawn!(async move {
                                        obj.set_room_category(&room, RoomCategory::LowPriority)
                                            .await;
                                    });
                                }
                            }
                        ))
                        .build()]);
                    }

                    action_group.add_action_entries([gio::ActionEntry::builder("leave")
                        .activate(clone!(
                            #[weak]
                            obj,
                            move |_, _, _| {
                                if let Some(room) = obj.room() {
                                    spawn!(async move {
                                        obj.set_room_category(&room, RoomCategory::Left).await;
                                    });
                                }
                            }
                        ))
                        .build()]);

                    if !room.is_read() {
                        action_group.add_action_entries([gio::ActionEntry::builder(
                            "mark-as-read",
                        )
                        .activate(clone!(
                            #[weak]
                            room,
                            move |_, _, _| {
                                spawn!(async move {
                                    room.send_receipt(ReceiptType::Read, ReceiptPosition::End)
                                        .await;
                                });
                            }
                        ))
                        .build()]);
                    }
                }
                RoomCategory::Left => {
                    if room.join_rule().we_can_join() {
                        action_group.add_action_entries([gio::ActionEntry::builder("join")
                            .activate(clone!(
                                #[weak]
                                obj,
                                move |_, _, _| {
                                    if let Some(room) = obj.room() {
                                        spawn!(async move {
                                            obj.set_room_category(&room, RoomCategory::Normal)
                                                .await;
                                        });
                                    }
                                }
                            ))
                            .build()]);
                    }

                    action_group.add_action_entries([gio::ActionEntry::builder("forget")
                        .activate(clone!(
                            #[weak]
                            obj,
                            move |_, _, _| {
                                if let Some(room) = obj.room() {
                                    spawn!(async move {
                                        obj.forget_room(&room).await;
                                    });
                                }
                            }
                        ))
                        .build()]);
                }
                RoomCategory::Outdated | RoomCategory::Space | RoomCategory::Ignored => {}
            }

            if matches!(
                category,
                RoomCategory::Favorite
                    | RoomCategory::Normal
                    | RoomCategory::LowPriority
                    | RoomCategory::Left
            ) {
                if room.is_direct() {
                    action_group.add_action_entries([gio::ActionEntry::builder(
                        "unset-direct-chat",
                    )
                    .activate(clone!(
                        #[weak]
                        obj,
                        move |_, _, _| {
                            if let Some(room) = obj.room() {
                                spawn!(async move {
                                    obj.set_room_is_direct(&room, false).await;
                                });
                            }
                        }
                    ))
                    .build()]);
                } else {
                    action_group.add_action_entries([gio::ActionEntry::builder("set-direct-chat")
                        .activate(clone!(
                            #[weak]
                            obj,
                            move |_, _, _| {
                                if let Some(room) = obj.room() {
                                    spawn!(async move {
                                        obj.set_room_is_direct(&room, true).await;
                                    });
                                }
                            }
                        ))
                        .build()]);
                }
            }

            Some(action_group)
        }
    }
}

glib::wrapper! {
    /// A row of the sidebar.
    pub struct Row(ObjectSubclass<imp::Row>)
        @extends gtk::Widget, ContextMenuBin, @implements gtk::Accessible;
}

impl Row {
    pub fn new(sidebar: &Sidebar) -> Self {
        glib::Object::builder().property("sidebar", sidebar).build()
    }

    /// Get the `Room` of this item, if this is a room row.
    pub fn room(&self) -> Option<Room> {
        self.imp().room()
    }

    /// Get the `RoomCategory` of this item.
    ///
    /// If this is not a `Category` containing rooms or a room, returns `None`.
    pub fn room_category(&self) -> Option<RoomCategory> {
        let item = self.item()?;

        if let Some(room) = item.downcast_ref::<Room>() {
            Some(room.category())
        } else {
            item.downcast_ref::<Category>()
                .and_then(|category| RoomCategory::try_from(category.category_type()).ok())
        }
    }

    /// Get the [`SidebarIconItemType`] of this item.
    ///
    /// If this is not a [`SidebarIconItem`], returns `None`.
    pub fn item_type(&self) -> Option<SidebarIconItemType> {
        self.item()
            .and_downcast_ref::<SidebarIconItem>()
            .map(|i| i.item_type())
    }

    /// Handle the drag-n-drop hovering this row.
    fn drop_accept(&self, drop: &gdk::Drop) -> bool {
        let Some(sidebar) = self.sidebar() else {
            return false;
        };

        let room = drop
            .drag()
            .map(|drag| drag.content())
            .and_then(|content| content.value(Room::static_type()).ok())
            .and_then(|value| value.get::<Room>().ok());
        if let Some(room) = room {
            if let Some(target_category) = self.room_category() {
                if room.category().can_change_to(target_category) {
                    sidebar.set_drop_active_target_category(Some(target_category));
                    return true;
                }
            } else if let Some(item_type) = self.item_type() {
                if room.category() == RoomCategory::Left && item_type == SidebarIconItemType::Forget
                {
                    self.add_css_class("drop-active");
                    sidebar.set_drop_active_target_category(None);
                    return true;
                }
            }
        }
        false
    }

    /// Handle the drag-n-drop leaving this row.
    fn drop_leave(&self) {
        self.remove_css_class("drop-active");
        if let Some(sidebar) = self.sidebar() {
            sidebar.set_drop_active_target_category(None);
        }
    }

    /// Handle the drop on this row.
    fn drop_end(&self, value: &glib::Value) -> bool {
        let mut ret = false;
        if let Ok(room) = value.get::<Room>() {
            if let Some(target_category) = self.room_category() {
                if room.category().can_change_to(target_category) {
                    spawn!(clone!(
                        #[weak(rename_to = obj)]
                        self,
                        async move {
                            obj.set_room_category(&room, target_category).await;
                        }
                    ));
                    ret = true;
                }
            } else if let Some(item_type) = self.item_type() {
                if room.category() == RoomCategory::Left && item_type == SidebarIconItemType::Forget
                {
                    spawn!(clone!(
                        #[strong(rename_to = obj)]
                        self,
                        async move {
                            obj.forget_room(&room).await;
                        }
                    ));
                    ret = true;
                }
            }
        }
        if let Some(sidebar) = self.sidebar() {
            sidebar.set_drop_source_category(None);
        }
        ret
    }

    /// Change the category of the given room.
    async fn set_room_category(&self, room: &Room, category: RoomCategory) {
        let ignored_inviter = if category == RoomCategory::Left {
            let Some(response) = confirm_leave_room_dialog(room, self).await else {
                return;
            };

            response.ignore_inviter.then(|| room.inviter()).flatten()
        } else {
            None
        };

        let previous_category = room.category();
        if room.set_category(category).await.is_err() {
            toast!(
                self,
                gettext(
                    // Translators: Do NOT translate the content between '{' and '}', this is a variable name.
                    "Could not move {room} from {previous_category} to {new_category}",
                ),
                @room,
                previous_category = previous_category.to_string(),
                new_category = category.to_string(),
            );
        }

        if let Some(inviter) = ignored_inviter {
            if inviter.upcast::<User>().ignore().await.is_err() {
                toast!(self, gettext("Could not ignore user"));
            }
        }
    }

    /// Forget the given room.
    async fn forget_room(&self, room: &Room) {
        if room.forget().await.is_err() {
            toast!(
                self,
                // Translators: Do NOT translate the content between '{' and '}', this is a variable name.
                gettext("Could not forget {room}"),
                @room,
            );
        }
    }

    /// Set or unset the room as a direct chat.
    async fn set_room_is_direct(&self, room: &Room, is_direct: bool) {
        let matrix_room = room.matrix_room().clone();
        let handle = spawn_tokio!(async move { matrix_room.set_is_direct(is_direct).await });

        if let Err(error) = handle.await.unwrap() {
            if is_direct {
                error!("Could not mark room as direct chat: {error}");
                // Translators: Do NOT translate the content between '{' and '}', this is a
                // variable name.
                toast!(self, gettext("Could not mark {room} as direct chat"), @room,);
            } else {
                error!("Could not unmark room as direct chat: {error}");
                // Translators: Do NOT translate the content between '{' and '}', this is a
                // variable name.
                toast!(self, gettext("Could not unmark {room} as direct chat"), @room,);
            }
        }
    }

    /// Update the disabled or empty state of this drop target.
    fn update_for_drop_source_type(&self) {
        let source_type = self.sidebar().and_then(|s| s.drop_source_category());

        if let Some(source_type) = source_type {
            if self
                .room_category()
                .is_some_and(|row_type| source_type.can_change_to(row_type))
            {
                self.remove_css_class("drop-disabled");

                if self
                    .item()
                    .and_downcast::<Category>()
                    .is_some_and(|category| category.empty())
                {
                    self.add_css_class("drop-empty");
                } else {
                    self.remove_css_class("drop-empty");
                }
            } else {
                let is_forget_item = self
                    .item_type()
                    .is_some_and(|item_type| item_type == SidebarIconItemType::Forget);
                if is_forget_item && source_type == RoomCategory::Left {
                    self.remove_css_class("drop-disabled");
                } else {
                    self.add_css_class("drop-disabled");
                    self.remove_css_class("drop-empty");
                }
            }
        } else {
            // Clear style
            self.remove_css_class("drop-disabled");
            self.remove_css_class("drop-empty");
            self.remove_css_class("drop-active");
        };

        if let Some(category_row) = self.child().and_downcast::<CategoryRow>() {
            category_row.set_show_label_for_category(
                source_type.map(CategoryType::from).unwrap_or_default(),
            );
        }
    }

    /// Update the active state of this drop target.
    fn update_for_drop_active_target_type(&self) {
        let Some(room_category) = self.room_category() else {
            return;
        };
        let target_category = self.sidebar().and_then(|s| s.drop_active_target_category());

        if target_category.is_some_and(|target_category| target_category == room_category) {
            self.add_css_class("drop-active");
        } else {
            self.remove_css_class("drop-active");
        }
    }
}
