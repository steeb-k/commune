use super::{VERIFICATION_CREATION_TIMEOUT, VERIFICATION_RECEIVE_TIMEOUT};
use crate::session::user::UserExt;
use crate::session::Session;
use crate::session::User;
use crate::spawn;
use crate::spawn_tokio;
use crate::Error;
use gettextrs::gettext;
use gtk::{glib, glib::clone, prelude::*, subclass::prelude::*};
use log::error;
use log::warn;
use matrix_sdk::{
    encryption::{
        identities::RequestVerificationError,
        verification::{
            CancelInfo, Emoji, QrVerificationData, SasVerification, Verification,
            VerificationRequest,
        },
    },
    ruma::{
        events::key::verification::{cancel::CancelCode, VerificationMethod},
        identifiers::UserId,
    },
    Client,
};
use qrcode::QrCode;
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Debug, Eq, PartialEq, Clone, Copy, glib::GEnum)]
#[repr(u32)]
#[genum(type_name = "VerificationState")]
pub enum State {
    Requested,
    RequestSend,
    SasV1,
    QrV1Show,
    QrV1Scan,
    Completed,
    Cancelled,
    Dismissed,
    Passive,
    Error,
}

impl Default for State {
    fn default() -> Self {
        Self::Requested
    }
}

#[derive(Debug, Eq, PartialEq, Clone, Copy, glib::GEnum)]
#[repr(u32)]
#[genum(type_name = "VerificationMode")]
pub enum Mode {
    CurrentSession,
    OtherSession,
    User,
}

impl Default for Mode {
    fn default() -> Self {
        Self::User
    }
}

#[glib::gflags("VerificationSupportedMethods")]
pub enum SupportedMethods {
    NONE = 0b00000000,
    SAS = 0b00000001,
    QR_SHOW = 0b00000010,
    QR_SCAN = 0b00000100,
}

impl From<VerificationMethod> for SupportedMethods {
    fn from(method: VerificationMethod) -> Self {
        match method {
            VerificationMethod::SasV1 => Self::SAS,
            VerificationMethod::QrCodeScanV1 => Self::QR_SHOW,
            VerificationMethod::QrCodeShowV1 => Self::QR_SCAN,
            _ => Self::NONE,
        }
    }
}

impl Default for SupportedMethods {
    fn default() -> Self {
        Self::NONE
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum UserAction {
    Accept,
    Match,
    NotMatch,
    Cancel,
    StartSas,
    Scanned(QrVerificationData),
}

#[derive(Debug, PartialEq)]
pub enum Message {
    UserAction(UserAction),
    NotifyState,
}

pub enum MainMessage {
    QrCode(QrCode),
    SasData(SasData),
    SupportedMethods(SupportedMethods),
    CancelInfo(CancelInfo),
    State(State),
}

#[derive(Debug)]
pub enum SasData {
    Emoji([Emoji; 7]),
    Decimal((u16, u16, u16)),
}

mod imp {
    use super::*;
    use glib::object::WeakRef;
    use once_cell::{sync::Lazy, unsync::OnceCell};
    use std::cell::{Cell, RefCell};

    #[derive(Default)]
    pub struct IdentityVerification {
        pub user: OnceCell<User>,
        pub session: OnceCell<WeakRef<Session>>,
        pub state: Cell<State>,
        pub mode: OnceCell<Mode>,
        pub supported_methods: Cell<SupportedMethods>,
        pub sync_sender: RefCell<Option<mpsc::Sender<Message>>>,
        pub main_sender: RefCell<Option<glib::SyncSender<MainMessage>>>,
        pub sas_data: OnceCell<SasData>,
        pub qr_code: OnceCell<QrCode>,
        pub cancel_info: OnceCell<CancelInfo>,
        pub flow_id: OnceCell<String>,
        pub start_time: OnceCell<glib::DateTime>,
        pub receive_time: OnceCell<glib::DateTime>,
        pub hide_error: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for IdentityVerification {
        const NAME: &'static str = "IdentityVerification";
        type Type = super::IdentityVerification;
        type ParentType = glib::Object;
    }

    impl ObjectImpl for IdentityVerification {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpec::new_object(
                        "user",
                        "User",
                        "The user to be verified",
                        User::static_type(),
                        glib::ParamFlags::READWRITE | glib::ParamFlags::CONSTRUCT_ONLY,
                    ),
                    glib::ParamSpec::new_object(
                        "session",
                        "Session",
                        "The current session",
                        Session::static_type(),
                        glib::ParamFlags::READWRITE | glib::ParamFlags::CONSTRUCT_ONLY,
                    ),
                    glib::ParamSpec::new_enum(
                        "state",
                        "State",
                        "The current state of this verification",
                        State::static_type(),
                        State::default() as i32,
                        glib::ParamFlags::READWRITE | glib::ParamFlags::CONSTRUCT_ONLY,
                    ),
                    glib::ParamSpec::new_enum(
                        "mode",
                        "Mode",
                        "The mode of this verification",
                        Mode::static_type(),
                        Mode::default() as i32,
                        glib::ParamFlags::READWRITE | glib::ParamFlags::CONSTRUCT_ONLY,
                    ),
                    glib::ParamSpec::new_flags(
                        "supported-methods",
                        "Supported Methods",
                        "The supported methods of this verification",
                        SupportedMethods::static_type(),
                        SupportedMethods::default().bits(),
                        glib::ParamFlags::READABLE | glib::ParamFlags::EXPLICIT_NOTIFY,
                    ),
                    glib::ParamSpec::new_string(
                        "display-name",
                        "Display name",
                        "The display name of this verificaiton request",
                        None,
                        glib::ParamFlags::READABLE | glib::ParamFlags::EXPLICIT_NOTIFY,
                    ),
                    glib::ParamSpec::new_string(
                        "flow-id",
                        "Flow Id",
                        "The flow id of this verification request",
                        None,
                        glib::ParamFlags::READWRITE | glib::ParamFlags::CONSTRUCT_ONLY,
                    ),
                    glib::ParamSpec::new_boxed(
                        "start-time",
                        "Start Time",
                        "The time when this verification request was started",
                        glib::DateTime::static_type(),
                        glib::ParamFlags::READWRITE | glib::ParamFlags::CONSTRUCT_ONLY,
                    ),
                    glib::ParamSpec::new_boxed(
                        "receive-time",
                        "Receive Time",
                        "The time when this verification request was received",
                        glib::DateTime::static_type(),
                        glib::ParamFlags::READABLE,
                    ),
                ]
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
                "user" => obj.set_user(value.get().unwrap()),
                "session" => obj.set_session(value.get().unwrap()),
                "state" => obj.set_state(value.get().unwrap()),
                "mode" => obj.set_mode(value.get().unwrap()),
                "flow-id" => obj.set_flow_id(value.get().unwrap()),
                "start-time" => obj.set_start_time(value.get().unwrap()),
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "user" => obj.user().to_value(),
                "session" => obj.session().to_value(),
                "state" => obj.state().to_value(),
                "mode" => obj.mode().to_value(),
                "display-name" => obj.display_name().to_value(),
                "flow-id" => obj.flow_id().to_value(),
                "supported-methods" => obj.supported_methods().to_value(),
                "start-time" => obj.start_time().to_value(),
                "receive-time" => obj.receive_time().to_value(),
                _ => unimplemented!(),
            }
        }

        fn constructed(&self, obj: &Self::Type) {
            self.parent_constructed(obj);

            let (main_sender, main_receiver) =
                glib::MainContext::sync_channel::<MainMessage>(Default::default(), 100);

            main_receiver.attach(
                None,
                clone!(@weak obj => @default-return glib::Continue(false), move |message| {
                    let priv_ = imp::IdentityVerification::from_instance(&obj);
                    match message {
                        MainMessage::QrCode(data) => { let _ = priv_.qr_code.set(data); },
                        MainMessage::CancelInfo(data) => priv_.cancel_info.set(data).unwrap(),
                        MainMessage::SasData(data) => priv_.sas_data.set(data).unwrap(),
                        MainMessage::SupportedMethods(flags) => priv_.supported_methods.set(flags),
                        MainMessage::State(state) => obj.set_state(state),
                    }

                    glib::Continue(true)
                }),
            );

            self.main_sender.replace(Some(main_sender));

            // We don't need to track ourselfs because we show "Login Request" as name in that case.
            if obj.user() != obj.session().user().unwrap() {
                obj.user().connect_notify_local(
                    Some("display-name"),
                    clone!(@weak obj => move |_, _| {
                        obj.notify("display-name");
                    }),
                );
            }

            self.receive_time
                .set(glib::DateTime::new_now_local().unwrap())
                .unwrap();
            obj.setup_timeout();
            obj.start_handler();
        }

        fn dispose(&self, obj: &Self::Type) {
            obj.cancel(true);
        }
    }
}

glib::wrapper! {
    pub struct IdentityVerification(ObjectSubclass<imp::IdentityVerification>);
}

impl IdentityVerification {
    fn for_error(mode: Mode, session: &Session, user: &User, start_time: &glib::DateTime) -> Self {
        glib::Object::new(&[
            ("state", &State::Error),
            ("mode", &mode),
            ("session", session),
            ("user", user),
            ("start-time", start_time),
        ])
        .expect("Failed to create IdentityVerification")
    }

    /// Create a new object tracking an already existing verification request
    pub fn for_flow_id(
        flow_id: &str,
        session: &Session,
        user: &User,
        start_time: &glib::DateTime,
    ) -> Self {
        glib::Object::new(&[
            ("flow-id", &flow_id),
            ("session", session),
            ("user", user),
            ("start-time", start_time),
        ])
        .expect("Failed to create IdentityVerification")
    }

    /// Creates and send a new verificaiton request
    ///
    /// If `User` is `None` a new session verification is started for our own user and send to other devices
    pub async fn create(session: &Session, user: Option<&User>) -> Self {
        let (mode, user) = if let Some(user) = user {
            (Mode::User, user)
        } else {
            (Mode::CurrentSession, session.user().unwrap())
        };

        if let Some(identity) = user.crypto_identity().await {
            let handle = spawn_tokio!(async move {
                identity
                    .request_verification_with_methods(vec![
                        VerificationMethod::SasV1,
                        VerificationMethod::QrCodeScanV1,
                        VerificationMethod::QrCodeShowV1,
                        VerificationMethod::ReciprocateV1,
                    ])
                    .await
            });

            match handle.await.unwrap() {
                Ok(request) => {
                    let obj = glib::Object::new(&[
                        ("state", &State::RequestSend),
                        ("mode", &mode),
                        ("flow-id", &request.flow_id()),
                        ("session", session),
                        ("user", user),
                        ("start-time", &glib::DateTime::new_now_local().unwrap()),
                    ])
                    .expect("Failed to create IdentityVerification");

                    return obj;
                }
                Err(error) => {
                    error!("Starting a verification failed: {}", error);
                }
            }
        } else {
            error!("Starting a verification failed: Crypto identity wasn't found");
        }

        Self::for_error(
            mode,
            session,
            user,
            &glib::DateTime::new_now_local().unwrap(),
        )
    }

    fn start_handler(&self) {
        let priv_ = imp::IdentityVerification::from_instance(self);

        let main_sender = if let Some(main_sender) = priv_.main_sender.take() {
            main_sender
        } else {
            warn!("The verification request was already started");
            return;
        };

        let client = self.session().client();
        let user_id = self.user().user_id().to_owned();
        let flow_id = self.flow_id().to_owned();

        let (sync_sender, sync_receiver) = mpsc::channel(100);
        priv_.sync_sender.replace(Some(sync_sender));

        let handle = spawn_tokio!(async move {
            if let Some(context) =
                Context::new(client, &user_id, &flow_id, main_sender, sync_receiver).await
            {
                context.start().await
            } else {
                Ok(State::Error)
            }
        });

        let weak_obj = self.downgrade();
        spawn!(async move {
            let result = handle.await.unwrap();
            if let Some(obj) = weak_obj.upgrade() {
                let priv_ = imp::IdentityVerification::from_instance(&obj);
                match result {
                    Ok(result) => obj.set_state(result),
                    Err(error) => {
                        // FIXME: report error to the user
                        error!("Verification failed: {}", error);
                        obj.set_state(State::Error);
                    }
                }
                priv_.sync_sender.take();
            }
        });
    }

    /// The user to be verified.
    pub fn user(&self) -> &User {
        let priv_ = imp::IdentityVerification::from_instance(self);
        priv_.user.get().unwrap()
    }

    fn set_user(&self, user: User) {
        let priv_ = imp::IdentityVerification::from_instance(self);
        priv_.user.set(user).unwrap()
    }

    /// The current `Session`.
    pub fn session(&self) -> Session {
        let priv_ = imp::IdentityVerification::from_instance(self);
        priv_.session.get().unwrap().upgrade().unwrap()
    }

    fn set_session(&self, session: Session) {
        let priv_ = imp::IdentityVerification::from_instance(self);
        priv_.session.set(session.downgrade()).unwrap()
    }

    fn setup_timeout(&self) {
        let difference = glib::DateTime::new_now_local()
            .unwrap()
            .difference(self.start_time());

        if difference < 0 {
            warn!("The verification request was sent in the future.");
            self.cancel(false);
            return;
        }
        let difference = Duration::from_secs(difference as u64);
        let remaining_creation = VERIFICATION_CREATION_TIMEOUT.saturating_sub(difference);

        let remaining_receive = VERIFICATION_RECEIVE_TIMEOUT.saturating_sub(difference);

        let remaining = std::cmp::max(remaining_creation, remaining_receive);

        if remaining.is_zero() {
            self.cancel(false);
            return;
        }

        glib::source::timeout_add_local(
            remaining,
            clone!(@weak self as obj => @default-return glib::Continue(false), move || {
                obj.cancel(false);

                glib::Continue(false)
            }),
        );
    }

    /// The time and date when this verification request was started.
    pub fn start_time(&self) -> &glib::DateTime {
        let priv_ = imp::IdentityVerification::from_instance(self);
        priv_.start_time.get().unwrap()
    }

    fn set_start_time(&self, time: glib::DateTime) {
        let priv_ = imp::IdentityVerification::from_instance(self);
        priv_.start_time.set(time).unwrap();
    }

    pub fn receive_time(&self) -> &glib::DateTime {
        let priv_ = imp::IdentityVerification::from_instance(self);
        priv_.receive_time.get().unwrap()
    }

    fn supported_methods(&self) -> SupportedMethods {
        let priv_ = imp::IdentityVerification::from_instance(self);
        priv_.supported_methods.get()
    }

    pub fn emoji_match(&self) {
        let priv_ = imp::IdentityVerification::from_instance(self);

        if let Some(sync_sender) = &*priv_.sync_sender.borrow() {
            let result = sync_sender.try_send(Message::UserAction(UserAction::Match));

            if let Err(error) = result {
                error!("Failed to send message to tokio runtime: {}", error);
            }
        }
    }

    pub fn emoji_not_match(&self) {
        let priv_ = imp::IdentityVerification::from_instance(self);

        if let Some(sync_sender) = &*priv_.sync_sender.borrow() {
            let result = sync_sender.try_send(Message::UserAction(UserAction::NotMatch));

            if let Err(error) = result {
                error!("Failed to send message to tokio runtime: {}", error);
            }
        }
    }

    pub fn state(&self) -> State {
        let priv_ = imp::IdentityVerification::from_instance(self);
        priv_.state.get()
    }

    fn set_state(&self, state: State) {
        let priv_ = imp::IdentityVerification::from_instance(self);

        if self.state() == state {
            return;
        }

        match state {
            State::Cancelled | State::Error => self.show_error(),
            _ => {}
        }

        priv_.state.set(state);
        self.notify("state");
    }

    pub fn mode(&self) -> Mode {
        let priv_ = imp::IdentityVerification::from_instance(self);
        *priv_.mode.get_or_init(|| {
            let session = self.session();
            let our_user = session.user().unwrap();
            if our_user.user_id() == self.user().user_id() {
                Mode::OtherSession
            } else {
                Mode::User
            }
        })
    }

    fn set_mode(&self, mode: Mode) {
        let priv_ = imp::IdentityVerification::from_instance(self);

        priv_.mode.set(mode).unwrap();
    }

    /// Whether this request is finished
    pub fn is_finished(&self) -> bool {
        matches!(
            self.state(),
            State::Error | State::Cancelled | State::Dismissed | State::Completed | State::Passive
        )
    }

    fn hide_error(&self) -> bool {
        let priv_ = imp::IdentityVerification::from_instance(self);
        priv_.hide_error.get()
    }

    fn show_error(&self) {
        if self.hide_error() {
            return;
        }

        let error_message = if let Some(info) = self.cancel_info() {
            match info.cancel_code() {
                CancelCode::User => Some(gettext("You cancelled the verificaiton process.")),
                CancelCode::Timeout => Some(gettext(
                    "The verification process failed because it reached a timeout.",
                )),
                CancelCode::Accepted => {
                    Some(gettext("You accepted the request from an other session."))
                }
                _ => match info.cancel_code().as_str() {
                    "m.mismatched_sas" => Some(gettext("The emoji did not match.")),
                    _ => None,
                },
            }
        } else {
            None
        };

        let error_message = error_message.unwrap_or_else(|| {
            gettext("An unknown error occured during the verification process.")
        });

        let error = Error::new(move |_| {
            let error_label = gtk::LabelBuilder::new()
                .label(&error_message)
                .wrap(true)
                .build();
            Some(error_label.upcast())
        });

        if let Some(window) = self.session().parent_window() {
            window.append_error(&error);
        }
    }

    pub fn display_name(&self) -> String {
        if self.user() != self.session().user().unwrap() {
            self.user().display_name()
        } else {
            // TODO: give this request a name based on the device
            "Login Request".to_string()
        }
    }

    pub fn flow_id(&self) -> &str {
        let priv_ = imp::IdentityVerification::from_instance(self);
        priv_
            .flow_id
            .get()
            .expect("Flow Id isn't always set on verifications with error state.")
    }

    fn set_flow_id(&self, flow_id: String) {
        let priv_ = imp::IdentityVerification::from_instance(self);

        priv_.flow_id.set(flow_id).unwrap();
    }

    /// Get the QrCode for this verification request
    ///
    /// This is only set once the request reached the `State::Ready`
    /// and if QrCode verification is possible
    pub fn qr_code(&self) -> Option<&QrCode> {
        let priv_ = imp::IdentityVerification::from_instance(self);

        priv_.qr_code.get()
    }

    /// Get the Emojis for this verification request
    ///
    /// This is only set once the request reached the `State::Ready`
    /// and if a Sas verification was started
    pub fn sas_data(&self) -> Option<&SasData> {
        let priv_ = imp::IdentityVerification::from_instance(self);

        priv_.sas_data.get()
    }

    pub fn start_sas(&self) {
        let priv_ = imp::IdentityVerification::from_instance(self);

        if let Some(sync_sender) = &*priv_.sync_sender.borrow() {
            let result = sync_sender.try_send(Message::UserAction(UserAction::StartSas));

            if let Err(error) = result {
                error!("Failed to send message to tokio runtime: {}", error);
            }
        }
    }

    pub fn scanned_qr_code(&self, data: QrVerificationData) {
        let priv_ = imp::IdentityVerification::from_instance(self);

        if let Some(sync_sender) = &*priv_.sync_sender.borrow() {
            let result = sync_sender.try_send(Message::UserAction(UserAction::Scanned(data)));

            if let Err(error) = result {
                error!("Failed to send message to tokio runtime: {}", error);
            }
        }
    }

    /// Accept an incomming request
    pub fn accept(&self) {
        let priv_ = imp::IdentityVerification::from_instance(self);
        if let Some(sync_sender) = &*priv_.sync_sender.borrow() {
            let result = sync_sender.try_send(Message::UserAction(UserAction::Accept));
            if let Err(error) = result {
                error!("Failed to send message to tokio runtime: {}", error);
            }
        }
    }

    pub fn cancel(&self, hide_error: bool) {
        let priv_ = imp::IdentityVerification::from_instance(self);

        priv_.hide_error.set(hide_error);

        if let Some(sync_sender) = &*priv_.sync_sender.borrow() {
            let result = sync_sender.try_send(Message::UserAction(UserAction::Cancel));
            if let Err(error) = result {
                error!("Failed to send message to tokio runtime: {}", error);
            }
        }
    }

    pub fn dismiss(&self) {
        self.set_state(State::Dismissed);
    }

    /// Get information about why the request was cancelled
    pub fn cancel_info(&self) -> Option<&CancelInfo> {
        let priv_ = imp::IdentityVerification::from_instance(self);

        priv_.cancel_info.get()
    }

    pub fn notify_state(&self) {
        let priv_ = imp::IdentityVerification::from_instance(self);

        if let Some(sync_sender) = &*priv_.sync_sender.borrow() {
            let result = sync_sender.try_send(Message::NotifyState);
            if let Err(error) = result {
                error!("Failed to send message to tokio runtime: {}", error);
            }
        }
    }
}

struct Context {
    client: Client,
    main_sender: glib::SyncSender<MainMessage>,
    sync_receiver: mpsc::Receiver<Message>,
    request: VerificationRequest,
}

macro_rules! wait {
    ( $this:ident $(, $expected:expr )? $(; expect_match $allow_action:ident )? ) => {
        {
            loop {
                // FIXME: add method to the sdk to check if a SAS verification was started
                if let Some(Verification::SasV1(sas)) = $this.client.get_verification($this.request.other_user_id(), $this.request.flow_id()).await {
                    return Ok($this.continue_sas(sas).await?);
                }

                if $this.request.is_passive() {
                    return Ok(State::Passive);
                }

                $(
                    if $expected {
                        break;
                    }
                )?

                match $this.sync_receiver.recv().await.expect("The channel was closed unexpected") {
                    Message::NotifyState if $this.request.is_cancelled() => {
                        if let Some(info) = $this.request.cancel_info() {
                            $this.send_cancel_info(info);
                        }
                        return Ok(State::Cancelled);
                    },
                    Message::UserAction(UserAction::Cancel) | Message::UserAction(UserAction::NotMatch) => {
                        return Ok($this.cancel_request().await?);
                    },
                    Message::UserAction(UserAction::Accept) => {
                        if true $(&& $allow_action)? {
                            break;
                        }
                    },
                    Message::UserAction(UserAction::StartSas) => {
                        if true $(&& $allow_action)? {
                            return Ok($this.start_sas().await?);
                        }
                    },
                    Message::UserAction(UserAction::Match) => {
                        if $this.request.is_passive() {
                            return Ok(State::Passive);
                        }

                        // Break only if we are in the expected state
                        if true $(&& $expected)? {
                            break;
                        }
                    },
                    Message::UserAction(UserAction::Scanned(data)) => {
                        if true $(&& $allow_action)? {
                            return Ok($this.finish_scanning(data).await?);
                        }
                    },
                    Message::NotifyState => {
                    }
                }
            }
        }
    };
}

// WORKAROUND: since rust thinks that we are creating a recursive async function
macro_rules! wait_without_scanning_sas {
    ( $this:ident $(, $expected:expr )?) => {
        {
            loop {
                if $this.request.is_passive() {
                    return Ok(State::Passive);
                }

                $(
                    if $expected {
                        break;
                    }
                )?

                match $this.sync_receiver.recv().await.expect("The channel was closed unexpected") {
                    Message::NotifyState if $this.request.is_cancelled() => {
                        if let Some(info) = $this.request.cancel_info() {
                            $this.send_cancel_info(info);
                        }
                        return Ok(State::Cancelled);
                    },
                    Message::UserAction(UserAction::Cancel) => {
                        return Ok($this.cancel_request().await?);
                    }
                    Message::UserAction(UserAction::NotMatch) => {
                        return Ok($this.cancel_request().await?);
                    },
                    Message::UserAction(UserAction::Accept) => {
                        break;
                    },
                    Message::UserAction(UserAction::StartSas) => {
                    },
                    Message::UserAction(UserAction::Match) => {
                        if $this.request.is_passive() {
                            return Ok(State::Passive);
                        }

                        // Break only if we are in the expected state
                        if true $(&& $expected)? {
                            break;
                        }
                    },
                    Message::UserAction(UserAction::Scanned(_)) => {
                    },
                    Message::NotifyState => {
                    }
                }
            }
        }
    };
}

impl Context {
    pub async fn new(
        client: Client,
        user_id: &UserId,
        flow_id: &str,
        main_sender: glib::SyncSender<MainMessage>,
        sync_receiver: mpsc::Receiver<Message>,
    ) -> Option<Self> {
        let request = client.get_verification_request(user_id, flow_id).await?;

        Some(Self {
            client,
            request,
            main_sender,
            sync_receiver,
        })
    }

    fn send_state(&self, state: State) {
        self.main_sender.send(MainMessage::State(state)).unwrap();
    }

    fn send_qr_code(&self, qr_code: QrCode) {
        self.main_sender.send(MainMessage::QrCode(qr_code)).unwrap();
    }

    fn send_sas_data(&self, data: SasData) {
        self.main_sender.send(MainMessage::SasData(data)).unwrap();
    }

    fn send_cancel_info(&self, cancel_info: CancelInfo) {
        self.main_sender
            .send(MainMessage::CancelInfo(cancel_info))
            .unwrap();
    }

    fn send_supported_methods(&self, flags: SupportedMethods) {
        self.main_sender
            .send(MainMessage::SupportedMethods(flags))
            .unwrap();
    }

    async fn start(mut self) -> Result<State, RequestVerificationError> {
        if self.request.we_started() {
            wait![self, self.request.is_ready()];
        } else {
            // Check if it was started by somebody else already
            if self.request.is_passive() {
                return Ok(State::Passive);
            }

            // Wait for the user to accept or cancel the request
            wait![self];

            self.request
                .accept_with_methods(vec![
                    VerificationMethod::SasV1,
                    VerificationMethod::QrCodeScanV1,
                    VerificationMethod::QrCodeShowV1,
                    VerificationMethod::ReciprocateV1,
                ])
                .await?;
        }

        let supported_methods: SupportedMethods = self
            .request
            .their_supported_methods()
            .unwrap()
            .into_iter()
            .map(Into::into)
            .collect();

        self.send_supported_methods(supported_methods);

        let request = if supported_methods.contains(SupportedMethods::QR_SHOW) {
            let request = self
                .request
                .generate_qr_code()
                .await
                .map_err(|error| RequestVerificationError::Sdk(error))?
                .expect("Couldn't create qr-code");

            if let Ok(qr_code) = request.to_qr_code() {
                self.send_qr_code(qr_code);
            } else {
                return Ok(State::Error);
            }

            self.send_state(State::QrV1Show);

            request
        } else if supported_methods.contains(SupportedMethods::QR_SCAN) {
            self.send_state(State::QrV1Scan);

            // Wait for scanned data
            wait![self];

            unreachable!();
        } else {
            return Ok(self.start_sas().await?);
        };

        // FIXME: we should automatically confirm
        request.confirm().await?;

        wait![self, request.is_done()];

        Ok(State::Completed)
    }

    async fn finish_scanning(
        mut self,
        data: QrVerificationData,
    ) -> Result<State, RequestVerificationError> {
        let request = self
            .request
            .scan_qr_code(data)
            .await?
            .expect("Scanning Qr Code should be supported");

        // FIXME: we should automatically confirm
        request.confirm().await?;

        wait_without_scanning_sas![self, request.is_done()];

        Ok(State::Completed)
    }

    async fn start_sas(self) -> Result<State, RequestVerificationError> {
        let request = self
            .request
            .start_sas()
            .await
            .map_err(|error| RequestVerificationError::Sdk(error))?
            .expect("Sas should be supported");

        self.continue_sas(request).await
    }

    async fn continue_sas(
        mut self,
        request: SasVerification,
    ) -> Result<State, RequestVerificationError> {
        request.accept().await?;

        wait_without_scanning_sas![self, request.can_be_presented()];

        let sas_data = if let Some(emoji) = request.emoji() {
            SasData::Emoji(emoji)
        } else if let Some(decimal) = request.decimals() {
            SasData::Decimal(decimal)
        } else {
            return Ok(State::Error);
        };

        self.send_sas_data(sas_data);
        self.send_state(State::SasV1);

        // Wait for match user action
        wait_without_scanning_sas![self];

        request.confirm().await?;

        wait_without_scanning_sas![self, request.is_done()];

        Ok(State::Completed)
    }

    async fn cancel_request(self) -> Result<State, RequestVerificationError> {
        self.request.cancel().await?;

        if let Some(info) = self.request.cancel_info() {
            self.send_cancel_info(info);
        }

        Ok(State::Cancelled)
    }
}
