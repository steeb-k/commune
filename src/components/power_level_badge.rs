use adw::{prelude::*, subclass::prelude::*};
use gtk::glib;

use crate::session::model::{MemberRole, PowerLevel, POWER_LEVEL_MAX, POWER_LEVEL_MIN};

mod imp {
    use std::cell::Cell;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::PowerLevelBadge)]
    pub struct PowerLevelBadge {
        pub label: gtk::Label,
        /// The power level displayed by this badge.
        #[property(get, set = Self::set_power_level, explicit_notify, minimum = POWER_LEVEL_MIN, maximum = POWER_LEVEL_MAX)]
        pub power_level: Cell<PowerLevel>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PowerLevelBadge {
        const NAME: &'static str = "PowerLevelBadge";
        type Type = super::PowerLevelBadge;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("power-level-badge");
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for PowerLevelBadge {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            obj.set_child(Some(&self.label));
        }
    }

    impl WidgetImpl for PowerLevelBadge {}
    impl BinImpl for PowerLevelBadge {}

    impl PowerLevelBadge {
        /// Set the power level this badge displays.
        fn set_power_level(&self, power_level: PowerLevel) {
            let obj = self.obj();
            obj.update_badge(power_level);

            self.power_level.set(power_level);
            obj.notify_power_level();
        }
    }
}

glib::wrapper! {
    /// Inline widget displaying a badge with a power level.
    ///
    /// The badge displays admin for a power level of 100 and mod for levels
    /// over or equal to 50.
    pub struct PowerLevelBadge(ObjectSubclass<imp::PowerLevelBadge>)
        @extends gtk::Widget, adw::Bin;
}

impl PowerLevelBadge {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Update the badge for the given power level.
    fn update_badge(&self, power_level: PowerLevel) {
        let label = &self.imp().label;
        let role = MemberRole::from(power_level);

        match role {
            MemberRole::Admin => {
                label.set_text(&format!("{role} {power_level}"));
                self.add_css_class("admin");
                self.remove_css_class("mod");
            }
            MemberRole::Mod => {
                label.set_text(&format!("{role} {power_level}"));
                self.add_css_class("mod");
                self.remove_css_class("admin");
            }
            MemberRole::Peasant => {
                label.set_text(&power_level.to_string());
                self.remove_css_class("admin");
                self.remove_css_class("mod");
            }
        };
        self.set_visible(power_level != 0);
    }
}
