use gtk::{glib, prelude::*, subclass::prelude::*};
use matrix_sdk_ui::timeline::{TimelineItem as SdkTimelineItem, TimelineItemKind};
use ruma::OwnedUserId;
use tracing::error;

use super::VirtualItem;
use crate::session::model::{Event, Room};

mod imp {
    use std::{
        cell::{Cell, RefCell},
        marker::PhantomData,
    };

    use super::*;

    #[repr(C)]
    pub struct TimelineItemClass {
        parent_class: glib::object::ObjectClass,
        pub(super) selectable: fn(&super::TimelineItem) -> bool,
        pub(super) can_hide_header: fn(&super::TimelineItem) -> bool,
        pub(super) event_sender_id: fn(&super::TimelineItem) -> Option<OwnedUserId>,
    }

    unsafe impl ClassStruct for TimelineItemClass {
        type Type = TimelineItem;
    }

    pub(super) fn timeline_item_selectable(this: &super::TimelineItem) -> bool {
        let klass = this.class();
        (klass.as_ref().selectable)(this)
    }

    pub(super) fn timeline_item_can_hide_header(this: &super::TimelineItem) -> bool {
        let klass = this.class();
        (klass.as_ref().can_hide_header)(this)
    }

    pub(super) fn timeline_item_event_sender_id(this: &super::TimelineItem) -> Option<OwnedUserId> {
        let klass = this.class();
        (klass.as_ref().event_sender_id)(this)
    }

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::TimelineItem)]
    pub struct TimelineItem {
        /// A unique ID for this `TimelineItem` in the local timeline.
        #[property(get, construct_only)]
        timeline_id: RefCell<String>,
        /// Whether this `TimelineItem` is selectable.
        ///
        /// Defaults to `false`.
        #[property(get = Self::selectable)]
        selectable: PhantomData<bool>,
        /// Whether this `TimelineItem` should show its header.
        ///
        /// Defaults to `false`.
        #[property(get, set = Self::set_show_header, explicit_notify)]
        show_header: Cell<bool>,
        /// Whether this `TimelineItem` is allowed to hide its header.
        ///
        /// Defaults to `false`.
        #[property(get = Self::can_hide_header)]
        can_hide_header: PhantomData<bool>,
        /// If this is a Matrix event, the sender of the event.
        ///
        /// Defaults to `None`.
        #[property(get = Self::event_sender_id)]
        event_sender_id: PhantomData<Option<String>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TimelineItem {
        const NAME: &'static str = "TimelineItem";
        const ABSTRACT: bool = true;
        type Type = super::TimelineItem;
        type Class = TimelineItemClass;
    }

    #[glib::derived_properties]
    impl ObjectImpl for TimelineItem {}

    impl TimelineItem {
        /// Whether this `TimelineItem` is selectable.
        ///
        /// Defaults to `false`.
        fn selectable(&self) -> bool {
            imp::timeline_item_selectable(&self.obj())
        }

        /// Set whether this `TimelineItem` should show its header.
        fn set_show_header(&self, show: bool) {
            if self.show_header.get() == show {
                return;
            }

            self.show_header.set(show);
            self.obj().notify_show_header();
        }

        /// Whether this `TimelineItem` is allowed to hide its header.
        ///
        /// Defaults to `false`.
        fn can_hide_header(&self) -> bool {
            imp::timeline_item_can_hide_header(&self.obj())
        }

        /// If this is a Matrix event, the sender of the event.
        ///
        /// Defaults to `None`.
        fn event_sender_id(&self) -> Option<String> {
            imp::timeline_item_event_sender_id(&self.obj()).map(Into::into)
        }
    }
}

glib::wrapper! {
    /// Interface implemented by items inside the `Timeline`.
    pub struct TimelineItem(ObjectSubclass<imp::TimelineItem>);
}

impl TimelineItem {
    /// Create a new `TimelineItem` with the given SDK timeline item.
    ///
    /// Constructs the proper child type.
    pub fn new(item: &SdkTimelineItem, room: &Room) -> Self {
        let timeline_id = &item.unique_id().0;

        match item.kind() {
            TimelineItemKind::Event(event) => Event::new(event.clone(), room, timeline_id).upcast(),
            TimelineItemKind::Virtual(item) => VirtualItem::with_item(item, timeline_id).upcast(),
        }
    }

    /// Update this `TimelineItem` with the given SDK timeline item.
    ///
    /// A `TimelineItem` should not be updated with a SDK item that has a
    /// different timeline ID.
    pub(crate) fn update_with(&self, item: &SdkTimelineItem) {
        if self.timeline_id() != item.unique_id().0 {
            error!("Should not update an item with a different timeline ID");
        }

        match item.kind() {
            TimelineItemKind::Event(new_event) => {
                if let Some(event) = self.downcast_ref::<Event>() {
                    event.update_with(new_event.clone());
                } else {
                    error!("Could not update a TimelineItem that is not an Event with an event SDK item");
                }
            }
            TimelineItemKind::Virtual(new_item) => {
                if let Some(virtual_item) = self.downcast_ref::<VirtualItem>() {
                    virtual_item.update_with_item(new_item);
                } else {
                    error!("Could not update a TimelineItem that is not a VirtualItem with a virtual SDK item");
                }
            }
        }
    }
}

/// Public trait containing implemented methods for everything that derives from
/// `TimelineItem`.
///
/// To override the behavior of these methods, override the corresponding method
/// of `TimelineItemImpl`.
#[allow(dead_code)]
pub(crate) trait TimelineItemExt: 'static {
    /// A unique ID for this `TimelineItem` in the local timeline.
    fn timeline_id(&self) -> String;

    /// Whether this `TimelineItem` is selectable.
    ///
    /// Defaults to `false`.
    fn selectable(&self) -> bool;

    /// Whether this `TimelineItem` should show its header.
    ///
    /// Defaults to `false`.
    fn show_header(&self) -> bool;

    /// Set whether this `TimelineItem` should show its header.
    fn set_show_header(&self, show: bool);

    /// Whether this `TimelineItem` is allowed to hide its header.
    ///
    /// Defaults to `false`.
    fn can_hide_header(&self) -> bool;

    /// If this is a Matrix event, the sender of the event.
    ///
    /// Defaults to `None`.
    fn event_sender_id(&self) -> Option<OwnedUserId>;
}

impl<O: IsA<TimelineItem>> TimelineItemExt for O {
    fn timeline_id(&self) -> String {
        self.upcast_ref().timeline_id()
    }

    fn selectable(&self) -> bool {
        self.upcast_ref().selectable()
    }

    fn show_header(&self) -> bool {
        self.upcast_ref().show_header()
    }

    fn set_show_header(&self, show: bool) {
        self.upcast_ref().set_show_header(show);
    }

    fn can_hide_header(&self) -> bool {
        self.upcast_ref().can_hide_header()
    }

    fn event_sender_id(&self) -> Option<OwnedUserId> {
        imp::timeline_item_event_sender_id(self.upcast_ref())
    }
}

/// Public trait that must be implemented for everything that derives from
/// `TimelineItem`.
///
/// Overriding a method from this Trait overrides also its behavior in
/// `TimelineItemExt`.
pub(crate) trait TimelineItemImpl: ObjectImpl {
    fn selectable(&self) -> bool {
        false
    }

    fn can_hide_header(&self) -> bool {
        false
    }

    fn event_sender_id(&self) -> Option<OwnedUserId> {
        None
    }
}

// Make `TimelineItem` subclassable.
unsafe impl<T> IsSubclassable<T> for TimelineItem
where
    T: TimelineItemImpl,
    T::Type: IsA<TimelineItem>,
{
    fn class_init(class: &mut glib::Class<Self>) {
        Self::parent_class_init::<T>(class.upcast_ref_mut());

        let klass = class.as_mut();

        klass.selectable = selectable_trampoline::<T>;
        klass.can_hide_header = can_hide_header_trampoline::<T>;
        klass.event_sender_id = event_sender_id_trampoline::<T>;
    }
}

// Virtual method implementation trampolines.
fn selectable_trampoline<T>(this: &TimelineItem) -> bool
where
    T: ObjectSubclass + TimelineItemImpl,
    T::Type: IsA<TimelineItem>,
{
    let this = this.downcast_ref::<T::Type>().unwrap();
    this.imp().selectable()
}

fn can_hide_header_trampoline<T>(this: &TimelineItem) -> bool
where
    T: ObjectSubclass + TimelineItemImpl,
    T::Type: IsA<TimelineItem>,
{
    let this = this.downcast_ref::<T::Type>().unwrap();
    this.imp().can_hide_header()
}

fn event_sender_id_trampoline<T>(this: &TimelineItem) -> Option<OwnedUserId>
where
    T: ObjectSubclass + TimelineItemImpl,
    T::Type: IsA<TimelineItem>,
{
    let this = this.downcast_ref::<T::Type>().unwrap();
    this.imp().event_sender_id()
}
