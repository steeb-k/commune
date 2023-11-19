use gtk::{glib, prelude::*, subclass::prelude::*};
use ruma::{DeviceId, UserId};
use url::Url;

use crate::secret::StoredSession;

#[derive(Clone, Debug, glib::Boxed)]
#[boxed_type(name = "BoxedStoredSession")]
pub struct BoxedStoredSession(pub StoredSession);

mod imp {
    use std::cell::OnceCell;

    use once_cell::sync::Lazy;

    use super::*;

    #[repr(C)]
    pub struct SessionInfoClass {
        pub parent_class: glib::object::ObjectClass,
    }

    unsafe impl ClassStruct for SessionInfoClass {
        type Type = SessionInfo;
    }

    #[derive(Debug, Default)]
    pub struct SessionInfo {
        /// The Matrix session's info.
        pub info: OnceCell<StoredSession>,
    }

    #[glib::object_subclass]
    unsafe impl ObjectSubclass for SessionInfo {
        const NAME: &'static str = "SessionInfo";
        const ABSTRACT: bool = true;
        type Type = super::SessionInfo;
        type Class = SessionInfoClass;
    }

    impl ObjectImpl for SessionInfo {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpecBoxed::builder::<BoxedStoredSession>("info")
                        .write_only()
                        .construct_only()
                        .build(),
                    glib::ParamSpecString::builder("user-id")
                        .read_only()
                        .build(),
                    glib::ParamSpecString::builder("homeserver")
                        .read_only()
                        .build(),
                    glib::ParamSpecString::builder("device-id")
                        .read_only()
                        .build(),
                    glib::ParamSpecString::builder("session-id")
                        .read_only()
                        .build(),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            match pspec.name() {
                "info" => self
                    .info
                    .set(value.get::<BoxedStoredSession>().unwrap().0)
                    .unwrap(),
                _ => unimplemented!(),
            }
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            let obj = self.obj();

            match pspec.name() {
                "user-id" => obj.user_id().as_str().to_value(),
                "homeserver" => obj.homeserver().as_str().to_value(),
                "device-id" => obj.device_id().as_str().to_value(),
                "session-id" => obj.session_id().to_value(),
                _ => unimplemented!(),
            }
        }
    }
}

glib::wrapper! {
    /// Parent class of objects containing a Matrix session's info.
    ///
    /// Its main purpose is to be able to handle `Session`s that are being initialized, or where initialization failed.
    pub struct SessionInfo(ObjectSubclass<imp::SessionInfo>);
}

/// Public trait containing implemented methods for everything that derives from
/// `SessionInfo`.
///
/// To override the behavior of these methods, override the corresponding method
/// of `SessionInfoImpl`.
pub trait SessionInfoExt: 'static {
    /// The Matrix session's info.
    fn info(&self) -> &StoredSession;

    /// The Matrix session's user ID.
    fn user_id(&self) -> &UserId {
        &self.info().user_id
    }

    /// The Matrix session's homeserver.
    fn homeserver(&self) -> &Url {
        &self.info().homeserver
    }

    /// The Matrix session's device ID.
    fn device_id(&self) -> &DeviceId {
        &self.info().device_id
    }

    /// The local session's ID.
    fn session_id(&self) -> &str {
        self.info().id()
    }
}

impl<O: IsA<SessionInfo>> SessionInfoExt for O {
    fn info(&self) -> &StoredSession {
        self.upcast_ref().imp().info.get().unwrap()
    }
}

/// Public trait that must be implemented for everything that derives from
/// `SessionInfo`.
///
/// Overriding a method from this Trait overrides also its behavior in
/// `SessionInfoExt`.
pub trait SessionInfoImpl: ObjectImpl {}

// Make `SessionInfo` subclassable.
unsafe impl<T> IsSubclassable<T> for SessionInfo
where
    T: SessionInfoImpl,
    T::Type: IsA<SessionInfo>,
{
    fn class_init(class: &mut glib::Class<Self>) {
        Self::parent_class_init::<T>(class.upcast_ref_mut());
    }
}
