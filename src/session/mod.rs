mod categories;
mod content;
mod room;
mod sidebar;
mod user;

use self::content::Content;
use self::sidebar::Sidebar;
use self::user::User;

use crate::event_from_sync_event;
use crate::secret;
use crate::RUNTIME;

use adw;
use adw::subclass::prelude::BinImpl;
use gtk::subclass::prelude::*;
use gtk::{self, prelude::*};
use gtk::{glib, glib::clone, glib::SyncSender, CompositeTemplate};
use gtk_macros::send;
use log::{error, warn};
use matrix_sdk::api::r0::{
    filter::{FilterDefinition, LazyLoadOptions, RoomEventFilter, RoomFilter},
    session::login,
};
use matrix_sdk::{
    self, assign,
    deserialized_responses::SyncResponse,
    events::{AnyRoomEvent, AnySyncRoomEvent},
    identifiers::{RoomId, UserId},
    Client, ClientConfig, RequestConfig, SyncSettings,
};
use std::time::Duration;

use crate::session::categories::Categories;

mod imp {
    use super::*;
    use glib::subclass::{InitializingObject, Signal};
    use once_cell::sync::{Lazy, OnceCell};
    use std::cell::RefCell;
    use std::collections::HashMap;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/session.ui")]
    pub struct Session {
        #[template_child]
        pub sidebar: TemplateChild<Sidebar>,
        #[template_child]
        pub content: TemplateChild<Content>,
        pub homeserver: OnceCell<String>,
        /// Contains the error if something went wrong
        pub error: RefCell<Option<matrix_sdk::Error>>,
        pub client: OnceCell<Client>,
        pub rooms: RefCell<HashMap<RoomId, room::Room>>,
        pub categories: Categories,
        pub user: OnceCell<User>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Session {
        const NAME: &'static str = "Session";
        type Type = super::Session;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            klass.install_action(
                "session.show-room",
                Some("s"),
                move |widget, _, parameter| {
                    use std::convert::TryInto;
                    if let Some(room_id) = parameter
                        .and_then(|p| p.str())
                        .and_then(|s| s.try_into().ok())
                    {
                        widget.handle_show_room_action(room_id);
                    } else {
                        warn!("Not a valid room id: {:?}", parameter);
                    }
                },
            );
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for Session {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpec::new_string(
                        "homeserver",
                        "Homeserver",
                        "The matrix homeserver of this session",
                        None,
                        glib::ParamFlags::READWRITE | glib::ParamFlags::CONSTRUCT_ONLY,
                    ),
                    glib::ParamSpec::new_object(
                        "categories",
                        "Categories",
                        "A list of rooms grouped into categories",
                        Categories::static_type(),
                        glib::ParamFlags::READABLE,
                    ),
                ]
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
                "homeserver" => {
                    let homeserver = value
                        .get()
                        .expect("type conformity checked by `Object::set_property`");
                    let _ = self.homeserver.set(homeserver);
                }
                _ => unimplemented!(),
            }
        }

        fn property(&self, _obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "homeserver" => self.homeserver.get().to_value(),
                "categories" => self.categories.to_value(),
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
    impl WidgetImpl for Session {}
    impl BinImpl for Session {}
}

/// Enum containing the supported methods to create a `Session`.
#[derive(Clone, Debug)]
enum CreationMethod {
    /// Restore a previous session: `matrix_sdk::Session`
    SessionRestore(matrix_sdk::Session),
    /// Password Login: `username`, 'password`
    Password(String, String),
}

glib::wrapper! {
    pub struct Session(ObjectSubclass<imp::Session>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl Session {
    pub fn new(homeserver: String) -> Self {
        glib::Object::new(&[("homeserver", &homeserver)]).expect("Failed to create Session")
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
        let priv_ = imp::Session::from_instance(self);
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
        let room_sender = self.create_new_sync_response_sender();

        RUNTIME.spawn(async move {
            let success = match method {
                CreationMethod::SessionRestore(session) => {
                    let res = client.restore_login(session).await;
                    let success = res.is_ok();
                    let user_id = client.user_id().await.unwrap();
                    send!(sender, res.map(|_| (user_id, None)));
                    success
                }
                CreationMethod::Password(username, password) => {
                    let response = client
                        .login(&username, &password, None, Some("Fractal Next"))
                        .await;
                    let success = response.is_ok();
                    let user_id = client.user_id().await.unwrap();
                    send!(sender, response.map(|r| (user_id, Some(r))));
                    success
                }
            };

            if success {
                // TODO: only create the filter once and reuse it in the future
                let filter = assign!(FilterDefinition::default(), {
                    room: assign!(RoomFilter::empty(), {
                        include_leave: true,
                        timeline: assign!(RoomEventFilter::default(), {
                            lazy_load_options: LazyLoadOptions::Enabled {include_redundant_members: false},
                        }),
                    }),
                });

                let sync_settings = SyncSettings::new()
                    .timeout(Duration::from_secs(30))
                    .filter(filter.into());
                client
                    .sync_with_callback(sync_settings, |response| {
                        let room_sender = room_sender.clone();
                        async move {
                            // Using the event hanlder doesn't make a lot of sense for us since we want every room event
                            // Eventually we should contribute a better EventHandler interface so that it makes sense to use it.
                            room_sender.send(response).unwrap();

                            matrix_sdk::LoopCtrl::Continue
                        }
                    })
                    .await;
            }
        });
    }

    fn setup(&self) -> glib::SyncSender<matrix_sdk::Result<(UserId, Option<login::Response>)>> {
        let (sender, receiver) = glib::MainContext::sync_channel::<
            matrix_sdk::Result<(UserId, Option<login::Response>)>,
        >(Default::default(), 100);
        receiver.attach(
            None,
            clone!(@weak self as obj => @default-return glib::Continue(false), move |result| {
                match result {
                    Err(error) => {
                        let priv_ = &imp::Session::from_instance(&obj);
                        priv_.error.replace(Some(error));
                    }
                    Ok((user_id, Some(response))) => {
                        let session = matrix_sdk::Session {
                            access_token: response.access_token,
                            user_id: response.user_id,
                            device_id: response.device_id,
                        };
                        obj.set_user(User::new(&user_id));

                        //TODO: set error to this error
                        obj.store_session(session).unwrap();
                    }
                    Ok((user_id, None)) => {
                        obj.set_user(User::new(&user_id));
                    }
                }

                obj.load();

                obj.emit_by_name("ready", &[]).unwrap();

                glib::Continue(false)
            }),
        );
        sender
    }

    fn set_user(&self, user: User) {
        let priv_ = &imp::Session::from_instance(self);
        priv_.user.set(user).unwrap();
    }

    fn user(&self) -> &User {
        let priv_ = &imp::Session::from_instance(self);
        priv_.user.get().unwrap()
    }

    /// Sets up the required channel to receive new room events
    fn create_new_sync_response_sender(&self) -> SyncSender<SyncResponse> {
        let (sender, receiver) =
            glib::MainContext::sync_channel::<SyncResponse>(Default::default(), 100);
        receiver.attach(
            None,
            clone!(@weak self as obj => @default-return glib::Continue(false), move |response| {
                obj.handle_sync_response(response);
                glib::Continue(true)
            }),
        );

        sender
    }

    /// Loads the state from the `Store`
    /// Note that the `Store` currently doesn't store all events, therefore, we arn't really
    /// loading much via this function.
    pub fn load(&self) {
        // TODO: load rooms from the store before the sync completes
    }

    /// Returns and consumes the `error` that was generated when the session failed to login,
    /// on a successful login this will be `None`.
    /// Unfortunatly it's not possible to connect the Error direclty to the `ready` signals.
    pub fn get_error(&self) -> Option<matrix_sdk::Error> {
        let priv_ = &imp::Session::from_instance(self);
        priv_.error.take()
    }

    pub fn connect_ready<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_local("ready", true, move |values| {
            let obj = values[0].get::<Self>().unwrap();

            f(&obj);

            None
        })
        .unwrap()
    }

    fn store_session(&self, session: matrix_sdk::Session) -> Result<(), secret_service::Error> {
        let priv_ = &imp::Session::from_instance(self);
        let homeserver = priv_.homeserver.get().unwrap();
        secret::store_session(homeserver, session)
    }

    fn handle_show_room_action(&self, room_id: RoomId) {
        let priv_ = imp::Session::from_instance(self);
        let room = priv_.rooms.borrow().get(&room_id).cloned();
        priv_.content.set_room(room);
    }

    fn handle_sync_response(&self, response: SyncResponse) {
        let priv_ = imp::Session::from_instance(self);

        let new_rooms_id: Vec<RoomId> = {
            let rooms_map = priv_.rooms.borrow();

            let new_left_rooms = response.rooms.leave.iter().filter_map(|(room_id, _)| {
                if !rooms_map.contains_key(room_id) {
                    Some(room_id)
                } else {
                    None
                }
            });

            let new_joined_rooms = response.rooms.join.iter().filter_map(|(room_id, _)| {
                if !rooms_map.contains_key(room_id) {
                    Some(room_id)
                } else {
                    None
                }
            });
            new_joined_rooms.chain(new_left_rooms).cloned().collect()
        };

        let mut new_rooms = Vec::new();
        let mut rooms_map = priv_.rooms.borrow_mut();

        for room_id in new_rooms_id {
            if let Some(matrix_room) = priv_.client.get().unwrap().get_room(&room_id) {
                let room = room::Room::new(matrix_room, self.user());
                rooms_map.insert(room_id.clone(), room.clone());
                new_rooms.push(room.clone());
            }
        }

        priv_.categories.append(new_rooms);

        for (room_id, matrix_room) in response.rooms.leave {
            if matrix_room.timeline.events.is_empty() {
                continue;
            }
            if let Some(room) = rooms_map.get(&room_id) {
                room.append_events(
                    matrix_room
                        .timeline
                        .events
                        .into_iter()
                        .map(|event| event_from_sync_event!(event, room_id))
                        .collect(),
                );
            }
        }

        for (room_id, matrix_room) in response.rooms.join {
            if matrix_room.timeline.events.is_empty() {
                continue;
            }

            if let Some(room) = rooms_map.get(&room_id) {
                room.append_events(
                    matrix_room
                        .timeline
                        .events
                        .into_iter()
                        .map(|event| event_from_sync_event!(event, room_id))
                        .collect(),
                );
            }
        }

        // TODO: handle StrippedStateEvents for invited rooms
    }
}
