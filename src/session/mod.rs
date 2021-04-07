mod content;
mod sidebar;
mod supervisor;

use self::content::FrctlContent;
use self::sidebar::FrctlSidebar;
use self::supervisor::Supervisor;

use crate::secret;
use crate::RUNTIME;

use adw;
use adw::subclass::prelude::BinImpl;
use gtk::subclass::prelude::*;
use gtk::{self, prelude::*};
use gtk::{glib, glib::clone, CompositeTemplate};
use gtk_macros::send;
use log::error;
use matrix_sdk::api::r0::{
    filter::{FilterDefinition, RoomFilter},
    session::login,
};
use matrix_sdk::{self, Client, ClientConfig, RequestConfig, SyncSettings};
use std::time::Duration;

mod imp {
    use super::*;
    use glib::subclass::{InitializingObject, Signal};
    use once_cell::sync::{Lazy, OnceCell};
    use std::cell::RefCell;

    #[derive(Debug, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/session.ui")]
    pub struct FrctlSession {
        #[template_child]
        pub sidebar: TemplateChild<FrctlSidebar>,
        #[template_child]
        pub content: TemplateChild<FrctlContent>,
        pub homeserver: OnceCell<String>,
        /// Contains the error if something went wrong
        pub error: RefCell<Option<matrix_sdk::Error>>,
        pub client: OnceCell<Client>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FrctlSession {
        const NAME: &'static str = "FrctlSession";
        type Type = super::FrctlSession;
        type ParentType = adw::Bin;

        fn new() -> Self {
            Self {
                sidebar: TemplateChild::default(),
                content: TemplateChild::default(),
                homeserver: OnceCell::new(),
                error: RefCell::new(None),
                client: OnceCell::new(),
            }
        }

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for FrctlSession {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![glib::ParamSpec::string(
                    "homeserver",
                    "Homeserver",
                    "The matrix homeserver of this session",
                    None,
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
            match pspec.get_name() {
                "homeserver" => {
                    let homeserver = value
                        .get()
                        .expect("type conformity checked by `Object::set_property`");
                    let _ = self.homeserver.set(homeserver.unwrap());
                }
                _ => unimplemented!(),
            }
        }

        fn get_property(
            &self,
            _obj: &Self::Type,
            _id: usize,
            pspec: &glib::ParamSpec,
        ) -> glib::Value {
            match pspec.get_name() {
                "homeserver" => self.homeserver.get().to_value(),
                _ => unimplemented!(),
            }
        }

        fn signals() -> &'static [Signal] {
            static SIGNALS: Lazy<Vec<Signal>> = Lazy::new(|| {
                vec![Signal::builder("ready", &[], <()>::static_type().into()).build()]
            });
            SIGNALS.as_ref()
        }

        fn constructed(&self, obj: &Self::Type) {
            self.parent_constructed(obj);
        }
    }
    impl WidgetImpl for FrctlSession {}
    impl BinImpl for FrctlSession {}
}

/// Enum containing the supported methods to create a `FrctlSession`.
#[derive(Clone, Debug)]
enum CreationMethod {
    /// Restore a previous session: `matrix_sdk::Session`
    SessionRestore(matrix_sdk::Session),
    /// Password Login: `username`, 'password`
    Password(String, String),
}

glib::wrapper! {
    pub struct FrctlSession(ObjectSubclass<imp::FrctlSession>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl FrctlSession {
    pub fn new(homeserver: String) -> Self {
        glib::Object::new(&[("homeserver", &homeserver)]).expect("Failed to create FrctlSession")
    }

    pub fn login_with_password(&self, username: String, password: String) {
        let method = CreationMethod::Password(username, password);
        self.login(method);
    }

    pub fn login_with_previous_session(&self, session: matrix_sdk::Session) {
        let method = CreationMethod::SessionRestore(session);
        self.login(method);
    }

    fn login(&self, method: CreationMethod) {
        let priv_ = &imp::FrctlSession::from_instance(self);
        let homeserver = priv_.homeserver.get().unwrap();

        let sender = self.setup();

        let config = ClientConfig::new().request_config(RequestConfig::new().retry_limit(2));
        // Please note the homeserver needs to be a valid url or the client will panic!
        let client = Client::new_with_config(homeserver.as_str(), config);

        if let Err(error) = client {
            send!(sender, Err(error));
            return;
        }

        let client = client.unwrap();

        priv_.client.set(client.clone()).unwrap();

        let sidebar_sender = priv_.sidebar.get().setup_channel();
        let content_sender = priv_.content.get().setup_channel();

        let handler = Supervisor::new(sidebar_sender, content_sender);

        RUNTIME.block_on(async {
            tokio::spawn(async move {
                client.set_event_handler(Box::new(handler)).await;

                let success = match method {
                    CreationMethod::SessionRestore(session) => {
                        let res = client.restore_login(session).await;
                        let success = res.is_ok();
                        send!(sender, res.map(|_| None));
                        success
                    }
                    CreationMethod::Password(username, password) => {
                        let response = client
                            .login(&username, &password, None, Some("Fractal Next"))
                            .await;
                        let success = response.is_ok();
                        send!(sender, response.map(|r| Some(r)));
                        success
                    }
                };

                if success {
                    // We need the filter or else left rooms won't be shown
                    let mut room_filter = RoomFilter::empty();
                    room_filter.include_leave = true;

                    let mut filter = FilterDefinition::empty();
                    filter.room = room_filter;

                    let sync_settings = SyncSettings::new()
                        .timeout(Duration::from_secs(30))
                        .full_state(true)
                        .filter(filter.into());
                    client.sync(sync_settings).await;
                }
            });
        });
    }

    fn setup(&self) -> glib::SyncSender<matrix_sdk::Result<Option<login::Response>>> {
        let (sender, receiver) = glib::MainContext::sync_channel::<
            matrix_sdk::Result<Option<login::Response>>,
        >(Default::default(), 100);
        receiver.attach(
            None,
            clone!(@weak self as obj => move |result| {
                match result {
                    Err(error) => {
                        let priv_ = &imp::FrctlSession::from_instance(&obj);
                        priv_.error.replace(Some(error));
                    }
                    Ok(Some(response)) => {
                        let session = matrix_sdk::Session {
                            access_token: response.access_token,
                            user_id: response.user_id,
                            device_id: response.device_id,
                        };
                        //TODO: set error to this error
                        obj.store_session(session).unwrap();
                    }
                    Ok(None) => {}
                }

                obj.load();

                obj.emit_by_name("ready", &[]).unwrap();

                glib::Continue(false)
            }),
        );
        sender
    }

    /// Loads the state from the `Store`
    /// Note that the `Store` currently doesn't store all events, therefore, we arn't really
    /// loading much via this function.
    pub fn load(&self) {
        let priv_ = imp::FrctlSession::from_instance(self);
        priv_.sidebar.load(&priv_.client.get().unwrap());
    }

    /// Returns and consumes the `error` that was generated when the session failed to login,
    /// on a successful login this will be `None`.
    /// Unfortunatly it's not possible to connect the Error direclty to the `ready` signals.
    pub fn get_error(&self) -> Option<matrix_sdk::Error> {
        let priv_ = &imp::FrctlSession::from_instance(self);
        priv_.error.take()
    }

    pub fn connect_ready<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_local("ready", true, move |values| {
            let obj = values[0].get::<Self>().unwrap().unwrap();

            f(&obj);

            None
        })
        .unwrap()
    }

    fn store_session(&self, session: matrix_sdk::Session) -> Result<(), secret_service::Error> {
        let priv_ = &imp::FrctlSession::from_instance(self);
        let homeserver = priv_.homeserver.get().unwrap();
        secret::store_session(homeserver, session)
    }
}
