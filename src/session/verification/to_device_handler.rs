use crate::session::{verification::IdentityVerification, Session};
use gtk::{glib, prelude::*, subclass::prelude::*};
use matrix_sdk::ruma::api::client::r0::sync::sync_events::ToDevice;

mod imp {
    use super::*;
    use glib::object::WeakRef;
    use once_cell::sync::{Lazy, OnceCell};
    use std::cell::RefCell;

    #[derive(Debug, Default)]
    pub struct ToDeviceHandler {
        pub session: OnceCell<WeakRef<Session>>,
        pub verifications: RefCell<Vec<IdentityVerification>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ToDeviceHandler {
        const NAME: &'static str = "ToDeviceHandler";
        type Type = super::ToDeviceHandler;
        type ParentType = glib::Object;
    }

    impl ObjectImpl for ToDeviceHandler {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![glib::ParamSpec::new_object(
                    "session",
                    "Session",
                    "The session",
                    Session::static_type(),
                    glib::ParamFlags::READWRITE | glib::ParamFlags::CONSTRUCT_ONLY,
                )]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(
            &self,
            obj: &Self::Type,
            _id: usize,
            value: &glib::Value,
            pspec: &glib::ParamSpec,
        ) {
            match pspec.name() {
                "session" => obj.set_session(value.get().unwrap()),
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "session" => obj.session().to_value(),
                _ => unimplemented!(),
            }
        }
    }
}

glib::wrapper! {
    pub struct ToDeviceHandler(ObjectSubclass<imp::ToDeviceHandler>);
}

impl ToDeviceHandler {
    pub fn new(session: &Session) -> Self {
        glib::Object::new(&[("session", session)]).expect("Failed to create ToDeviceHandler")
    }

    pub fn session(&self) -> Session {
        let priv_ = imp::ToDeviceHandler::from_instance(self);
        priv_.session.get().unwrap().upgrade().unwrap()
    }

    fn set_session(&self, session: Session) {
        let priv_ = imp::ToDeviceHandler::from_instance(self);
        priv_.session.set(session.downgrade()).unwrap()
    }

    pub fn handle_response_to_device(&self, to_device: ToDevice) {
        let priv_ = imp::ToDeviceHandler::from_instance(self);

        for verification in &*priv_.verifications.borrow() {
            // TODO: handle incomming requests
            verification.handle_response_to_device(to_device.clone());
        }
    }

    /// Add a new `IdentityVerification` request that should be tracked
    pub fn add_request(&self, request: IdentityVerification) {
        let priv_ = imp::ToDeviceHandler::from_instance(self);

        priv_.verifications.borrow_mut().push(request);
    }
}
