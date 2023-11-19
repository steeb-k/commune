use std::sync::Arc;

use gtk::{glib, prelude::*, subclass::prelude::*};

use super::{BoxedStoredSession, SessionInfo, SessionInfoImpl};
use crate::{secret::StoredSession, utils::matrix::ClientSetupError};

#[derive(Clone, Debug, glib::Boxed)]
#[boxed_type(name = "BoxedClientSetupError")]
struct BoxedClientSetupError(Arc<ClientSetupError>);

mod imp {
    use once_cell::{sync::Lazy, unsync::OnceCell};

    use super::*;

    #[derive(Debug, Default)]
    pub struct FailedSession {
        /// The error encountered when initializing the session.
        pub error: OnceCell<Arc<ClientSetupError>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FailedSession {
        const NAME: &'static str = "FailedSession";
        type Type = super::FailedSession;
        type ParentType = SessionInfo;
    }

    impl ObjectImpl for FailedSession {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpecBoxed::builder::<BoxedClientSetupError>("error")
                        .write_only()
                        .construct_only()
                        .build(),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            match pspec.name() {
                "error" => self
                    .error
                    .set(value.get::<BoxedClientSetupError>().unwrap().0)
                    .unwrap(),
                _ => unimplemented!(),
            }
        }
    }

    impl SessionInfoImpl for FailedSession {}
}

glib::wrapper! {
    /// A Matrix user session that encountered an error when initializing the client.
    pub struct FailedSession(ObjectSubclass<imp::FailedSession>)
        @extends SessionInfo;
}

impl FailedSession {
    /// Constructs a new `FailedSession` with the given info and error.
    pub fn new(stored_session: StoredSession, error: ClientSetupError) -> Self {
        glib::Object::builder()
            .property("info", BoxedStoredSession(stored_session))
            .property("error", BoxedClientSetupError(Arc::new(error)))
            .build()
    }

    /// The error of the session.
    pub fn error(&self) -> Arc<ClientSetupError> {
        self.imp().error.get().unwrap().clone()
    }
}
