use std::ops::Deref;

use gtk::{glib, prelude::*, subclass::prelude::*};
use matrix_sdk_ui::timeline::VirtualTimelineItem;
use ruma::MilliSecondsSinceUnixEpoch;

use super::{Timeline, TimelineItem, TimelineItemImpl};
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
    /// Create a new `VirtualItem`.
    fn new(timeline: &Timeline, kind: VirtualItemKind, timeline_id: &str) -> Self {
        glib::Object::builder()
            .property("timeline", timeline)
            .property("kind", kind.boxed())
            .property("timeline-id", timeline_id)
            .build()
    }

    /// Create a new `VirtualItem` from a virtual timeline item.
    pub(crate) fn with_item(
        timeline: &Timeline,
        item: &VirtualTimelineItem,
        timeline_id: &str,
    ) -> Self {
        let kind = match item {
            VirtualTimelineItem::DateDivider(ts) => VirtualItemKind::with_timestamp(*ts),
            VirtualTimelineItem::ReadMarker => VirtualItemKind::NewMessages,
        };

        Self::new(timeline, kind, timeline_id)
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
    pub(crate) fn spinner(timeline: &Timeline) -> Self {
        Self::new(
            timeline,
            VirtualItemKind::Spinner,
            "VirtualItemKind::Spinner",
        )
    }

    /// Whether this is a spinner virtual item.
    pub(crate) fn is_spinner(&self) -> bool {
        self.kind().0 == VirtualItemKind::Spinner
    }

    /// Create a typing virtual item.
    pub(crate) fn typing(timeline: &Timeline) -> Self {
        Self::new(timeline, VirtualItemKind::Typing, "VirtualItemKind::Typing")
    }

    /// Create a timeline start virtual item.
    pub(crate) fn timeline_start(timeline: &Timeline) -> Self {
        Self::new(
            timeline,
            VirtualItemKind::TimelineStart,
            "VirtualItemKind::TimelineStart",
        )
    }

    /// Whether this is a timeline start virtual item.
    pub(crate) fn is_timeline_start(&self) -> bool {
        self.kind().0 == VirtualItemKind::TimelineStart
    }
}
