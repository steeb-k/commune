mod account_settings;
mod avatar;
mod content;
mod event_source_dialog;
mod media_viewer;
pub mod room;
mod room_creation;
mod room_list;
mod sidebar;
mod user;
pub mod verification;

use self::account_settings::AccountSettings;
pub use self::avatar::Avatar;
use self::content::verification::SessionVerification;
use self::content::Content;
use self::media_viewer::MediaViewer;
pub use self::room::{Event, Item, Room};
pub use self::room_creation::RoomCreation;
use self::room_list::RoomList;
use self::sidebar::Sidebar;
pub use self::user::{User, UserExt};
use self::verification::{IdentityVerification, VerificationList, VerificationMode};
use crate::session::sidebar::ItemList;

use crate::secret;
use crate::secret::Secret;
use crate::secret::StoredSession;
use crate::Error;
use crate::UserFacingError;
use crate::Window;
use crate::{spawn, spawn_tokio};

use adw::subclass::prelude::BinImpl;
use futures::StreamExt;
use gettextrs::gettext;
use gtk::subclass::prelude::*;
use gtk::{self, prelude::*};
use gtk::{
    gdk, glib, glib::clone, glib::source::SourceId, glib::SyncSender, CompositeTemplate,
    SelectionModel,
};
use log::{debug, error, warn};
use matrix_sdk::ruma::{
    api::client::r0::{
        filter::{FilterDefinition, LazyLoadOptions, RoomEventFilter, RoomFilter},
        session::logout,
    },
    assign,
    identifiers::RoomId,
};
use matrix_sdk::{
    config::{ClientConfig, RequestConfig, SyncSettings},
    deserialized_responses::SyncResponse,
    ruma::api::{
        client::error::ErrorKind,
        error::{FromHttpResponseError, ServerError},
    },
    uuid::Uuid,
    Client, Error as MatrixError, HttpError,
};
use rand::{distributions::Alphanumeric, thread_rng, Rng};
use std::convert::TryFrom;
use std::fs;
use std::time::Duration;
use tokio::task::JoinHandle;
use url::Url;

mod imp {
    use super::*;
    use glib::subclass::{InitializingObject, Signal};
    use once_cell::{sync::Lazy, unsync::OnceCell};
    use std::cell::{Cell, RefCell};

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/session.ui")]
    pub struct Session {
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub content: TemplateChild<adw::Leaflet>,
        #[template_child]
        pub sidebar: TemplateChild<Sidebar>,
        #[template_child]
        pub media_viewer: TemplateChild<MediaViewer>,
        pub client: RefCell<Option<Client>>,
        pub item_list: OnceCell<ItemList>,
        pub user: OnceCell<User>,
        pub is_ready: Cell<bool>,
        pub logout_on_dispose: Cell<bool>,
        pub info: OnceCell<StoredSession>,
        pub source_id: RefCell<Option<SourceId>>,
        pub sync_tokio_handle: RefCell<Option<JoinHandle<()>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Session {
        const NAME: &'static str = "Session";
        type Type = super::Session;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);

            klass.install_action("session.close-room", None, move |session, _, _| {
                session.select_room(None);
            });

            klass.install_action(
                "session.show-room",
                Some("s"),
                move |session, _, parameter| {
                    if let Ok(room_id) =
                        RoomId::try_from(parameter.unwrap().get::<String>().unwrap())
                    {
                        session.select_room_by_id(room_id);
                    } else {
                        error!("Can't show room because the provided id is invalid");
                    }
                },
            );

            klass.install_action("session.logout", None, move |session, _, _| {
                spawn!(clone!(@weak session => async move {
                    let priv_ = imp::Session::from_instance(&session);
                    priv_.logout_on_dispose.set(false);
                    session.logout().await
                }));
            });

            klass.install_action("session.show-content", None, move |session, _, _| {
                session.show_content();
            });

            klass.install_action("session.room-creation", None, move |session, _, _| {
                session.show_room_creation_dialog();
            });

            klass.add_binding_action(
                gdk::keys::constants::Escape,
                gdk::ModifierType::empty(),
                "session.close-room",
                None,
            );

            klass.install_action("session.toggle-room-search", None, move |session, _, _| {
                session.toggle_room_search();
            });

            klass.add_binding_action(
                gdk::keys::constants::k,
                gdk::ModifierType::CONTROL_MASK,
                "session.toggle-room-search",
                None,
            );

            klass.install_action(
                "session.open-account-settings",
                None,
                move |widget, _, _| {
                    widget.open_account_settings();
                },
            );
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            Sidebar::static_type();
            Content::static_type();
            obj.init_template();
        }
    }

    impl ObjectImpl for Session {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpec::new_object(
                        "item-list",
                        "Item List",
                        "The list of items in the sidebar",
                        ItemList::static_type(),
                        glib::ParamFlags::READABLE,
                    ),
                    glib::ParamSpec::new_object(
                        "user",
                        "User",
                        "The user of this session",
                        User::static_type(),
                        glib::ParamFlags::READABLE,
                    ),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "item-list" => obj.item_list().to_value(),
                "user" => obj.user().to_value(),
                _ => unimplemented!(),
            }
        }

        fn signals() -> &'static [Signal] {
            static SIGNALS: Lazy<Vec<Signal>> = Lazy::new(|| {
                vec![
                    Signal::builder(
                        "prepared",
                        &[Option::<Error>::static_type().into()],
                        <()>::static_type().into(),
                    )
                    .build(),
                    Signal::builder("ready", &[], <()>::static_type().into()).build(),
                    Signal::builder("logged-out", &[], <()>::static_type().into()).build(),
                ]
            });
            SIGNALS.as_ref()
        }

        fn constructed(&self, obj: &Self::Type) {
            self.parent_constructed(obj);

            self.sidebar.connect_notify_local(
                Some("selected-item"),
                clone!(@weak obj => move |_, _| {
                    let priv_ = imp::Session::from_instance(&obj);

                    if priv_.sidebar.selected_item().is_none() {
                        priv_.content.navigate(adw::NavigationDirection::Back);
                    } else {
                        priv_.content.navigate(adw::NavigationDirection::Forward);
                    }
                }),
            );
        }

        fn dispose(&self, obj: &Self::Type) {
            if let Some(source_id) = self.source_id.take() {
                let _ = glib::Source::remove(source_id);
            }

            if let Some(handle) = self.sync_tokio_handle.take() {
                handle.abort();
            }

            if self.logout_on_dispose.get() {
                glib::MainContext::default().block_on(obj.logout());
            }
        }
    }
    impl WidgetImpl for Session {}
    impl BinImpl for Session {}
}

glib::wrapper! {
    pub struct Session(ObjectSubclass<imp::Session>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl Session {
    pub fn new() -> Self {
        glib::Object::new(&[]).expect("Failed to create Session")
    }

    pub fn select_room(&self, room: Option<Room>) {
        let priv_ = imp::Session::from_instance(self);
        priv_
            .sidebar
            .set_selected_item(room.map(|item| item.upcast()));
    }

    pub fn select_room_by_id(&self, room_id: RoomId) {
        if let Some(room) = self.room_list().get(&room_id) {
            self.select_room(Some(room));
        } else {
            warn!("A room with id {} couldn't be found", room_id);
        }
    }

    pub fn login_with_password(&self, homeserver: Url, username: String, password: String) {
        let priv_ = imp::Session::from_instance(self);
        priv_.logout_on_dispose.set(true);
        let mut path = glib::user_data_dir();
        path.push(
            &Uuid::new_v4()
                .to_hyphenated()
                .encode_lower(&mut Uuid::encode_buffer()),
        );

        let handle = spawn_tokio!(async move {
            let passphrase: String = {
                let mut rng = thread_rng();
                (&mut rng)
                    .sample_iter(Alphanumeric)
                    .take(30)
                    .map(char::from)
                    .collect()
            };
            let config = ClientConfig::new()
                .request_config(RequestConfig::new().retry_limit(2))
                .passphrase(passphrase.clone())
                .store_path(path.clone());

            let client = Client::new_with_config(homeserver.clone(), config).unwrap();
            let response = client
                .login(&username, &password, None, Some("Fractal Next"))
                .await;
            match response {
                Ok(response) => Ok((
                    client,
                    StoredSession {
                        homeserver,
                        path,
                        user_id: response.user_id,
                        device_id: response.device_id,
                        secret: Secret {
                            passphrase,
                            access_token: response.access_token,
                        },
                    },
                )),
                Err(error) => {
                    // Remove the store created by Client::new()
                    fs::remove_dir_all(path).unwrap();
                    Err(error)
                }
            }
        });

        spawn!(
            glib::PRIORITY_DEFAULT_IDLE,
            clone!(@weak self as obj => async move {
                obj.handle_login_result(handle.await.unwrap(), true);
            })
        );
    }

    fn toggle_room_search(&self) {
        let priv_ = imp::Session::from_instance(self);
        let room_search = priv_.sidebar.room_search_bar();
        room_search.set_search_mode(!room_search.is_search_mode());
    }

    pub fn login_with_previous_session(&self, session: StoredSession) {
        let handle = spawn_tokio!(async move {
            let config = ClientConfig::new()
                .request_config(RequestConfig::new().retry_limit(2))
                .passphrase(session.secret.passphrase.clone())
                .store_path(session.path.clone());

            let client = Client::new_with_config(session.homeserver.clone(), config).unwrap();
            client
                .restore_login(matrix_sdk::Session {
                    user_id: session.user_id.clone(),
                    device_id: session.device_id.clone(),
                    access_token: session.secret.access_token.clone(),
                })
                .await
                .map(|_| (client, session))
        });

        spawn!(
            glib::PRIORITY_DEFAULT_IDLE,
            clone!(@weak self as obj => async move {
                obj.handle_login_result(handle.await.unwrap(), false);
            })
        );
    }

    fn handle_login_result(
        &self,
        result: Result<(Client, StoredSession), matrix_sdk::Error>,
        store_session: bool,
    ) {
        let priv_ = imp::Session::from_instance(self);
        let error = match result {
            Ok((client, session)) => {
                priv_.client.replace(Some(client.clone()));
                let user = User::new(self, &session.user_id);
                priv_.user.set(user.clone()).unwrap();
                self.notify("user");

                let handle = spawn_tokio!(async move {
                    let display_name = client.display_name().await?;
                    let avatar_url = client.avatar_url().await?;
                    let result: Result<_, MatrixError> = Ok((display_name, avatar_url));
                    result
                });

                spawn!(glib::PRIORITY_LOW, async move {
                    match handle.await.unwrap() {
                        Ok((display_name, avatar_url)) => {
                            user.set_display_name(display_name);
                            user.set_avatar_url(avatar_url);
                        }
                        Err(error) => error!("Couldn’t fetch account metadata: {}", error),
                    }
                });

                let res = if store_session {
                    match secret::store_session(&session) {
                        Ok(()) => None,
                        Err(error) => {
                            warn!("Couldn't store session: {:?}", error);
                            let error_string = error.to_user_facing();
                            Some(Error::new(move |_| {
                                let error_label = gtk::LabelBuilder::new()
                                    .label(
                                        &(gettext("Unable to store session")
                                            + ": "
                                            + &error_string),
                                    )
                                    .wrap(true)
                                    .build();
                                Some(error_label.upcast())
                            }))
                        }
                    }
                } else {
                    None
                };

                priv_.info.set(session).unwrap();

                self.room_list().load();

                self.sync();

                res
            }
            Err(error) => {
                error!("Failed to prepare the session: {}", error);

                priv_.logout_on_dispose.set(false);

                let error_string = error.to_user_facing();

                Some(Error::new(move |_| {
                    let error_label = gtk::LabelBuilder::new()
                        .label(&error_string)
                        .wrap(true)
                        .build();
                    Some(error_label.upcast())
                }))
            }
        };

        self.emit_by_name("prepared", &[&error]).unwrap();
    }

    fn sync(&self) {
        let priv_ = imp::Session::from_instance(self);
        let sender = self.create_new_sync_response_sender();
        let client = self.client();
        let handle = spawn_tokio!(async move {
            // TODO: only create the filter once and reuse it in the future
            let room_event_filter = assign!(RoomEventFilter::default(), {
                lazy_load_options: LazyLoadOptions::Enabled {include_redundant_members: false},
            });
            let filter = assign!(FilterDefinition::default(), {
                room: assign!(RoomFilter::empty(), {
                    include_leave: true,
                    state: room_event_filter,
                }),
            });

            let sync_settings = SyncSettings::new()
                .timeout(Duration::from_secs(30))
                .filter(filter.into());

            // We need to automatically restart the stream because it gets killed on error
            loop {
                let mut sync_stream = Box::pin(client.sync_stream(sync_settings.clone()).await);
                while let Some(response) = sync_stream.next().await {
                    if sender.send(response).is_err() {
                        debug!("Stop syncing because the session was disposed");
                        return;
                    }
                }
            }
        });

        priv_.sync_tokio_handle.replace(Some(handle));
    }

    async fn create_session_verification(&self) {
        let priv_ = imp::Session::from_instance(self);

        let request = IdentityVerification::create(&self, self.user().unwrap()).await;

        if let Some(widget) = priv_.stack.child_by_name("session-verification") {
            widget
                .downcast::<SessionVerification>()
                .unwrap()
                .set_request(request.clone());
        } else {
            let widget = SessionVerification::new(&request);
            priv_.stack.add_named(&widget, Some("session-verification"));
            priv_.stack.set_visible_child(&widget);
        }

        request.connect_notify_local(
            Some("mode"),
            clone!(@weak self as obj => move |request, _| {
                if request.is_finished() && request.mode() !=  VerificationMode::Completed {
                    spawn!(async move {
                        obj.create_session_verification().await;
                    });
                }
            }),
        );

        self.verification_list().add(request);
    }

    fn mark_ready(&self) {
        let priv_ = imp::Session::from_instance(self);
        let client = self.client();
        let user_id = self.user().unwrap().user_id().to_owned();

        priv_.is_ready.set(true);

        let has_cross_signing_keys = spawn_tokio!(async move {
            if let Some(cross_signing_status) = client.cross_signing_status().await {
                cross_signing_status.has_master
                    && cross_signing_status.has_self_signing
                    && cross_signing_status.has_user_signing
            } else {
                false
            }
        });

        let client = self.client();
        let need_new_identity = spawn_tokio!(async move {
            // If there is an error just assume we don't need a new identity since
            // we will try again during the session verification
            client
                .get_user_identity(&user_id)
                .await
                .map_or(false, |identity| identity.is_none())
        });

        spawn!(clone!(@weak self as obj => async move {
            let priv_ = imp::Session::from_instance(&obj);
            if !has_cross_signing_keys.await.unwrap() {
                if need_new_identity.await.unwrap() {
                    let client = obj.client();
                    if spawn_tokio!(async move { client.bootstrap_cross_signing(None).await }).await.is_ok() {
                        priv_.stack.set_visible_child(&*priv_.content);
                        return;
                    }
                }

                priv_.logout_on_dispose.set(true);
                obj.create_session_verification().await;

                return;
            }

            obj.show_content();
        }));
    }

    fn is_ready(&self) -> bool {
        let priv_ = &imp::Session::from_instance(self);
        priv_.is_ready.get()
    }

    pub fn room_list(&self) -> &RoomList {
        self.item_list().room_list()
    }

    pub fn verification_list(&self) -> &VerificationList {
        self.item_list().verification_list()
    }

    pub fn item_list(&self) -> &ItemList {
        let priv_ = &imp::Session::from_instance(self);
        priv_
            .item_list
            .get_or_init(|| ItemList::new(&RoomList::new(self), &VerificationList::new(self)))
    }

    pub fn user(&self) -> Option<&User> {
        let priv_ = &imp::Session::from_instance(self);
        priv_.user.get()
    }

    pub fn client(&self) -> Client {
        let priv_ = &imp::Session::from_instance(self);
        priv_
            .client
            .borrow()
            .clone()
            .expect("The session isn't ready")
    }

    /// Sets up the required channel to receive new room events
    fn create_new_sync_response_sender(
        &self,
    ) -> SyncSender<Result<SyncResponse, matrix_sdk::Error>> {
        let priv_ = imp::Session::from_instance(self);
        let (sender, receiver) = glib::MainContext::sync_channel::<
            Result<SyncResponse, matrix_sdk::Error>,
        >(Default::default(), 100);
        let source_id = receiver.attach(
            None,
            clone!(@weak self as obj => @default-return glib::Continue(false), move |response| {
                obj.handle_sync_response(response);

                glib::Continue(true)
            }),
        );

        priv_.source_id.replace(Some(source_id));

        sender
    }

    /// Connects the prepared signals to the function f given in input
    pub fn connect_prepared<F: Fn(&Self, Option<Error>) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_local("prepared", true, move |values| {
            let obj = values[0].get::<Self>().unwrap();
            let err = values[1].get::<Option<Error>>().unwrap();

            f(&obj, err);

            None
        })
        .unwrap()
    }

    pub fn connect_logged_out<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_local("logged-out", true, move |values| {
            let obj = values[0].get::<Self>().unwrap();

            f(&obj);

            None
        })
        .unwrap()
    }

    pub fn connect_ready<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_local("ready", true, move |values| {
            let obj = values[0].get::<Self>().unwrap();

            f(&obj);

            None
        })
        .unwrap()
    }

    fn handle_sync_response(&self, response: Result<SyncResponse, matrix_sdk::Error>) {
        match response {
            Ok(response) => {
                if !self.is_ready() {
                    self.mark_ready();
                }
                self.room_list().handle_response_rooms(response.rooms);
                self.verification_list()
                    .handle_response_to_device(response.to_device);
            }
            Err(error) => {
                if let matrix_sdk::Error::Http(HttpError::ClientApi(FromHttpResponseError::Http(
                    ServerError::Known(ref error),
                ))) = error
                {
                    match error.kind {
                        ErrorKind::UnknownToken { soft_logout: _ } => {
                            self.emit_by_name("logged-out", &[]).unwrap();
                            self.cleanup_session();
                        }
                        _ => {}
                    }
                }
                error!("Failed to perform sync: {:?}", error);
            }
        }
    }

    pub fn set_logged_in_users(&self, sessions_stack_pages: &SelectionModel) {
        let priv_ = &imp::Session::from_instance(self);
        priv_
            .sidebar
            .set_logged_in_users(sessions_stack_pages, self);
    }

    /// Returns the parent GtkWindow containing this widget.
    fn parent_window(&self) -> Option<Window> {
        self.root()?.downcast().ok()
    }

    fn open_account_settings(&self) {
        if let Some(user) = self.user() {
            let window = AccountSettings::new(self.parent_window().as_ref(), user);
            window.show();
        }
    }

    fn show_room_creation_dialog(&self) {
        let window = RoomCreation::new(self.parent_window().as_ref(), self);
        window.show();
    }

    pub async fn logout(&self) {
        let priv_ = imp::Session::from_instance(self);
        self.emit_by_name("logged-out", &[]).unwrap();

        debug!("The session is about to be logout");

        // First stop the verification in progress
        if let Some(session_verificiation) = priv_.stack.child_by_name("session-verification") {
            priv_.stack.remove(&session_verificiation);
            session_verificiation
                .downcast_ref::<SessionVerification>()
                .unwrap()
                .request()
                .cancel();
        }

        let client = self.client();
        let handle = spawn_tokio!(async move {
            let request = logout::Request::new();
            client.send(request, None).await
        });

        match handle.await.unwrap() {
            Ok(_) => self.cleanup_session(),
            Err(error) => {
                error!("Couldn’t logout the session {}", error);
                let error = Error::new(move |_| {
                    let label = gtk::Label::new(Some(&gettext("Failed to logout the session.")));
                    Some(label.upcast())
                });

                if let Some(window) = self.parent_window() {
                    window.append_error(&error);
                }
            }
        }
    }

    fn cleanup_session(&self) {
        let priv_ = imp::Session::from_instance(self);
        let info = priv_.info.get().unwrap();

        priv_.is_ready.set(false);

        if let Some(source_id) = priv_.source_id.take() {
            let _ = glib::Source::remove(source_id);
        }

        if let Some(handle) = priv_.sync_tokio_handle.take() {
            handle.abort();
        }

        if let Err(error) = secret::remove_session(info) {
            error!(
                "Failed to remove credentials from SecretService after logout: {}",
                error
            );
        }

        if let Err(error) = fs::remove_dir_all(info.path.clone()) {
            error!("Failed to remove database after logout: {}", error);
        }

        debug!("The logged out session was cleaned up");
    }

    /// Show the content of the session
    pub fn show_content(&self) {
        let priv_ = imp::Session::from_instance(self);

        // FIXME: we should actually check if we have now the keys
        priv_.stack.set_visible_child(&*priv_.content);
        priv_.logout_on_dispose.set(false);

        if let Some(session_verificiation) = priv_.stack.child_by_name("session-verification") {
            priv_.stack.remove(&session_verificiation);
        }

        self.emit_by_name("ready", &[]).unwrap();
    }

    /// Show a media event
    pub fn show_media(&self, event: &Event) {
        let priv_ = imp::Session::from_instance(self);
        priv_.media_viewer.set_event(Some(event.clone()));

        priv_.stack.set_visible_child(&*priv_.media_viewer);
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}
