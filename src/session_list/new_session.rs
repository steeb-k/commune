use gtk::{glib, subclass::prelude::*};

use super::{BoxedStoredSession, SessionInfo, SessionInfoImpl};
use crate::secret::StoredSession;

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct NewSession {}

    #[glib::object_subclass]
    impl ObjectSubclass for NewSession {
        const NAME: &'static str = "NewSession";
        type Type = super::NewSession;
        type ParentType = SessionInfo;
    }

    impl ObjectImpl for NewSession {}
    impl SessionInfoImpl for NewSession {}
}

glib::wrapper! {
    /// A brand new Matrix user session that is not constructed yet.
    ///
    /// This is just a wrapper around [`StoredSession`].
    pub struct NewSession(ObjectSubclass<imp::NewSession>)
        @extends SessionInfo;
}

impl NewSession {
    /// Constructs a new `NewSession` with the given info.
    pub fn new(stored_session: StoredSession) -> Self {
        glib::Object::builder()
            .property("info", BoxedStoredSession(stored_session))
            .build()
    }
}
