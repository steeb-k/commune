use std::{ops::Deref, sync::Arc};

use gtk::{glib, prelude::*, subclass::prelude::*};

use super::{SessionInfo, SessionInfoImpl};
use crate::{
    components::AvatarData, prelude::*, secret::StoredSession, utils::matrix::ClientSetupError,
};

#[derive(Clone, Debug, glib::Boxed)]
#[boxed_type(name = "BoxedClientSetupError")]
pub struct BoxedClientSetupError(pub Arc<ClientSetupError>);

impl Deref for BoxedClientSetupError {
    type Target = Arc<ClientSetupError>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

mod imp {
    use std::cell::OnceCell;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::FailedSession)]
    pub struct FailedSession {
        /// The error encountered when initializing the session.
        #[property(get, construct_only)]
        pub error: OnceCell<BoxedClientSetupError>,
        /// The data for the avatar representation for this session.
        pub avatar_data: OnceCell<AvatarData>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FailedSession {
        const NAME: &'static str = "FailedSession";
        type Type = super::FailedSession;
        type ParentType = SessionInfo;
    }

    #[glib::derived_properties]
    impl ObjectImpl for FailedSession {}

    impl SessionInfoImpl for FailedSession {
        fn avatar_data(&self) -> AvatarData {
            self.avatar_data
                .get_or_init(|| {
                    let avatar_data = AvatarData::new();
                    avatar_data.set_display_name(self.obj().user_id().to_string());
                    avatar_data
                })
                .clone()
        }
    }
}

glib::wrapper! {
    /// A Matrix user session that encountered an error when initializing the client.
    pub struct FailedSession(ObjectSubclass<imp::FailedSession>)
        @extends SessionInfo;
}

impl FailedSession {
    /// Constructs a new `FailedSession` with the given info and error.
    pub fn new(stored_session: &StoredSession, error: ClientSetupError) -> Self {
        glib::Object::builder()
            .property("info", stored_session)
            .property("error", BoxedClientSetupError(Arc::new(error)))
            .build()
    }
}
