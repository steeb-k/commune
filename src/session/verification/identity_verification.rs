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
        verification::{CancelInfo, Emoji, QrVerificationData, VerificationRequest},
    },
    ruma::{
        events::key::verification::{cancel::CancelCode, VerificationMethod},
        identifiers::UserId,
    },
    Client,
};
use qrcode::QrCode;
use tokio::sync::mpsc;

#[derive(Debug, Eq, PartialEq, Clone, Copy, glib::GEnum)]
#[repr(u32)]
#[genum(type_name = "VerificationMode")]
pub enum Mode {
    Requested,
    SasV1,
    QrV1Show,
    QrV1Scan,
    Completed,
    Cancelled,
    Dismissed,
    Passive,
    Error,
}

impl Default for Mode {
    fn default() -> Self {
        Self::Requested
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
    Mode(Mode),
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
        pub mode: Cell<Mode>,
        pub supported_methods: Cell<SupportedMethods>,
        pub sync_sender: RefCell<Option<mpsc::Sender<Message>>>,
        pub main_sender: RefCell<Option<glib::SyncSender<MainMessage>>>,
        pub sas_data: OnceCell<SasData>,
        pub qr_code: OnceCell<QrCode>,
        pub cancel_info: OnceCell<CancelInfo>,
        pub flow_id: OnceCell<String>,
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
                        "mode",
                        "Mode",
                        "The verification mode used",
                        Mode::static_type(),
                        Mode::default() as i32,
                        glib::ParamFlags::READABLE | glib::ParamFlags::EXPLICIT_NOTIFY,
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
                "flow-id" => obj.set_flow_id(value.get().unwrap()),
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "user" => obj.user().to_value(),
                "session" => obj.session().to_value(),
                "mode" => obj.mode().to_value(),
                "display-name" => obj.display_name().to_value(),
                "flow-id" => obj.flow_id().to_value(),
                "supported-methods" => obj.supported_methods().to_value(),
                _ => unimplemented!(),
            }
        }

        fn constructed(&self, obj: &Self::Type) {
            self.parent_constructed(obj);

            let (main_sender, main_receiver) =
                glib::MainContext::sync_channel::<MainMessage>(Default::default(), 100);

            main_receiver.attach(
                None,
                clone!(@weak obj => @default-panic, move |message| {
                    let priv_ = imp::IdentityVerification::from_instance(&obj);
                    match message {
                        MainMessage::QrCode(data) => { let _ = priv_.qr_code.set(data); },
                        MainMessage::CancelInfo(data) => priv_.cancel_info.set(data).unwrap(),
                        MainMessage::SasData(data) => priv_.sas_data.set(data).unwrap(),
                        MainMessage::SupportedMethods(flags) => priv_.supported_methods.set(flags),
                        MainMessage::Mode(mode) => obj.set_mode(mode),
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
        }

        fn dispose(&self, obj: &Self::Type) {
            obj.cancel();
        }
    }
}

glib::wrapper! {
    pub struct IdentityVerification(ObjectSubclass<imp::IdentityVerification>);
}

impl IdentityVerification {
    fn for_mode(mode: Mode, session: &Session, user: &User) -> Self {
        glib::Object::new(&[("mode", &mode), ("session", session), ("user", user)])
            .expect("Failed to create IdentityVerification")
    }

    /// Create a new object tracking an already existing verification request
    pub fn for_flow_id(flow_id: &str, session: &Session, user: &User) -> Self {
        glib::Object::new(&[("flow-id", &flow_id), ("session", session), ("user", user)])
            .expect("Failed to create IdentityVerification")
    }

    /// Creates and send a new verificaiton request
    pub async fn create(session: &Session, user: &User) -> Self {
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
                    let obj = Self::for_flow_id(request.flow_id(), session, user);
                    // This will start the request handling
                    obj.accept();
                    return obj;
                }
                Err(error) => {
                    error!("Starting a verification failed: {}", error);
                }
            }
        } else {
            error!("Starting a verification failed: Crypto identity wasn't found");
        }

        Self::for_mode(Mode::Error, session, user)
    }

    /// Accept an incomming request
    pub fn accept(&self) {
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

        // TODO add timeout

        let handle = spawn_tokio!(async move {
            if let Some(context) =
                Context::new(client, &user_id, &flow_id, main_sender, sync_receiver).await
            {
                context.start().await
            } else {
                Ok(Mode::Error)
            }
        });

        let weak_obj = self.downgrade();
        spawn!(async move {
            let result = handle.await.unwrap();
            if let Some(obj) = weak_obj.upgrade() {
                let priv_ = imp::IdentityVerification::from_instance(&obj);
                match result {
                    Ok(result) => obj.set_mode(result),
                    Err(error) => {
                        // FIXME: report error to the user
                        error!("Verification failed: {}", error);
                        obj.set_mode(Mode::Error);
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

    pub fn mode(&self) -> Mode {
        let priv_ = imp::IdentityVerification::from_instance(self);
        priv_.mode.get()
    }

    fn set_mode(&self, mode: Mode) {
        let priv_ = imp::IdentityVerification::from_instance(self);

        if self.mode() == mode {
            return;
        }

        match mode {
            Mode::Cancelled | Mode::Error => self.show_error(),
            _ => {}
        }

        priv_.mode.set(mode);
        self.notify("mode");
    }

    /// Whether this request is finished
    pub fn is_finished(&self) -> bool {
        matches!(
            self.mode(),
            Mode::Error | Mode::Cancelled | Mode::Dismissed | Mode::Completed | Mode::Passive
        )
    }

    fn show_error(&self) {
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

    pub fn cancel(&self) {
        let priv_ = imp::IdentityVerification::from_instance(self);
        if let Some(sync_sender) = &*priv_.sync_sender.borrow() {
            let result = sync_sender.try_send(Message::UserAction(UserAction::Cancel));
            if let Err(error) = result {
                error!("Failed to send message to tokio runtime: {}", error);
            }
        }
    }

    pub fn dismiss(&self) {
        self.set_mode(Mode::Dismissed);
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
    main_sender: glib::SyncSender<MainMessage>,
    sync_receiver: mpsc::Receiver<Message>,
    request: VerificationRequest,
}

macro_rules! wait {
    ( $this:ident $(, $expected:expr )? $(; expect_match $allow_action:ident )? ) => {
        {
            loop {
                match $this.sync_receiver.recv().await.expect("The channel was closed unexpected") {
                    Message::NotifyState if $this.request.is_cancelled() => {
                        if let Some(info) = $this.request.cancel_info() {
                            $this.send_cancel_info(info);
                        }
                        return Ok(Mode::Cancelled);
                    },
                    Message::UserAction(UserAction::Cancel) | Message::UserAction(UserAction::NotMatch) => {
                        return Ok($this.cancel_request().await?);
                    }
                    Message::UserAction(UserAction::StartSas) => {
                        if true $(&& $allow_action)? {
                            return Ok($this.start_sas().await?);
                        }
                    },
                    Message::UserAction(UserAction::Match) => {
                        if $this.request.is_passive() {
                            return Ok(Mode::Passive);
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

                if $this.request.is_passive() {
                    return Ok(Mode::Passive);
                }

                $(
                    if $expected {
                        break;
                    }
                )?
            }
        }
    };
}

// WORKAROUND: since rust thinks that we are creating a recursive async function
macro_rules! wait_without_scanning_sas {
    ( $this:ident $(, $expected:expr )?) => {
        {
            loop {
                match $this.sync_receiver.recv().await.expect("The channel was closed unexpected") {
                    Message::NotifyState if $this.request.is_cancelled() => {
                        if let Some(info) = $this.request.cancel_info() {
                            $this.send_cancel_info(info);
                        }
                        return Ok(Mode::Cancelled);
                    },
                    Message::UserAction(UserAction::Cancel) => {
                        return Ok($this.cancel_request().await?);
                    }
                    Message::UserAction(UserAction::NotMatch) => {
                        return Ok($this.cancel_request().await?);
                    },
                    Message::UserAction(UserAction::StartSas) => {
                    },
                    Message::UserAction(UserAction::Match) => {
                        if $this.request.is_passive() {
                            return Ok(Mode::Passive);
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

                if $this.request.is_passive() {
                    return Ok(Mode::Passive);
                }

                $(
                    if $expected {
                        break;
                    }
                )?
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
            request,
            main_sender,
            sync_receiver,
        })
    }

    fn send_mode(&self, mode: Mode) {
        self.main_sender.send(MainMessage::Mode(mode)).unwrap();
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

    async fn start(mut self) -> Result<Mode, RequestVerificationError> {
        if self.request.we_started() {
            wait![self, self.request.is_ready()];
        } else {
            // Check if it was started by somebody else already
            if self.request.is_passive() {
                return Ok(Mode::Passive);
            }

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
                return Ok(Mode::Error);
            }

            self.send_mode(Mode::QrV1Show);

            request
        } else if supported_methods.contains(SupportedMethods::QR_SCAN) {
            self.send_mode(Mode::QrV1Scan);

            // Wait for scanned data
            wait![self];

            unreachable!();
        } else {
            return Ok(self.start_sas().await?);
        };

        // FIXME: we should automatically confirm
        request.confirm().await?;

        wait![self, request.is_done()];

        Ok(Mode::Completed)
    }

    async fn finish_scanning(
        mut self,
        data: QrVerificationData,
    ) -> Result<Mode, RequestVerificationError> {
        let request = self
            .request
            .scan_qr_code(data)
            .await?
            .expect("Scanning Qr Code should be supported");

        // FIXME: we should automatically confirm
        request.confirm().await?;

        wait_without_scanning_sas![self, request.is_done()];

        Ok(Mode::Completed)
    }

    async fn start_sas(mut self) -> Result<Mode, RequestVerificationError> {
        let request = self
            .request
            .start_sas()
            .await
            .map_err(|error| RequestVerificationError::Sdk(error))?
            .expect("Sas should be supported");

        request.accept().await?;

        wait_without_scanning_sas![self, request.can_be_presented()];

        let sas_data = if let Some(emoji) = request.emoji() {
            SasData::Emoji(emoji)
        } else if let Some(decimal) = request.decimals() {
            SasData::Decimal(decimal)
        } else {
            return Ok(Mode::Error);
        };

        self.send_sas_data(sas_data);
        self.send_mode(Mode::SasV1);

        // Wait for match user action
        wait_without_scanning_sas![self];

        request.confirm().await?;

        wait_without_scanning_sas![self, request.is_done()];

        Ok(Mode::Completed)
    }

    async fn cancel_request(self) -> Result<Mode, RequestVerificationError> {
        self.request.cancel().await?;

        if let Some(info) = self.request.cancel_info() {
            self.send_cancel_info(info);
        }

        Ok(Mode::Cancelled)
    }
}
