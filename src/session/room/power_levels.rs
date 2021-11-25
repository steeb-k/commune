use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use matrix_sdk::ruma::events::room::power_levels::RoomPowerLevelsEventContent;
use matrix_sdk::ruma::events::{EventType, SyncStateEvent};

use crate::session::room::Member;
use crate::utils::prop_expr;

#[derive(Clone, Debug, Default, glib::GBoxed)]
#[gboxed(type_name = "BoxedPowerLevelsEventContent")]
pub struct BoxedPowerLevelsEventContent(RoomPowerLevelsEventContent);

/// Power level of a user.
///
/// Is usually in the range (0..=100), but can be any JS integer.
pub type PowerLevel = i64;
// Same value as MAX_SAFE_INT from js_int.
pub const POWER_LEVEL_MAX: i64 = 0x001F_FFFF_FFFF_FFFF;
pub const POWER_LEVEL_MIN: i64 = -POWER_LEVEL_MAX;

mod imp {
    use super::*;
    use once_cell::sync::Lazy;
    use std::cell::RefCell;

    #[derive(Debug, Default)]
    pub struct PowerLevels {
        pub content: RefCell<BoxedPowerLevelsEventContent>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PowerLevels {
        const NAME: &'static str = "PowerLevels";
        type Type = super::PowerLevels;
        type ParentType = glib::Object;
    }

    impl ObjectImpl for PowerLevels {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![glib::ParamSpec::new_boxed(
                    "power-levels",
                    "Power levels",
                    "Ruma struct containing all power level information of a room",
                    BoxedPowerLevelsEventContent::static_type(),
                    glib::ParamFlags::READABLE,
                )]
            });

            PROPERTIES.as_ref()
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "power-levels" => obj.power_levels().to_value(),
                _ => unimplemented!(),
            }
        }
    }
}

glib::wrapper! {
    pub struct PowerLevels(ObjectSubclass<imp::PowerLevels>);
}

impl PowerLevels {
    pub fn new() -> Self {
        glib::Object::new(&[]).expect("Failed to create PowerLevels")
    }

    pub fn power_levels(&self) -> BoxedPowerLevelsEventContent {
        let priv_ = imp::PowerLevels::from_instance(self);
        priv_.content.borrow().clone()
    }

    /// Returns the power level minimally required to perform the given action.
    pub fn min_level_for_room_action(&self, room_action: &RoomAction) -> PowerLevel {
        let priv_ = imp::PowerLevels::from_instance(self);
        let content = priv_.content.borrow();
        min_level_for_room_action(&content.0, room_action)
    }

    /// Creates an expression that is true when the user is allowed the given action.
    pub fn new_allowed_expr(&self, member: &Member, room_action: RoomAction) -> gtk::Expression {
        gtk::ClosureExpression::new(
            move |args| {
                let power_level: PowerLevel = args[1].get().unwrap();
                let content = args[2].get::<BoxedPowerLevelsEventContent>().unwrap().0;
                power_level >= min_level_for_room_action(&content, &room_action)
            },
            &[
                prop_expr(member, "power-level"),
                prop_expr(self, "power-levels"),
            ],
        )
        .upcast()
    }

    /// Updates the power levels from the given event.
    pub fn update_from_event(&self, event: SyncStateEvent<RoomPowerLevelsEventContent>) {
        let priv_ = imp::PowerLevels::from_instance(self);
        let content = BoxedPowerLevelsEventContent(event.content);
        priv_.content.replace(content);
        self.notify("power-levels");
    }
}

impl Default for PowerLevels {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns the power level minimally required to perform the given action.
fn min_level_for_room_action(
    content: &RoomPowerLevelsEventContent,
    room_action: &RoomAction,
) -> PowerLevel {
    match room_action {
        RoomAction::Ban => content.ban,
        RoomAction::Invite => content.invite,
        RoomAction::Kick => content.kick,
        RoomAction::Redact => content.redact,
        RoomAction::RoomNotification => content.notifications.room,
        RoomAction::StateEvent(event_type) => *content
            .events
            .get(event_type)
            .unwrap_or(&content.state_default),
        RoomAction::MessageEvent(event_type) => *content
            .events
            .get(event_type)
            .unwrap_or(&content.events_default),
    }
    .into()
}

/// Actions that require different power levels to perform them.
pub enum RoomAction {
    Ban,
    Invite,
    Kick,
    Redact,
    RoomNotification,
    StateEvent(EventType),
    MessageEvent(EventType),
}
