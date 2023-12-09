use adw::{prelude::*, subclass::prelude::*};
use gtk::glib;

use crate::session::model::{MemberRole, PowerLevel, POWER_LEVEL_MAX, POWER_LEVEL_MIN};

mod imp {
    use std::cell::Cell;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::Badge)]
    pub struct Badge {
        /// The power level displayed by this badge.
        #[property(get, set = Self::set_power_level, explicit_notify, builder().minimum(POWER_LEVEL_MIN).maximum(POWER_LEVEL_MAX))]
        pub power_level: Cell<PowerLevel>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Badge {
        const NAME: &'static str = "Badge";
        type Type = super::Badge;
        type ParentType = adw::Bin;
    }

    #[glib::derived_properties]
    impl ObjectImpl for Badge {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            obj.add_css_class("badge");
            let label = gtk::Label::new(Some("default"));
            obj.set_child(Some(&label));
        }
    }

    impl WidgetImpl for Badge {}
    impl BinImpl for Badge {}

    impl Badge {
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
    pub struct Badge(ObjectSubclass<imp::Badge>)
        @extends gtk::Widget, adw::Bin;
}

impl Badge {
    pub fn new() -> Self {
        glib::Object::new()
    }

    fn update_badge(&self, power_level: PowerLevel) {
        let label: gtk::Label = self.child().and_downcast().unwrap();
        let role = MemberRole::from(power_level);

        let visible = match role {
            MemberRole::Admin => {
                label.set_text(&format!("{role} {power_level}"));
                self.add_css_class("admin");
                self.remove_css_class("mod");
                true
            }
            MemberRole::Mod => {
                label.set_text(&format!("{role} {power_level}"));
                self.add_css_class("mod");
                self.remove_css_class("admin");
                true
            }
            MemberRole::Peasant if power_level != 0 => {
                label.set_text(&power_level.to_string());
                self.remove_css_class("admin");
                self.remove_css_class("mod");
                true
            }
            _ => false,
        };
        self.set_visible(visible);
    }
}
