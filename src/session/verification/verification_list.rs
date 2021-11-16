use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};
use matrix_sdk::ruma::{api::client::r0::sync::sync_events::ToDevice, events::AnyToDeviceEvent};

use crate::session::{
    verification::{IdentityVerification, VerificationMode},
    Session,
};

mod imp {
    use glib::object::WeakRef;
    use once_cell::{sync::Lazy, unsync::OnceCell};
    use std::cell::RefCell;

    use super::*;

    #[derive(Debug, Default)]
    pub struct VerificationList {
        pub list: RefCell<Vec<IdentityVerification>>,
        pub session: OnceCell<WeakRef<Session>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for VerificationList {
        const NAME: &'static str = "VerificationList";
        type Type = super::VerificationList;
        type ParentType = glib::Object;
        type Interfaces = (gio::ListModel,);
    }

    impl ObjectImpl for VerificationList {
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
            _obj: &Self::Type,
            _id: usize,
            value: &glib::Value,
            pspec: &glib::ParamSpec,
        ) {
            match pspec.name() {
                "session" => self
                    .session
                    .set(value.get::<Session>().unwrap().downgrade())
                    .unwrap(),
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

    impl ListModelImpl for VerificationList {
        fn item_type(&self, _list_model: &Self::Type) -> glib::Type {
            IdentityVerification::static_type()
        }
        fn n_items(&self, _list_model: &Self::Type) -> u32 {
            self.list.borrow().len() as u32
        }
        fn item(&self, _list_model: &Self::Type, position: u32) -> Option<glib::Object> {
            self.list
                .borrow()
                .get(position as usize)
                .map(glib::object::Cast::upcast_ref::<glib::Object>)
                .cloned()
        }
    }
}

glib::wrapper! {
    pub struct VerificationList(ObjectSubclass<imp::VerificationList>)
        @implements gio::ListModel;
}

impl VerificationList {
    pub fn new(session: &Session) -> Self {
        glib::Object::new(&[("session", session)]).expect("Failed to create VerificationList")
    }

    pub fn session(&self) -> Session {
        let priv_ = imp::VerificationList::from_instance(self);
        priv_.session.get().unwrap().upgrade().unwrap()
    }

    pub fn handle_response_to_device(&self, to_device: ToDevice) {
        let priv_ = imp::VerificationList::from_instance(self);

        for event in &to_device.events {
            if let Ok(AnyToDeviceEvent::KeyVerificationRequest(event)) = event.deserialize() {
                let request = IdentityVerification::new(self.session().user().unwrap());
                request.set_flow_id(Some(event.content.transaction_id.to_owned()));
                self.add(request);
            }
        }

        for verification in &*priv_.list.borrow() {
            verification.handle_response_to_device(to_device.clone());
        }
    }

    /// Add a new `IdentityVerification` request
    pub fn add(&self, request: IdentityVerification) {
        let priv_ = imp::VerificationList::from_instance(self);
        let length = {
            let mut list = priv_.list.borrow_mut();
            let length = list.len();
            request.connect_notify_local(Some("mode"), clone!(@weak self as obj => move |request, _| {
                if request.mode() == VerificationMode::Error || request.mode() == VerificationMode::Cancelled || request.mode() == VerificationMode::Dismissed || request.mode() == VerificationMode::Completed {
                    obj.remove(request);
                }
            }));
            list.push(request);
            length as u32
        };
        self.items_changed(length, 0, 1)
    }

    pub fn remove(&self, request: &IdentityVerification) {
        let priv_ = imp::VerificationList::from_instance(self);
        let position = {
            let mut list = priv_.list.borrow_mut();
            let mut position = None;
            for (index, item) in list.iter().enumerate() {
                if item == request {
                    position = Some(index);
                    break;
                }
            }
            if let Some(position) = position {
                list.remove(position);
            }
            position
        };
        if let Some(position) = position {
            self.items_changed(position as u32, 1, 0);
        }
    }
}
