use std::ops::Deref;

use gtk::{glib, prelude::*, subclass::prelude::*};
use ruma::{DeviceId, UserId};
use url::Url;

use crate::{secret::StoredSession, session::model::AvatarData};

#[derive(Clone, Debug, glib::Boxed)]
#[boxed_type(name = "BoxedStoredSession")]
pub struct BoxedStoredSession(pub StoredSession);

impl Deref for BoxedStoredSession {
    type Target = StoredSession;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

mod imp {
    use std::{cell::OnceCell, marker::PhantomData};

    use super::*;

    #[repr(C)]
    pub struct SessionInfoClass {
        pub parent_class: glib::object::ObjectClass,
        pub avatar_data: fn(&super::SessionInfo) -> AvatarData,
    }

    unsafe impl ClassStruct for SessionInfoClass {
        type Type = SessionInfo;
    }

    pub(super) fn session_info_avatar_data(this: &super::SessionInfo) -> AvatarData {
        let klass = this.class();
        (klass.as_ref().avatar_data)(this)
    }

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::SessionInfo)]
    pub struct SessionInfo {
        /// The Matrix session's info.
        #[property(get, construct_only)]
        pub info: OnceCell<BoxedStoredSession>,
        /// The Matrix session's user ID.
        #[property(get = Self::user_id)]
        pub user_id: PhantomData<String>,
        /// The Matrix session's homeserver.
        #[property(get = Self::homeserver)]
        pub homeserver: PhantomData<String>,
        /// The Matrix session's device ID.
        #[property(get = Self::device_id)]
        pub device_id: PhantomData<String>,
        /// The local session's ID.
        #[property(get = Self::session_id)]
        pub session_id: PhantomData<String>,
        /// The avatar data to represent this session.
        #[property(get = Self::avatar_data)]
        pub avatar_data: PhantomData<AvatarData>,
    }

    #[glib::object_subclass]
    unsafe impl ObjectSubclass for SessionInfo {
        const NAME: &'static str = "SessionInfo";
        const ABSTRACT: bool = true;
        type Type = super::SessionInfo;
        type Class = SessionInfoClass;
    }

    #[glib::derived_properties]
    impl ObjectImpl for SessionInfo {}

    impl SessionInfo {
        /// The Matrix session's info.
        pub fn info(&self) -> &StoredSession {
            &self.info.get().unwrap().0
        }

        /// The Matrix session's user ID.
        fn user_id(&self) -> String {
            self.info().user_id.to_string()
        }

        /// The Matrix session's homeserver.
        fn homeserver(&self) -> String {
            self.info().homeserver.to_string()
        }

        /// The Matrix session's device ID.
        fn device_id(&self) -> String {
            self.info().device_id.to_string()
        }

        /// The local session's ID.
        fn session_id(&self) -> String {
            self.info().id().to_owned()
        }

        /// The avatar data to represent this session.
        fn avatar_data(&self) -> AvatarData {
            session_info_avatar_data(&self.obj())
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

    /// The avatar data to represent this session.
    fn avatar_data(&self) -> AvatarData;
}

impl<O: IsA<SessionInfo>> SessionInfoExt for O {
    fn info(&self) -> &StoredSession {
        self.upcast_ref().imp().info()
    }

    fn avatar_data(&self) -> AvatarData {
        imp::session_info_avatar_data(self.upcast_ref())
    }
}

/// Public trait that must be implemented for everything that derives from
/// `SessionInfo`.
///
/// Overriding a method from this Trait overrides also its behavior in
/// `SessionInfoExt`.
pub trait SessionInfoImpl: ObjectImpl {
    fn avatar_data(&self) -> AvatarData;
}

// Make `SessionInfo` subclassable.
unsafe impl<T> IsSubclassable<T> for SessionInfo
where
    T: SessionInfoImpl,
    T::Type: IsA<SessionInfo>,
{
    fn class_init(class: &mut glib::Class<Self>) {
        Self::parent_class_init::<T>(class.upcast_ref_mut());
        let klass = class.as_mut();

        klass.avatar_data = avatar_data_trampoline::<T>;
    }
}

// Virtual method implementation trampolines.
fn avatar_data_trampoline<T>(this: &SessionInfo) -> AvatarData
where
    T: ObjectSubclass + SessionInfoImpl,
    T::Type: IsA<SessionInfo>,
{
    let this = this.downcast_ref::<T::Type>().unwrap();
    this.imp().avatar_data()
}
