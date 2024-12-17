use std::ops::Deref;

use gtk::{glib, prelude::*, subclass::prelude::*};
use matrix_sdk_ui::timeline::VirtualTimelineItem;
use ruma::MilliSecondsSinceUnixEpoch;

use super::{TimelineItem, TimelineItemImpl};

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

    impl TimelineItemImpl for VirtualItem {
        fn id(&self) -> String {
            match &**self.kind.borrow() {
                VirtualItemKind::Spinner => "VirtualItem::Spinner".to_owned(),
                VirtualItemKind::Typing => "VirtualItem::Typing".to_owned(),
                VirtualItemKind::TimelineStart => "VirtualItem::TimelineStart".to_owned(),
                VirtualItemKind::DayDivider(date) => {
                    format!("VirtualItem::DayDivider({})", date.format("%F").unwrap())
                }
                VirtualItemKind::NewMessages => "VirtualItem::NewMessages".to_owned(),
            }
        }
    }
}

glib::wrapper! {
    /// A virtual item in the timeline.
    ///
    /// A virtual item is an item not based on a timeline event.
    pub struct VirtualItem(ObjectSubclass<imp::VirtualItem>) @extends TimelineItem;
}

impl VirtualItem {
    /// Create a new `VirtualItem` from a virtual timeline item.
    pub fn new(item: &VirtualTimelineItem) -> Self {
        match item {
            VirtualTimelineItem::DateDivider(ts) => Self::day_divider_with_timestamp(*ts),
            VirtualTimelineItem::ReadMarker => Self::new_messages(),
        }
    }

    /// Create a spinner virtual item.
    pub(crate) fn spinner() -> Self {
        glib::Object::builder()
            .property("kind", VirtualItemKind::Spinner.boxed())
            .build()
    }

    /// Create a typing virtual item.
    pub(crate) fn typing() -> Self {
        glib::Object::builder()
            .property("kind", VirtualItemKind::Typing.boxed())
            .build()
    }

    /// Create a timeline start virtual item.
    pub(crate) fn timeline_start() -> Self {
        glib::Object::builder()
            .property("kind", VirtualItemKind::TimelineStart.boxed())
            .build()
    }

    /// Create a new messages virtual item.
    pub(crate) fn new_messages() -> Self {
        glib::Object::builder()
            .property("kind", VirtualItemKind::NewMessages.boxed())
            .build()
    }

    /// Creates a new day divider virtual item for the given timestamp.
    ///
    /// If the timestamp is out of range for `glib::DateTime` (later than the
    /// end of year 9999), this fallbacks to creating a divider with the
    /// current local time.
    ///
    /// Panics if an error occurred when accessing the current local time.
    pub(crate) fn day_divider_with_timestamp(timestamp: MilliSecondsSinceUnixEpoch) -> Self {
        let date = glib::DateTime::from_unix_utc(timestamp.as_secs().into())
            .or_else(|_| glib::DateTime::now_utc())
            .expect("We should be able to get the current time");

        glib::Object::builder()
            .property("kind", VirtualItemKind::DayDivider(date).boxed())
            .build()
    }
}
