//! Extension traits for Matrix types.

use std::borrow::Cow;

use gtk::{glib, prelude::*};
use matrix_sdk_ui::timeline::{Message, TimelineEventItemId, TimelineItemContent};

/// Helper trait for types possibly containing an `@room` mention.
pub trait AtMentionExt {
    /// Whether this event might contain an `@room` mention.
    ///
    /// This means that either it doesn't have intentional mentions, or it has
    /// intentional mentions and `room` is set to `true`.
    fn can_contain_at_room(&self) -> bool;
}

impl AtMentionExt for TimelineItemContent {
    fn can_contain_at_room(&self) -> bool {
        match self {
            TimelineItemContent::Message(msg) => msg.can_contain_at_room(),
            _ => false,
        }
    }
}

impl AtMentionExt for Message {
    fn can_contain_at_room(&self) -> bool {
        let Some(mentions) = self.mentions() else {
            return true;
        };

        mentions.room
    }
}

/// Extension trait for [`TimelineEventItemId`].
pub trait TimelineEventItemIdExt: Sized {
    /// The type used to represent a [`TimelineEventItemId`] as a `GVariant`.
    fn static_variant_type() -> Cow<'static, glib::VariantTy>;

    /// Convert this [`TimelineEventItemId`] to a `GVariant`.
    fn to_variant(&self) -> glib::Variant;

    /// Try to convert a `GVariant` to a [`TimelineEventItemId`].
    fn from_variant(variant: &glib::Variant) -> Option<Self>;
}

impl TimelineEventItemIdExt for TimelineEventItemId {
    fn static_variant_type() -> Cow<'static, glib::VariantTy> {
        Cow::Borrowed(glib::VariantTy::STRING)
    }

    fn to_variant(&self) -> glib::Variant {
        let s = match self {
            Self::TransactionId(txn_id) => format!("transaction_id:{txn_id}"),
            Self::EventId(event_id) => format!("event_id:{event_id}"),
        };

        s.to_variant()
    }

    fn from_variant(variant: &glib::Variant) -> Option<Self> {
        let s = variant.str()?;

        if let Some(s) = s.strip_prefix("transaction_id:") {
            Some(Self::TransactionId(s.into()))
        } else if let Some(s) = s.strip_prefix("event_id:") {
            s.try_into().ok().map(Self::EventId)
        } else {
            None
        }
    }
}
