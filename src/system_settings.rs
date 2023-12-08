use std::sync::Arc;

use ashpd::{desktop::settings::Settings as SettingsProxy, zvariant};
use futures_util::StreamExt;
use gtk::{glib, glib::clone, prelude::*, subclass::prelude::*};
use tracing::error;

use crate::{spawn, spawn_tokio};

const GNOME_DESKTOP_NAMESPACE: &str = "org.gnome.desktop.interface";

mod imp {
    use std::cell::Cell;

    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug)]
    pub struct SystemSettings {
        /// The clock format setting.
        pub clock_format: Cell<ClockFormat>,
    }

    impl Default for SystemSettings {
        fn default() -> Self {
            // Use the locale's default clock format as a fallback.
            let local_formatted_time = glib::DateTime::now_local()
                .and_then(|d| d.format("%X"))
                .map(|s| s.to_ascii_lowercase());
            let clock_format = match &local_formatted_time {
                Ok(s) if s.ends_with("am") || s.ends_with("pm") => ClockFormat::TwelveHours,
                Ok(_) => ClockFormat::TwentyFourHours,
                Err(error) => {
                    error!("Failed to get local formatted time: {error}");
                    ClockFormat::TwelveHours
                }
            };

            Self {
                clock_format: Cell::new(clock_format),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SystemSettings {
        const NAME: &'static str = "SystemSettings";
        type Type = super::SystemSettings;
    }

    impl ObjectImpl for SystemSettings {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![glib::ParamSpecEnum::builder::<ClockFormat>("clock-format")
                    .read_only()
                    .build()]
            });

            PROPERTIES.as_ref()
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            let obj = self.obj();

            match pspec.name() {
                "clock-format" => obj.clock_format().to_value(),
                _ => unimplemented!(),
            }
        }

        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            spawn!(clone!(@weak obj => async move {
                obj.init().await;
            }));
        }
    }
}

glib::wrapper! {
    /// An API to access system settings.
    pub struct SystemSettings(ObjectSubclass<imp::SystemSettings>);
}

impl SystemSettings {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Initialize the system settings.
    async fn init(&self) {
        let proxy = match spawn_tokio!(async move { SettingsProxy::new().await })
            .await
            .unwrap()
        {
            Ok(proxy) => proxy,
            Err(error) => {
                error!("Failed to access settings portal: {error}");
                return;
            }
        };
        let proxy = Arc::new(proxy);

        let proxy_clone = proxy.clone();
        match spawn_tokio!(async move {
            proxy_clone
                .read::<ClockFormat>(GNOME_DESKTOP_NAMESPACE, ClockFormat::KEY)
                .await
        })
        .await
        .unwrap()
        {
            Ok(clock_format) => self.set_clock_format(clock_format),
            Err(error) => {
                error!("Failed to access clock format system setting: {error}");
                return;
            }
        };

        let clock_format_changed_stream = match spawn_tokio!(async move {
            proxy
                .receive_setting_changed_with_args(GNOME_DESKTOP_NAMESPACE, ClockFormat::KEY)
                .await
        })
        .await
        .unwrap()
        {
            Ok(stream) => stream,
            Err(error) => {
                error!("Failed to listen to changes of the clock format system setting: {error}");
                return;
            }
        };

        let obj_weak = self.downgrade();
        clock_format_changed_stream.for_each(move |setting| {
            let obj_weak = obj_weak.clone();
            async move {
                let clock_format = match ClockFormat::try_from(setting.value()) {
                    Ok(clock_format) => clock_format,
                    Err(error) => {
                        error!("Could not update clock format setting: {error}");
                        return;
                    }
                };

                if let Some(obj) = obj_weak.upgrade() {
                    obj.set_clock_format(clock_format);
                } else {
                    error!("Could not update clock format setting: could not upgrade weak reference");
                }
            }
        }).await;
    }

    /// The clock format setting.
    pub fn clock_format(&self) -> ClockFormat {
        self.imp().clock_format.get()
    }

    /// Set the clock format setting.
    fn set_clock_format(&self, clock_format: ClockFormat) {
        if self.clock_format() == clock_format {
            return;
        }

        self.imp().clock_format.set(clock_format);
        self.notify("clock-format");
    }
}

impl Default for SystemSettings {
    fn default() -> Self {
        Self::new()
    }
}

/// The clock format setting.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, glib::Enum)]
#[repr(u32)]
#[enum_type(name = "ClockFormat")]
pub enum ClockFormat {
    /// The 12h format, i.e. AM/PM.
    #[default]
    TwelveHours = 0,
    /// The 24h format.
    TwentyFourHours = 1,
}

impl ClockFormat {
    const KEY: &'static str = "clock-format";
}

impl TryFrom<&zvariant::OwnedValue> for ClockFormat {
    type Error = zvariant::Error;

    fn try_from(value: &zvariant::OwnedValue) -> Result<Self, Self::Error> {
        let Ok(s) = <&str>::try_from(value) else {
            return Err(zvariant::Error::IncorrectType);
        };

        match s {
            "12h" => Ok(Self::TwelveHours),
            "24h" => Ok(Self::TwentyFourHours),
            _ => Err(zvariant::Error::Message(format!(
                "Invalid string `{s}`, expected `12h` or `24h`"
            ))),
        }
    }
}

impl TryFrom<zvariant::OwnedValue> for ClockFormat {
    type Error = zvariant::Error;

    fn try_from(value: zvariant::OwnedValue) -> Result<Self, Self::Error> {
        Self::try_from(&value)
    }
}
