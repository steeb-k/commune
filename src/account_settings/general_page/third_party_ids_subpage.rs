use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{gio, glib, glib::clone};
use ruma::{
    ClientSecret, OwnedClientSecret, OwnedSessionId,
    api::error::ErrorKind,
    thirdparty::{Medium, ThirdPartyIdentifier},
};
use tracing::{error, warn};

use crate::{
    components::{AuthDialog, AuthError, EntryAddRow, RemovableRow},
    i18n::gettext_f,
    session::Session,
    spawn, spawn_tokio, toast,
    utils::{PlaceholderObject, SingleItemListModel},
};

/// The state of an email address that was requested to be added.
#[derive(Debug, Clone)]
struct PendingEmail {
    /// The address.
    address: String,
    /// The client secret identifying the validation session.
    client_secret: OwnedClientSecret,
    /// The ID of the validation session.
    sid: OwnedSessionId,
    /// The number of times a validation email was requested.
    send_attempt: u32,
}

mod imp {
    use std::cell::{Cell, RefCell};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/account_settings/general_page/third_party_ids_subpage.ui"
    )]
    #[properties(wrapper_type = super::ThirdPartyIdsSubpage)]
    pub struct ThirdPartyIdsSubpage {
        #[template_child]
        stack: TemplateChild<gtk::Stack>,
        #[template_child]
        emails_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        emails_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        email_add_row: TemplateChild<EntryAddRow>,
        #[template_child]
        phones_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        phones_list: TemplateChild<gtk::ListBox>,
        /// The current session.
        #[property(get, set = Self::set_session, construct_only)]
        session: glib::WeakRef<Session>,
        /// The email addresses on the account.
        emails: gtk::StringList,
        /// The phone numbers on the account.
        phones: gtk::StringList,
        /// Whether the homeserver allows changing the identifiers.
        can_change: Cell<bool>,
        /// The email address being added, if any.
        pending_email: RefCell<Option<PendingEmail>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ThirdPartyIdsSubpage {
        const NAME: &'static str = "ThirdPartyIdsSubpage";
        type Type = super::ThirdPartyIdsSubpage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for ThirdPartyIdsSubpage {
        fn constructed(&self) {
            self.parent_constructed();

            self.init_lists();

            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    imp.load().await;
                }
            ));
        }
    }

    impl WidgetImpl for ThirdPartyIdsSubpage {}
    impl NavigationPageImpl for ThirdPartyIdsSubpage {}

    #[gtk::template_callbacks]
    impl ThirdPartyIdsSubpage {
        /// Set the current session.
        fn set_session(&self, session: &Session) {
            self.session.set(Some(session));
        }

        /// Initialize the lists.
        fn init_lists(&self) {
            let extra_items = SingleItemListModel::new(Some(&PlaceholderObject::new("add")));

            let all_items = gio::ListStore::new::<glib::Object>();
            all_items.append(&self.emails);
            all_items.append(&extra_items);

            let flattened_list = gtk::FlattenListModel::new(Some(all_items));
            self.emails_list.bind_model(
                Some(&flattened_list),
                clone!(
                    #[weak(rename_to = imp)]
                    self,
                    #[upgrade_or_else]
                    || { adw::ActionRow::new().upcast() },
                    move |item| imp.create_row(item, Medium::Email)
                ),
            );

            self.phones_list.bind_model(
                Some(&self.phones),
                clone!(
                    #[weak(rename_to = imp)]
                    self,
                    #[upgrade_or_else]
                    || { adw::ActionRow::new().upcast() },
                    move |item| imp.create_row(item, Medium::Msisdn)
                ),
            );
        }

        /// Load the identifiers on the account.
        #[template_callback]
        pub(super) async fn load(&self) {
            let Some(session) = self.session.upgrade() else {
                return;
            };

            self.stack.set_visible_child_name("loading");

            let client = session.client();
            let handle = spawn_tokio!(async move {
                let can_change = client
                    .homeserver_capabilities()
                    .can_change_thirdparty_ids()
                    .await
                    .unwrap_or(true);
                client
                    .account()
                    .get_3pids()
                    .await
                    .map(|response| (response.threepids, can_change))
            });

            match handle.await.expect("task was not aborted") {
                Ok((threepids, can_change)) => {
                    self.can_change.set(can_change);
                    self.update_lists(&threepids);
                    self.stack.set_visible_child_name("content");
                }
                Err(error) => {
                    error!("Could not load the third-party identifiers: {error}");
                    self.stack.set_visible_child_name("error");
                }
            }
        }

        /// Update the lists with the given identifiers.
        fn update_lists(&self, threepids: &[ThirdPartyIdentifier]) {
            let emails = threepids
                .iter()
                .filter(|t| t.medium == Medium::Email)
                .map(|t| t.address.as_str())
                .collect::<Vec<_>>();
            let phones = threepids
                .iter()
                .filter(|t| t.medium == Medium::Msisdn)
                .map(|t| t.address.as_str())
                .collect::<Vec<_>>();

            self.emails.splice(0, self.emails.n_items(), &emails);
            self.phones.splice(0, self.phones.n_items(), &phones);

            // An account can only get a phone number from somewhere else, so
            // an empty list would only say something confusing.
            self.phones_group.set_visible(!phones.is_empty());

            self.update_add_email();
        }

        /// Create a row in a list for the given item.
        fn create_row(&self, item: &glib::Object, medium: Medium) -> gtk::Widget {
            let Some(string_obj) = item.downcast_ref::<gtk::StringObject>() else {
                // It can only be the dummy item to add a new email address.
                return self.email_add_row.clone().upcast();
            };

            let address = string_obj.string();

            if !self.can_change.get() {
                // The homeserver refuses changes, so the address can only be
                // looked at.
                let row = adw::ActionRow::new();
                row.set_title(&address);
                return row.upcast();
            }

            let row = RemovableRow::new();
            row.set_title(&address);
            row.set_remove_button_tooltip_text(Some(gettext_f(
                "Remove “{address}”",
                &[("address", &address)],
            )));

            row.connect_remove(clone!(
                #[weak(rename_to = imp)]
                self,
                move |row| {
                    let row = row.clone();
                    let medium = medium.clone();
                    spawn!(async move {
                        imp.remove_address(&row, medium).await;
                    });
                }
            ));

            row.upcast()
        }

        /// Remove the identifier of the given row.
        async fn remove_address(&self, row: &RemovableRow, medium: Medium) {
            let Some(session) = self.session.upgrade() else {
                return;
            };

            let address = row.title().to_string();

            // The homeserver can use an identifier to let the user get back
            // into the account, so removing one must not be a slip of the
            // pointer.
            let confirm_dialog = adw::AlertDialog::builder()
                .default_response("cancel")
                .heading(gettext("Remove Address?"))
                .body(gettext_f(
                    // Translators: Do NOT translate the content between '{' and
                    // '}', this is a variable name.
                    "{address} will no longer be linked to this account. If it could be used to reset your password, it cannot be after this.",
                    &[("address", &address)],
                ))
                .build();
            confirm_dialog.add_responses(&[
                ("cancel", &gettext("Cancel")),
                ("remove", &gettext("Remove")),
            ]);
            confirm_dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);

            if confirm_dialog.choose_future(Some(&*self.obj())).await != "remove" {
                return;
            }

            row.set_is_loading(true);

            let client = session.client();
            let address_clone = address.clone();
            let handle = spawn_tokio!(async move {
                client
                    .account()
                    .delete_3pid(&address_clone, medium, None)
                    .await
            });

            match handle.await.expect("task was not aborted") {
                Ok(_) => {
                    toast!(self.obj(), gettext("Address removed"));
                    self.load().await;
                }
                Err(error) => {
                    warn!("Could not remove the third-party identifier: {error}");
                    toast!(self.obj(), gettext("Could not remove address"));
                }
            }

            row.set_is_loading(false);
        }

        /// Whether the address in the entry can be added.
        fn can_add_email(&self) -> bool {
            if !self.can_change.get() || self.email_add_row.is_loading() {
                return false;
            }

            let text = self.email_add_row.text().trim().to_lowercase();

            // A light check, the homeserver validates for real: an address has
            // a local part, an @ that is not the first or last character, and
            // a domain.
            if !text
                .split_once('@')
                .is_some_and(|(local, domain)| !local.is_empty() && !domain.is_empty())
            {
                return false;
            }

            // Cannot add an address that is already on the account.
            for email_obj in self.emails.iter::<glib::Object>() {
                let Ok(email_obj) = email_obj else {
                    break;
                };

                if email_obj
                    .downcast_ref::<gtk::StringObject>()
                    .map(gtk::StringObject::string)
                    .is_some_and(|email| email.to_lowercase() == text)
                {
                    return false;
                }
            }

            true
        }

        /// Update the state of the row to add an email address.
        #[template_callback]
        fn update_add_email(&self) {
            self.email_add_row.set_visible(self.can_change.get());
            self.email_add_row.set_inhibit_add(!self.can_add_email());
        }

        /// Add the email address that is currently in the entry.
        #[template_callback]
        async fn add_email(&self) {
            if !self.can_add_email() {
                return;
            }

            let Some(session) = self.session.upgrade() else {
                return;
            };

            let address = self.email_add_row.text().trim().to_lowercase();

            // A second try for the same address keeps the validation session
            // and bumps the attempt, so the homeserver can resend the email
            // rather than opening a new session.
            let (client_secret, send_attempt) = match self.pending_email.borrow().as_ref() {
                Some(pending) if pending.address == address => {
                    (pending.client_secret.clone(), pending.send_attempt + 1)
                }
                _ => (ClientSecret::new(), 1),
            };

            self.email_add_row.set_is_loading(true);

            let client = session.client();
            let client_secret_clone = client_secret.clone();
            let address_clone = address.clone();
            let handle = spawn_tokio!(async move {
                client
                    .account()
                    .request_3pid_email_token(
                        &client_secret_clone,
                        &address_clone,
                        send_attempt.into(),
                    )
                    .await
            });

            match handle.await.expect("task was not aborted") {
                Ok(response) => {
                    self.pending_email.replace(Some(PendingEmail {
                        address,
                        client_secret,
                        sid: response.sid,
                        send_attempt,
                    }));

                    self.email_add_row.set_is_loading(false);
                    self.confirm_pending_email().await;
                }
                Err(error) => {
                    warn!("Could not request an email validation token: {error}");
                    let text = match error.client_api_error_kind() {
                        Some(ErrorKind::ThreepidInUse) => {
                            gettext("This email address is already linked to an account")
                        }
                        Some(ErrorKind::ThreepidDenied) => {
                            gettext("The homeserver does not accept this email address")
                        }
                        _ => gettext("Could not send the validation email"),
                    };
                    toast!(self.obj(), text);
                    self.email_add_row.set_is_loading(false);
                }
            }
        }

        /// Finish adding the pending email address, once its link has been
        /// opened.
        async fn confirm_pending_email(&self) {
            let Some(session) = self.session.upgrade() else {
                return;
            };
            let Some(pending) = self.pending_email.borrow().clone() else {
                return;
            };

            let obj = self.obj();

            let dialog = adw::AlertDialog::builder()
                .default_response("continue")
                .heading(gettext("Open the Link in the Email"))
                .body(gettext_f(
                    // Translators: Do NOT translate the content between '{' and
                    // '}', this is a variable name.
                    "A validation link was sent to {address}. Open it, then continue here.",
                    &[("address", &pending.address)],
                ))
                .build();
            dialog.add_responses(&[
                ("cancel", &gettext("Cancel")),
                ("continue", &gettext("Continue")),
            ]);
            dialog.set_response_appearance("continue", adw::ResponseAppearance::Suggested);

            loop {
                if dialog.clone().choose_future(Some(&*obj)).await != "continue" {
                    // The validation session simply expires on the server.
                    self.pending_email.take();
                    return;
                }

                let auth_dialog = AuthDialog::new(&session);
                let pending_clone = pending.clone();
                let result = auth_dialog
                    .authenticate(&*obj, move |client, auth| {
                        let pending = pending_clone.clone();
                        async move {
                            client
                                .account()
                                .add_3pid(&pending.client_secret, &pending.sid, auth)
                                .await
                        }
                    })
                    .await;

                match result {
                    Ok(_) => {
                        self.pending_email.take();
                        self.email_add_row.set_text("");
                        toast!(obj, gettext("Email address added"));
                        self.load().await;
                        return;
                    }
                    Err(AuthError::UserCancelled) => {
                        // Offer the first dialog again, the link may simply
                        // not have been opened yet.
                    }
                    Err(AuthError::ServerResponse(error))
                        if matches!(
                            error.client_api_error_kind(),
                            Some(ErrorKind::ThreepidAuthFailed)
                        ) =>
                    {
                        toast!(
                            obj,
                            gettext("Open the link in the email first, then try again")
                        );
                    }
                    Err(error) => {
                        warn!("Could not add the email address: {error}");
                        toast!(obj, gettext("Could not add email address"));
                        return;
                    }
                }
            }
        }
    }
}

glib::wrapper! {
    /// Subpage to manage the third-party identifiers of the account.
    ///
    /// Email addresses can be seen, added and removed. Phone numbers can be
    /// seen and removed, but not added: validating one takes a text message,
    /// and the infrastructure to send one is not something this client can
    /// assume.
    pub struct ThirdPartyIdsSubpage(ObjectSubclass<imp::ThirdPartyIdsSubpage>)
        @extends gtk::Widget, adw::NavigationPage,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ThirdPartyIdsSubpage {
    pub fn new(session: &Session) -> Self {
        glib::Object::builder().property("session", session).build()
    }
}
