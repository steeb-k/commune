use std::ops::Deref;

use gtk::{glib, glib::closure_local, prelude::*, subclass::prelude::*};
use ruma::{
    events::{
        room::power_levels::{PowerLevelAction, RoomPowerLevels, RoomPowerLevelsEventContent},
        SyncStateEvent,
    },
    OwnedUserId, UserId,
};

#[derive(Clone, Debug, glib::Boxed)]
#[boxed_type(name = "BoxedPowerLevels")]
pub struct BoxedPowerLevels(RoomPowerLevels);

impl Deref for BoxedPowerLevels {
    type Target = RoomPowerLevels;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Default for BoxedPowerLevels {
    fn default() -> Self {
        Self(RoomPowerLevelsEventContent::default().into())
    }
}

/// Power level of a user.
///
/// Is usually in the range (0..=100), but can be any JS integer.
pub type PowerLevel = i64;
// Same value as MAX_SAFE_INT from js_int.
pub const POWER_LEVEL_MAX: i64 = 0x001F_FFFF_FFFF_FFFF;
pub const POWER_LEVEL_MIN: i64 = -POWER_LEVEL_MAX;

mod imp {
    use std::cell::RefCell;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::PowerLevels)]
    pub struct PowerLevels {
        /// The source of the power levels information.
        #[property(get)]
        pub power_levels: RefCell<BoxedPowerLevels>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PowerLevels {
        const NAME: &'static str = "PowerLevels";
        type Type = super::PowerLevels;
    }

    #[glib::derived_properties]
    impl ObjectImpl for PowerLevels {}
}

glib::wrapper! {
    /// The power levels of a room.
    pub struct PowerLevels(ObjectSubclass<imp::PowerLevels>);
}

impl PowerLevels {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Returns whether the member with the given user ID is allowed to do the
    /// given action.
    pub fn member_is_allowed_to(&self, user_id: &UserId, room_action: PowerLevelAction) -> bool {
        self.imp()
            .power_levels
            .borrow()
            .user_can_do(user_id, room_action)
    }

    /// Creates an expression that is true when the member with the given user
    /// ID is allowed to do the given action.
    pub fn member_is_allowed_to_expr(
        &self,
        user_id: OwnedUserId,
        room_action: PowerLevelAction,
    ) -> gtk::ClosureExpression {
        gtk::ClosureExpression::new::<bool>(
            &[self.property_expression("power-levels")],
            closure_local!(
                move |_: Option<glib::Object>, power_levels: BoxedPowerLevels| {
                    power_levels.user_can_do(&user_id, room_action.clone())
                }
            ),
        )
    }

    /// Updates the power levels from the given event.
    pub fn update_from_event(&self, event: &SyncStateEvent<RoomPowerLevelsEventContent>) {
        self.imp()
            .power_levels
            .replace(BoxedPowerLevels(event.power_levels()));
        self.notify_power_levels();
    }
}

impl Default for PowerLevels {
    fn default() -> Self {
        Self::new()
    }
}
