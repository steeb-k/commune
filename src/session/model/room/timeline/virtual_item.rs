use std::ops::Deref;

use gtk::{glib, prelude::*, subclass::prelude::*};
use matrix_sdk_ui::timeline::VirtualTimelineItem;
use ruma::MilliSecondsSinceUnixEpoch;

use super::{TimelineItem, TimelineItemImpl};
use crate::utils::matrix::timestamp_to_date;

/// The kind of virtual item.
#[derive(Debug, Default, Eq, PartialEq, Clone)]
pub enum VirtualItemKind {
    /// A spinner, when the timeline is loading.
    #[default]
    Spinner,
    /// The typing status.
    Typing,
    /// The start of the timeline.
    TimelineStart,
    /// A day separator.
    ///
    /// The date is in UTC.
    DayDivider(glib::DateTime),
    /// A separator for the read marker.
    NewMessages,
}

impl VirtualItemKind {
    /// Construct the `DayDivider` from the given timestamp.
    fn with_timestamp(ts: MilliSecondsSinceUnixEpoch) -> Self {
        Self::DayDivider(timestamp_to_date(ts))
    }

    /// Convert this into a [`BoxedVirtualItemKind`].
    fn boxed(self) -> BoxedVirtualItemKind {
        BoxedVirtualItemKind(self)
    }
}

/// A boxed [`VirtualItemKind`].
#[derive(Clone, Debug, Default, PartialEq, Eq, glib::Boxed)]
#[boxed_type(name = "BoxedVirtualItemKind")]
pub struct BoxedVirtualItemKind(VirtualItemKind);

impl Deref for BoxedVirtualItemKind {
    type Target = VirtualItemKind;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

mod imp {
    use std::cell::RefCell;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::VirtualItem)]
    pub struct VirtualItem {
        /// The kind of virtual item.
        #[property(get, set, construct)]
        kind: RefCell<BoxedVirtualItemKind>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for VirtualItem {
        const NAME: &'static str = "TimelineVirtualItem";
        type Type = super::VirtualItem;
        type ParentType = TimelineItem;
    }

    #[glib::derived_properties]
    impl ObjectImpl for VirtualItem {}

    impl TimelineItemImpl for VirtualItem {}
}

glib::wrapper! {
    /// A virtual item in the timeline.
    ///
    /// A virtual item is an item not based on a timeline event.
    pub struct VirtualItem(ObjectSubclass<imp::VirtualItem>) @extends TimelineItem;
}

impl VirtualItem {
    /// Create a new `VirtualItem` from a virtual timeline item.
    pub(crate) fn with_item(item: &VirtualTimelineItem, timeline_id: &str) -> Self {
        match item {
            VirtualTimelineItem::DateDivider(ts) => {
                Self::day_divider_with_timestamp(*ts, timeline_id)
            }
            VirtualTimelineItem::ReadMarker => Self::new_messages(timeline_id),
        }
    }

    /// Update this `VirtualItem` with the given virtual timeline item.
    pub(crate) fn update_with_item(&self, item: &VirtualTimelineItem) {
        let kind = match item {
            VirtualTimelineItem::DateDivider(ts) => VirtualItemKind::with_timestamp(*ts),
            VirtualTimelineItem::ReadMarker => VirtualItemKind::NewMessages,
        };

        self.set_kind(kind.boxed());
    }

    /// Create a spinner virtual item.
    pub(crate) fn spinner() -> Self {
        glib::Object::builder()
            .property("kind", VirtualItemKind::Spinner.boxed())
            .property("timeline-id", "VirtualItemKind::Spinner")
            .build()
    }

    /// Create a typing virtual item.
    pub(crate) fn typing() -> Self {
        glib::Object::builder()
            .property("kind", VirtualItemKind::Typing.boxed())
            .property("timeline-id", "VirtualItemKind::Typing")
            .build()
    }

    /// Create a timeline start virtual item.
    pub(crate) fn timeline_start() -> Self {
        glib::Object::builder()
            .property("kind", VirtualItemKind::TimelineStart.boxed())
            .property("timeline-id", "VirtualItemKind::TimelineStart")
            .build()
    }

    /// Create a new messages virtual item.
    fn new_messages(timeline_id: &str) -> Self {
        glib::Object::builder()
            .property("kind", VirtualItemKind::NewMessages.boxed())
            .property("timeline-id", timeline_id)
            .build()
    }

    /// Creates a new day divider virtual item for the given timestamp.
    ///
    /// If the timestamp is out of range for `glib::DateTime` (later than the
    /// end of year 9999), this fallbacks to creating a divider with the
    /// current local time.
    ///
    /// Panics if an error occurred when accessing the current local time.
    fn day_divider_with_timestamp(
        timestamp: MilliSecondsSinceUnixEpoch,
        timeline_id: &str,
    ) -> Self {
        glib::Object::builder()
            .property("kind", VirtualItemKind::with_timestamp(timestamp).boxed())
            .property("timeline-id", timeline_id)
            .build()
    }
}
