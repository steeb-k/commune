use gtk::{glib, glib::clone, prelude::*, subclass::prelude::*};
use tracing::warn;

use super::BoundObjectWeakRef;
use crate::session::model::Room;

mod imp {
    use std::cell::RefCell;

    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug)]
    pub struct RoomSpellChecker {
        /// The spell checker used.
        pub checker: spelling::Checker,
        pub checker_handler: RefCell<Option<glib::SignalHandlerId>>,
        /// The room to spell check.
        pub room: BoundObjectWeakRef<Room>,
    }

    impl Default for RoomSpellChecker {
        fn default() -> Self {
            Self {
                // FIXME: this is different than Default::default()
                // See: https://gitlab.gnome.org/World/Rust/libspelling-rs/-/issues/1
                checker: spelling::Checker::default(),
                checker_handler: Default::default(),
                room: Default::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RoomSpellChecker {
        const NAME: &'static str = "RoomSpellChecker";
        type Type = super::RoomSpellChecker;
    }

    impl ObjectImpl for RoomSpellChecker {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpecObject::builder::<spelling::Checker>("checker")
                        .read_only()
                        .build(),
                    glib::ParamSpecObject::builder::<Room>("room")
                        .explicit_notify()
                        .build(),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            match pspec.name() {
                "room" => self
                    .obj()
                    .set_room(value.get::<Option<Room>>().unwrap().as_ref()),
                _ => unimplemented!(),
            }
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            let obj = self.obj();

            match pspec.name() {
                "checker" => obj.checker().to_value(),
                "room" => obj.room().to_value(),
                _ => unimplemented!(),
            }
        }
    }
}

glib::wrapper! {
    /// A spell checker that follows the language of a [`Room`].
    pub struct RoomSpellChecker(ObjectSubclass<imp::RoomSpellChecker>);
}

impl RoomSpellChecker {
    /// Construct a new default `RoomSpellChecker`.
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Set up a `sourceview::View` with this `RoomSpellChecker`.
    pub fn set_up_sourceview(&self, sourceview: &sourceview::View) {
        let imp = self.imp();

        let buffer = sourceview
            .buffer()
            .downcast::<sourceview::Buffer>()
            .unwrap();
        let adapter = spelling::TextBufferAdapter::new(&buffer, &imp.checker);
        let extra_menu = adapter.menu_model();
        sourceview.set_extra_menu(Some(&extra_menu));
        sourceview.insert_action_group("spelling", Some(&adapter));
        adapter.set_enabled(true);
    }

    /// The spell checker used.
    pub fn checker(&self) -> &spelling::Checker {
        &self.imp().checker
    }

    /// The room to spell check.
    pub fn room(&self) -> Option<Room> {
        self.imp().room.obj()
    }

    /// Set the room to spell check.
    pub fn set_room(&self, room: Option<&Room>) {
        if self.room().as_ref() == room {
            return;
        }

        let imp = self.imp();
        imp.room.disconnect_signals();

        if let Some(handler) = imp.checker_handler.take() {
            imp.checker.disconnect(handler);
        }

        if let Some(room) = room {
            let room_language_handler = room.connect_notify_local(
                Some("language"),
                clone!(@weak self as obj => move |room, _| {
                    obj.set_language(room.language());
                }),
            );
            imp.room.set(room, vec![room_language_handler]);

            self.set_language(room.language());

            let checker_language_handler =
                self.checker()
                    .connect_language_notify(clone!(@weak room => move |checker| {
                        let lang = checker.language();
                        let default = checker.provider().default_code();

                        // Do not set the room account data if it was not set and
                        // it uses the default language.
                        if room.language().is_some() || lang != Some(default) {
                            room.set_language(lang.map(Into::into));
                        }
                    }));
            imp.checker_handler.replace(Some(checker_language_handler));
        }

        self.notify("room");
    }

    /// Set the language used by the spell checker.
    ///
    /// This does nothing if the language is not supported by the spell checker.
    fn set_language(&self, language: Option<String>) {
        let checker = self.checker();
        let provider = checker.provider();

        let language = language
            .filter(|lang| {
                if !provider.supports_language(lang) {
                    warn!("Spell checker provider does not support language: {lang}");
                    false
                } else {
                    true
                }
            })
            .unwrap_or_else(|| provider.default_code().into());

        checker.set_language(&language);
    }
}

impl Default for RoomSpellChecker {
    fn default() -> Self {
        Self::new()
    }
}
