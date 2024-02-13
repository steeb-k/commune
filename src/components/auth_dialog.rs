use std::{fmt::Debug, future::Future};

use adw::{prelude::*, subclass::prelude::*};
use futures_channel::oneshot;
use gettextrs::gettext;
use gtk::{glib, glib::clone, CompositeTemplate};
use matrix_sdk::Error;
use ruma::{
    api::client::{
        error::StandardErrorBody,
        uiaa::{AuthData, AuthType, Dummy, FallbackAcknowledgement, Password, UserIdentifier},
    },
    assign,
};
use thiserror::Error;
use tracing::error;

use crate::{prelude::*, session::model::Session, spawn, spawn_tokio};

/// An error during UIAA interaction.
#[derive(Debug, Error)]
pub enum AuthError {
    /// The server returned a non-UIAA error.
    #[error(transparent)]
    ServerResponse(#[from] Error),

    /// The ID of the UIAA session is missing for a stage that requires it.
    #[error("The ID of the session is missing")]
    MissingSessionId,

    /// The available flows are empty or done but the endpoint still requires
    /// UIAA.
    #[error("There is no stage to choose from")]
    NoStageToChoose,

    /// The user cancelled the authentication.
    #[error("The user cancelled the authentication")]
    UserCancelled,

    /// The parent `Session` could not be upgraded.
    #[error("The session could not be upgraded")]
    NoSession,

    /// The parent `gtk::Widget` could not be upgraded.
    #[error("The parent widget could not be upgraded")]
    NoParentWidget,
}

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/auth_dialog.ui")]
    #[properties(wrapper_type = super::AuthDialog)]
    pub struct AuthDialog {
        #[template_child]
        pub password: TemplateChild<gtk::PasswordEntry>,
        #[template_child]
        pub open_browser_btn: TemplateChild<gtk::Button>,
        pub open_browser_btn_handler: RefCell<Option<glib::SignalHandlerId>>,
        #[template_child]
        pub error: TemplateChild<gtk::Label>,
        #[property(get, set, construct_only)]
        /// The parent session.
        pub session: glib::WeakRef<Session>,
        #[property(get)]
        /// The parent widget.
        pub parent: glib::WeakRef<gtk::Widget>,
        pub sender: RefCell<Option<oneshot::Sender<String>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AuthDialog {
        const NAME: &'static str = "ComponentsAuthDialog";
        type Type = super::AuthDialog;
        type ParentType = adw::AlertDialog;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for AuthDialog {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            self.password
                .connect_changed(clone!(@weak obj => move |password| {
                    obj.set_response_enabled("confirm", !password.text().is_empty());
                }));
        }
    }

    impl WidgetImpl for AuthDialog {}
    impl AdwDialogImpl for AuthDialog {}

    impl AdwAlertDialogImpl for AuthDialog {
        fn response(&self, response: &str) {
            if let Some(sender) = self.sender.take() {
                if sender.send(response.to_owned()).is_err() {
                    error!("Failed to send response");
                }
            }
        }
    }
}

glib::wrapper! {
    /// Dialog to guide the user through the [User-Interactive Authentication API] (UIAA).
    ///
    /// [User-Interaction Authentication API]: https://spec.matrix.org/v1.7/client-server-api/#user-interactive-authentication-api
    pub struct AuthDialog(ObjectSubclass<imp::AuthDialog>)
        @extends gtk::Widget, adw::Dialog, adw::AlertDialog, @implements gtk::Accessible;
}

impl AuthDialog {
    pub fn new(session: &Session) -> Self {
        glib::Object::builder().property("session", session).build()
    }

    /// Authenticates the user to the server via an authentication flow.
    ///
    /// The type of flow and the required stages are negotiated at time of
    /// authentication. Returns the last server response on success.
    pub async fn authenticate<
        Response: Send + 'static,
        F1: Future<Output = Result<Response, Error>> + Send + 'static,
        FN: Fn(matrix_sdk::Client, Option<AuthData>) -> F1 + Send + 'static + Sync + Clone,
    >(
        &self,
        parent: &impl IsA<gtk::Widget>,
        callback: FN,
    ) -> Result<Response, AuthError> {
        let Some(client) = self.session().map(|s| s.client()) else {
            return Err(AuthError::NoSession);
        };

        self.imp().parent.set(Some(parent.upcast_ref()));

        let mut auth_data = None;

        loop {
            let callback_clone = callback.clone();
            let client_clone = client.clone();
            let handle = spawn_tokio!(async move { callback_clone(client_clone, auth_data).await });
            let response = handle.await.unwrap();

            let uiaa_info = match response {
                Ok(result) => return Ok(result),
                Err(error) => {
                    if let Some(uiaa_info) = error.as_uiaa_response() {
                        uiaa_info.clone()
                    } else {
                        return Err(error.into());
                    }
                }
            };

            self.show_auth_error(&uiaa_info.auth_error);

            let stage_nr = uiaa_info.completed.len();
            let possible_stages: Vec<&AuthType> = uiaa_info
                .flows
                .iter()
                .filter(|flow| flow.stages.starts_with(&uiaa_info.completed))
                .flat_map(|flow| flow.stages.get(stage_nr))
                .collect();

            let uiaa_session = uiaa_info.session;
            auth_data = Some(
                self.perform_next_stage(&uiaa_session, &possible_stages)
                    .await?,
            );
        }
    }

    /// Performs the most preferred one of the given stages.
    ///
    /// Stages that Fractal actually implements are preferred.
    async fn perform_next_stage(
        &self,
        uiaa_session: &Option<String>,
        stages: &[&AuthType],
    ) -> Result<AuthData, AuthError> {
        // Default to first stage if non is supported.
        let a_stage = stages.first().ok_or(AuthError::NoStageToChoose)?;
        for stage in stages {
            if let Some(auth_result) = self.try_perform_stage(uiaa_session, stage).await {
                return auth_result;
            }
        }
        self.perform_fallback(uiaa_session.clone(), a_stage).await
    }

    /// Tries to perform the given stage.
    ///
    /// Returns None if the stage is not implemented by Fractal.
    async fn try_perform_stage(
        &self,
        uiaa_session: &Option<String>,
        stage: &AuthType,
    ) -> Option<Result<AuthData, AuthError>> {
        match stage {
            AuthType::Password => Some(self.perform_password_stage(uiaa_session.clone()).await),
            AuthType::Sso => Some(self.perform_fallback(uiaa_session.clone(), stage).await),
            AuthType::Dummy => Some(self.perform_dummy_stage(uiaa_session.clone())),
            // TODO implement other authentication types
            // See: https://gitlab.gnome.org/World/fractal/-/issues/835
            _ => None,
        }
    }

    /// Performs the password stage.
    async fn perform_password_stage(
        &self,
        uiaa_session: Option<String>,
    ) -> Result<AuthData, AuthError> {
        let Some(session) = self.session() else {
            return Err(AuthError::NoSession);
        };

        let imp = self.imp();
        imp.password.set_visible(true);
        imp.open_browser_btn.set_visible(false);
        self.set_body(&gettext(
            "Please authenticate the operation with your password",
        ));
        self.set_response_enabled("confirm", false);

        self.show_and_wait_for_response().await?;

        let user_id = session.user_id().to_string();
        let password = imp.password.text().into();

        let data = assign!(
            Password::new(UserIdentifier::UserIdOrLocalpart(user_id), password),
            { session: uiaa_session }
        );

        Ok(AuthData::Password(data))
    }

    /// Performs the dummy stage.
    fn perform_dummy_stage(&self, uiaa_session: Option<String>) -> Result<AuthData, AuthError> {
        Ok(AuthData::Dummy(
            assign!(Dummy::new(), { session: uiaa_session }),
        ))
    }

    /// Performs a web-based fallback for the given stage.
    async fn perform_fallback(
        &self,
        uiaa_session: Option<String>,
        stage: &AuthType,
    ) -> Result<AuthData, AuthError> {
        let Some(client) = self.session().map(|s| s.client()) else {
            return Err(AuthError::NoSession);
        };
        let uiaa_session = uiaa_session.ok_or(AuthError::MissingSessionId)?;

        let imp = self.imp();
        imp.password.set_visible(false);
        imp.open_browser_btn.set_visible(true);
        self.set_body(&gettext(
            "Please authenticate the operation via the browser and, once completed, press confirm",
        ));
        self.set_response_enabled("confirm", false);

        let homeserver = client.homeserver();
        self.setup_fallback_page(homeserver.as_str(), stage.as_ref(), &uiaa_session);

        self.show_and_wait_for_response().await?;

        Ok(AuthData::FallbackAcknowledgement(
            FallbackAcknowledgement::new(uiaa_session),
        ))
    }

    /// Lets the user complete the current stage.
    async fn show_and_wait_for_response(&self) -> Result<(), AuthError> {
        let Some(parent) = self.parent() else {
            return Err(AuthError::NoParentWidget);
        };

        let (sender, receiver) = futures_channel::oneshot::channel();
        self.imp().sender.replace(Some(sender));

        self.present(&parent);

        let result = receiver.await.unwrap();
        self.close();

        if result == "confirm" {
            Ok(())
        } else {
            Err(AuthError::UserCancelled)
        }
    }

    fn show_auth_error(&self, auth_error: &Option<StandardErrorBody>) {
        let imp = self.imp();

        if let Some(auth_error) = auth_error {
            imp.error.set_label(&auth_error.message);
        }

        imp.error.set_visible(auth_error.is_some());
    }

    fn setup_fallback_page(&self, homeserver: &str, auth_type: &str, uiaa_session: &str) {
        let imp = self.imp();

        if let Some(handler) = imp.open_browser_btn_handler.take() {
            imp.open_browser_btn.disconnect(handler);
        }

        let uri = format!(
            "{homeserver}_matrix/client/r0/auth/{auth_type}/fallback/web?session={uiaa_session}"
        );

        let handler = imp
            .open_browser_btn
            .connect_clicked(clone!(@weak self as obj => move |_| {
                let uri = uri.clone();
                spawn!(async move {
                    let Some(parent) = obj.parent() else {
                        return;
                    };

                    if let Err(error) = gtk::UriLauncher::new(&uri)
                        .launch_future(parent.root().and_downcast_ref::<gtk::Window>())
                        .await
                    {
                        error!("Could not launch URI: {error}");
                    }

                    obj.set_response_enabled("confirm", true);
                });
            }));

        imp.open_browser_btn_handler.replace(Some(handler));
    }
}
