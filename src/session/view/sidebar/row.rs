use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{accessible::Relation, gdk, glib, glib::clone};

use super::{CategoryRow, IconItemRow, RoomRow, Sidebar, VerificationRow};
use crate::{
    session::model::{
        Category, CategoryType, IconItem, IdentityVerification, ItemType, Room, RoomType,
        SidebarItem,
    },
    spawn, toast,
    utils::{message_dialog, BoundObjectWeakRef},
};

mod imp {
    use std::{cell::RefCell, marker::PhantomData};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::Row)]
    pub struct Row {
        /// The ancestor sidebar of this row.
        #[property(get, set = Self::set_sidebar, construct_only)]
        pub sidebar: BoundObjectWeakRef<Sidebar>,
        /// The list row to track for expander state.
        #[property(get, set = Self::set_list_row, explicit_notify, nullable)]
        pub list_row: RefCell<Option<gtk::TreeListRow>>,
        /// The sidebar item of this row.
        #[property(get = Self::item)]
        pub item: PhantomData<Option<SidebarItem>>,
        pub bindings: RefCell<Vec<glib::Binding>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Row {
        const NAME: &'static str = "SidebarRow";
        type Type = super::Row;
        type ParentType = adw::Bin;

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
            drop.connect_accept(clone!(@weak obj => @default-return false, move |_, drop| {
                obj.drop_accept(drop)
            }));
            drop.connect_leave(clone!(@weak obj => move |_| {
                obj.drop_leave();
            }));
            drop.connect_drop(
                clone!(@weak obj => @default-return false, move |_, v, _, _| {
                    obj.drop_end(v)
                }),
            );
            obj.add_controller(drop);
        }
    }

    impl WidgetImpl for Row {}
    impl BinImpl for Row {}

    impl Row {
        /// Set the ancestor sidebar of this row.
        fn set_sidebar(&self, sidebar: Sidebar) {
            let obj = self.obj();

            let drop_source_type_handler =
                sidebar.connect_drop_source_category_type_notify(clone!(@weak obj => move |_| {
                    obj.update_for_drop_source_type();
                }));

            let drop_active_target_type_handler = sidebar
                .connect_drop_active_target_category_type_notify(clone!(@weak obj => move |_| {
                    obj.update_for_drop_active_target_type();
                }));

            self.sidebar.set(
                &sidebar,
                vec![drop_source_type_handler, drop_active_target_type_handler],
            );
        }

        /// Set the list row to track for expander state.
        fn set_list_row(&self, list_row: Option<gtk::TreeListRow>) {
            if *self.list_row.borrow() == list_row {
                return;
            }
            let obj = self.obj();

            for binding in self.bindings.take() {
                binding.unbind();
            }

            self.list_row.replace(list_row.clone());

            let mut bindings = vec![];
            if let Some((row, item)) = list_row.zip(self.item()) {
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

                    bindings.push(
                        row.bind_property("expanded", &child, "expanded")
                            .sync_create()
                            .build(),
                    );
                } else if let Some(room) = item.downcast_ref::<Room>() {
                    let child = if let Some(child) = obj.child().and_downcast::<RoomRow>() {
                        child
                    } else {
                        let child = RoomRow::new();
                        obj.set_child(Some(&child));
                        child
                    };

                    child.set_room(Some(room.clone()));
                } else if let Some(icon_item) = item.downcast_ref::<IconItem>() {
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

            self.bindings.replace(bindings);

            obj.notify_item();
            obj.notify_list_row();
        }

        /// The sidebar item of this row.
        fn item(&self) -> Option<SidebarItem> {
            self.list_row
                .borrow()
                .as_ref()
                .and_then(|r| r.item())
                .and_downcast()
        }
    }
}

glib::wrapper! {
    /// A row of the sidebar.
    pub struct Row(ObjectSubclass<imp::Row>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl Row {
    pub fn new(sidebar: &Sidebar) -> Self {
        glib::Object::builder()
            .property("sidebar", sidebar)
            .property("focusable", true)
            .build()
    }

    /// Get the `RoomType` of this item.
    ///
    /// If this is not a `Category` or one of its children, returns `None`.
    pub fn room_type(&self) -> Option<RoomType> {
        let item = self.item()?;

        if let Some(room) = item.downcast_ref::<Room>() {
            Some(room.category())
        } else {
            item.downcast_ref::<Category>()
                .and_then(|category| RoomType::try_from(category.r#type()).ok())
        }
    }

    /// Get the [`ItemType`] of this item.
    ///
    /// If this is not an [`IconItem`], returns `None`.
    pub fn item_type(&self) -> Option<ItemType> {
        self.item()
            .and_downcast_ref::<IconItem>()
            .map(|i| i.r#type())
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
            if let Some(target_type) = self.room_type() {
                if room.category().can_change_to(target_type) {
                    sidebar.set_drop_active_target_type(Some(target_type));
                    return true;
                }
            } else if let Some(item_type) = self.item_type() {
                if room.category() == RoomType::Left && item_type == ItemType::Forget {
                    self.add_css_class("drop-active");
                    sidebar.set_drop_active_target_type(None);
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
            sidebar.set_drop_active_target_type(None);
        }
    }

    /// Handle the drop on this row.
    fn drop_end(&self, value: &glib::Value) -> bool {
        let mut ret = false;
        if let Ok(room) = value.get::<Room>() {
            if let Some(target_type) = self.room_type() {
                if room.category().can_change_to(target_type) {
                    spawn!(clone!(@weak self as obj, @weak room => async move {
                        obj.set_room_category(&room, target_type).await;
                    }));
                    ret = true;
                }
            } else if let Some(item_type) = self.item_type() {
                if room.category() == RoomType::Left && item_type == ItemType::Forget {
                    spawn!(clone!(@strong self as obj, @weak room => async move {
                        obj.forget_room(&room).await;
                    }));
                    ret = true;
                }
            }
        }
        if let Some(sidebar) = self.sidebar() {
            sidebar.set_drop_source_type(None);
        }
        ret
    }

    /// Change the category of the given room room.
    async fn set_room_category(&self, room: &Room, category: RoomType) {
        let Some(window) = self.root().and_downcast::<gtk::Window>() else {
            return;
        };

        if category == RoomType::Left && !message_dialog::confirm_leave_room(room, &window).await {
            return;
        }

        let previous_category = room.category();
        if room.set_category(category).await.is_err() {
            toast!(
                self,
                gettext(
                    // Translators: Do NOT translate the content between '{' and '}', this is a variable name.
                    "Failed to move {room} from {previous_category} to {new_category}.",
                ),
                @room,
                previous_category = previous_category.to_string(),
                new_category = category.to_string(),
            );
        }
    }

    /// Forget the given room.
    async fn forget_room(&self, room: &Room) {
        if room.forget().await.is_err() {
            toast!(
                self,
                // Translators: Do NOT translate the content between '{' and '}', this is a variable name.
                gettext("Failed to forget {room}."),
                @room,
            );
        }
    }

    /// Update the disabled or empty state of this drop target.
    fn update_for_drop_source_type(&self) {
        let source_type = self.sidebar().and_then(|s| s.drop_source_type());

        if let Some(source_type) = source_type {
            if self
                .room_type()
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
                    .is_some_and(|item_type| item_type == ItemType::Forget);
                if is_forget_item && source_type == RoomType::Left {
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
        let Some(room_type) = self.room_type() else {
            return;
        };
        let target_type = self.sidebar().and_then(|s| s.drop_active_target_type());

        if target_type.is_some_and(|target_type| target_type == room_type) {
            self.add_css_class("drop-active");
        } else {
            self.remove_css_class("drop-active");
        }
    }
}
