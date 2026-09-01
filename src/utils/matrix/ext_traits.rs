//! Extension traits for Matrix types that the application adds to.
//!
//! All but one of these are `commune_core::matrix::ext_traits` now and are
//! re-exported here, because `crate::prelude` glob-imports this module and
//! every caller reaches them through it. The one that stays is below: a
//! `TimelineEventItemId` travels as a `GAction` parameter, and `GVariant` is
//! not something a crate with no display can know about. See
//! `doc/track3-convergence.md`.

use std::borrow::Cow;

pub(crate) use commune_core::matrix::ext_traits::{
    AtMentionExt, EventTimelineItemExt, FormattedBodyExt, TimelineItemContentExt,
};
use gtk::{glib, prelude::*};
use matrix_sdk_ui::timeline::TimelineEventItemId;

/// Extension trait for [`TimelineEventItemId`].
pub(crate) trait TimelineEventItemIdExt: Sized {
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
