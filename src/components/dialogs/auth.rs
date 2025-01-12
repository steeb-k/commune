use std::{fmt::Debug, future::Future};

use adw::{prelude::*, subclass::prelude::*};
use futures_channel::oneshot;
use gettextrs::gettext;
use gtk::{glib, glib::clone, CompositeTemplate};
use matrix_sdk::{encryption::CrossSigningResetAuthType, Error};
use ruma::{
    api::client::{
        error::StandardErrorBody,
        uiaa::{
            AuthData, AuthType, Dummy, FallbackAcknowledgement, Password, UiaaInfo, UserIdentifier,
        },
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

    /// An unexpected error occurred.
    #[error("An unexpected error occurred")]
    Unknown,
}

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/dialogs/auth.ui")]
    #[properties(wrapper_type = super::AuthDialog)]
    pub struct AuthDialog {
        #[template_child]
        password: TemplateChild<gtk::PasswordEntry>,
        #[template_child]
        open_browser_btn: TemplateChild<gtk::Button>,
        open_browser_btn_handler: RefCell<Option<glib::SignalHandlerId>>,
        #[template_child]
        error: TemplateChild<gtk::Label>,
        /// The parent session.
        #[property(get, set, construct_only)]
        session: glib::WeakRef<Session>,
        /// The parent widget.
        #[property(get)]
        parent: glib::WeakRef<gtk::Widget>,
        /// The sender for the response.
        sender: RefCell<Option<oneshot::Sender<String>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AuthDialog {
        const NAME: &'static str = "AuthDialog";
        type Type = super::AuthDialog;
        type ParentType = adw::AlertDialog;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for AuthDialog {}

    impl WidgetImpl for AuthDialog {}
    impl AdwDialogImpl for AuthDialog {}

    impl AdwAlertDialogImpl for AuthDialog {
        fn response(&self, response: &str) {
            if let Some(sender) = self.sender.take() {
                if sender.send(response.to_owned()).is_err() {
                    error!("Could not send response");
                }
            }
        }
    }

    #[gtk::template_callbacks]
    impl AuthDialog {
        /// Authenticate the user to the server via an interactive
        /// authentication flow.
        ///
        /// The type of flow and the required stages are negotiated during the
        /// authentication. Returns the last server response on success.
        pub(super) async fn authenticate<
            Response: Send + 'static,
            F1: Future<Output = Result<Response, Error>> + Send + 'static,
            FN: Fn(matrix_sdk::Client, Option<AuthData>) -> F1 + Send + 'static + Sync + Clone,
        >(
            &self,
            parent: &gtk::Widget,
            callback: FN,
        ) -> Result<Response, AuthError> {
            let Some(client) = self.session.upgrade().map(|s| s.client()) else {
                return Err(AuthError::NoSession);
            };

            self.parent.set(Some(parent));

            let mut auth_data = None;

            loop {
                let callback_clone = callback.clone();
                let client_clone = client.clone();

                // Get the current state of the authentication.
                let handle =
                    spawn_tokio!(async move { callback_clone(client_clone, auth_data).await });
                let response = handle.await.expect("task was not aborted");

                let error = match response {
                    // Authentication is over.
                    Ok(result) => return Ok(result),
                    Err(error) => error,
                };
                // If this is a UIAA error, authentication continues.
                let Some(uiaa_info) = error.as_uiaa_response() else {
                    return Err(error.into());
                };

                let next_auth_data = self.perform_next_stage(uiaa_info).await?;
                auth_data = Some(next_auth_data);
            }
        }

        /// Reset the cross-signing keys while handling the interactive
        /// authentication flow.
        ///
        /// The type of flow and the required stages are negotiated during the
        /// authentication. Returns the last server response on success.
        pub(super) async fn reset_cross_signing(
            &self,
            parent: &gtk::Widget,
        ) -> Result<(), AuthError> {
            let Some(encryption) = self.session.upgrade().map(|s| s.client().encryption()) else {
                return Err(AuthError::NoSession);
            };

            self.parent.set(Some(parent));

            let handle = spawn_tokio!(async move { encryption.reset_cross_signing().await })
                .await
                .expect("task was not aborted")?;

            if let Some(handle) = handle {
                match handle.auth_type() {
                    CrossSigningResetAuthType::Uiaa(uiaa_info) => {
                        let auth_data = self.perform_next_stage(uiaa_info).await?;

                        spawn_tokio!(async move { handle.auth(Some(auth_data)).await })
                            .await
                            .expect("task was not aborted")?;
                    }
                    CrossSigningResetAuthType::Oidc(_) => {
                        // According to the code, this is only used with the `experimental-oidc`
                        // feature. Return an error in case this changes.
                        error!(
                            "Could not perform cross-signing reset: received unexpected OIDC stage"
                        );
                        return Err(AuthError::Unknown);
                    }
                }
            }

            Ok(())
        }

        /// Performs the preferred next stage in the given UIAA info.
        ///
        /// Stages that are actually supported are preferred. If no stages are
        /// supported, we use the web-based fallback.
        async fn perform_next_stage(&self, uiaa_info: &UiaaInfo) -> Result<AuthData, AuthError> {
            // Show the authentication error, if there is one.
            self.show_auth_error(uiaa_info.auth_error.as_ref());

            // Find and perform the next stage.
            let stages = uiaa_info
                .flows
                .iter()
                .filter_map(|flow| flow.stages.strip_prefix(uiaa_info.completed.as_slice()))
                .filter_map(|stages_left| stages_left.first());

            let mut first_stage = None;
            for stage in stages {
                if let Some(auth_result) = self
                    .try_perform_stage(uiaa_info.session.as_ref(), stage)
                    .await
                {
                    return auth_result;
                }

                if first_stage.is_none() {
                    first_stage = Some(stage);
                }
            }

            // Default to first stage if no stages are supported.
            let first_stage = first_stage.ok_or(AuthError::NoStageToChoose)?;
            self.perform_fallback(uiaa_info.session.clone(), first_stage)
                .await
        }

        /// Tries to perform the given stage.
        ///
        /// Returns `None` if the stage is not implemented.
        async fn try_perform_stage(
            &self,
            uiaa_session: Option<&String>,
            stage: &AuthType,
        ) -> Option<Result<AuthData, AuthError>> {
            match stage {
                AuthType::Password => {
                    Some(self.perform_password_stage(uiaa_session.cloned()).await)
                }
                AuthType::Sso => Some(self.perform_fallback(uiaa_session.cloned(), stage).await),
                AuthType::Dummy => Some(Ok(Self::perform_dummy_stage(uiaa_session.cloned()))),
                _ => None,
            }
        }

        /// Performs the password stage.
        async fn perform_password_stage(
            &self,
            uiaa_session: Option<String>,
        ) -> Result<AuthData, AuthError> {
            let Some(session) = self.session.upgrade() else {
                return Err(AuthError::NoSession);
            };
            let obj = self.obj();

            self.password.set_visible(true);
            self.open_browser_btn.set_visible(false);
            obj.set_body(&gettext(
                "Please authenticate the operation with your password",
            ));
            obj.set_response_enabled("confirm", false);

            self.show_and_wait_for_response().await?;

            let user_id = session.user_id().to_string();
            let password = self.password.text().into();

            let data = assign!(
                Password::new(UserIdentifier::UserIdOrLocalpart(user_id), password),
                { session: uiaa_session }
            );

            Ok(AuthData::Password(data))
        }

        /// Performs the dummy stage.
        fn perform_dummy_stage(uiaa_session: Option<String>) -> AuthData {
            AuthData::Dummy(assign!(Dummy::new(), { session: uiaa_session }))
        }

        /// Performs a web-based fallback for the given stage.
        async fn perform_fallback(
            &self,
            uiaa_session: Option<String>,
            stage: &AuthType,
        ) -> Result<AuthData, AuthError> {
            let Some(client) = self.session.upgrade().map(|s| s.client()) else {
                return Err(AuthError::NoSession);
            };
            let uiaa_session = uiaa_session.ok_or(AuthError::MissingSessionId)?;
            let obj = self.obj();

            self.password.set_visible(false);
            self.open_browser_btn.set_visible(true);
            obj.set_body(&gettext(
                "Please authenticate the operation via the browser and, once completed, press confirm",
            ));
            obj.set_response_enabled("confirm", false);

            let homeserver = client.homeserver();
            self.set_up_fallback(homeserver.as_str(), stage.as_ref(), &uiaa_session);

            self.show_and_wait_for_response().await?;

            Ok(AuthData::FallbackAcknowledgement(
                FallbackAcknowledgement::new(uiaa_session),
            ))
        }

        /// Let the user complete the current stage.
        async fn show_and_wait_for_response(&self) -> Result<(), AuthError> {
            let Some(parent) = self.parent.upgrade() else {
                return Err(AuthError::NoParentWidget);
            };
            let obj = self.obj();

            let (sender, receiver) = futures_channel::oneshot::channel();
            self.sender.replace(Some(sender));

            // Show this dialog.
            obj.present(Some(&parent));

            // Wait for the response.
            let result = receiver.await;

            // Close this dialog.
            obj.close();

            match result.as_deref() {
                Ok("confirm") => Ok(()),
                Ok(_) => Err(AuthError::UserCancelled),
                Err(_) => {
                    error!("Could not get the response, the channel was closed");
                    Err(AuthError::Unknown)
                }
            }
        }

        /// Show the given error.
        fn show_auth_error(&self, auth_error: Option<&StandardErrorBody>) {
            if let Some(auth_error) = auth_error {
                self.error.set_label(&auth_error.message);
            }

            self.error.set_visible(auth_error.is_some());
        }

        /// Prepare the button to open the web-based fallback with the given
        /// settings.
        fn set_up_fallback(&self, homeserver: &str, auth_type: &str, uiaa_session: &str) {
            if let Some(handler) = self.open_browser_btn_handler.take() {
                self.open_browser_btn.disconnect(handler);
            }

            let uri = format!(
                "{homeserver}_matrix/client/r0/auth/{auth_type}/fallback/web?session={uiaa_session}"
            );

            let handler = self.open_browser_btn.connect_clicked(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    let uri = uri.clone();
                    spawn!(async move {
                        let Some(parent) = imp.parent.upgrade() else {
                            return;
                        };

                        if let Err(error) = gtk::UriLauncher::new(&uri)
                            .launch_future(parent.root().and_downcast_ref::<gtk::Window>())
                            .await
                        {
                            error!("Could not launch URI: {error}");
                        }

                        imp.obj().set_response_enabled("confirm", true);
                    });
                }
            ));

            self.open_browser_btn_handler.replace(Some(handler));
        }

        /// Update the confirm response for the current state.
        #[template_callback]
        fn update_confirm(&self) {
            self.obj()
                .set_response_enabled("confirm", !self.password.text().is_empty());
        }
    }
}

glib::wrapper! {
    /// Dialog to guide the user through the [User-Interactive Authentication API] (UIAA).
    ///
    /// [User-Interactive Authentication API]: https://spec.matrix.org/latest/client-server-api/#user-interactive-authentication-api
    pub struct AuthDialog(ObjectSubclass<imp::AuthDialog>)
        @extends gtk::Widget, adw::Dialog, adw::AlertDialog, @implements gtk::Accessible;
}

impl AuthDialog {
    pub fn new(session: &Session) -> Self {
        glib::Object::builder().property("session", session).build()
    }

    /// Authenticate the user to the server via an interactive authentication
    /// flow.
    ///
    /// The type of flow and the required stages are negotiated during the
    /// authentication. Returns the last server response on success.
    pub(crate) async fn authenticate<
        Response: Send + 'static,
        F1: Future<Output = Result<Response, Error>> + Send + 'static,
        FN: Fn(matrix_sdk::Client, Option<AuthData>) -> F1 + Send + 'static + Sync + Clone,
    >(
        &self,
        parent: &impl IsA<gtk::Widget>,
        callback: FN,
    ) -> Result<Response, AuthError> {
        self.imp().authenticate(parent.upcast_ref(), callback).await
    }

    /// Reset the cross-signing keys while handling the interactive
    /// authentication flow.
    ///
    /// The type of flow and the required stages are negotiated during the
    /// authentication. Returns the last server response on success.
    pub(crate) async fn reset_cross_signing(
        &self,
        parent: &impl IsA<gtk::Widget>,
    ) -> Result<(), AuthError> {
        self.imp().reset_cross_signing(parent.upcast_ref()).await
    }
}
