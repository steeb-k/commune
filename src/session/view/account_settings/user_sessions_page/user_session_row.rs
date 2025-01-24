use adw::prelude::*;
use gettextrs::gettext;
use gtk::{glib, glib::clone, subclass::prelude::*, CompositeTemplate};
use tracing::error;
use url::Url;

use super::AccountSettings;
use crate::{
    components::{AuthError, LoadingButton},
    gettext_f,
    session::{model::UserSession, view::account_settings::AccountSettingsSubpage},
    system_settings::ClockFormat,
    toast,
    utils::{oauth, BoundConstructOnlyObject},
    Application,
};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/account_settings/user_sessions_page/user_session_row.ui"
    )]
    #[properties(wrapper_type = super::UserSessionRow)]
    pub struct UserSessionRow {
        #[template_child]
        display_name: TemplateChild<gtk::Label>,
        #[template_child]
        verified_icon: TemplateChild<gtk::Image>,
        #[template_child]
        last_seen_ip: TemplateChild<gtk::Label>,
        #[template_child]
        last_seen_ts: TemplateChild<gtk::Label>,
        #[template_child]
        loading_disconnect_button: TemplateChild<LoadingButton>,
        #[template_child]
        open_url_disconnect_button: TemplateChild<gtk::Button>,
        #[template_child]
        verify_button: TemplateChild<LoadingButton>,
        /// The user session displayed by this row.
        #[property(get, set = Self::set_user_session, construct_only)]
        user_session: RefCell<Option<UserSession>>,
        /// The ancestor [`AccountSettings`].
        #[property(get, set = Self::set_account_settings, construct_only)]
        account_settings: BoundConstructOnlyObject<AccountSettings>,
        system_settings_handler: RefCell<Option<glib::SignalHandlerId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for UserSessionRow {
        const NAME: &'static str = "AccountSettingsUserSessionRow";
        type Type = super::UserSessionRow;
        type ParentType = gtk::ListBoxRow;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for UserSessionRow {
        fn constructed(&self) {
            self.parent_constructed();

            self.update_disconnect_button();

            let system_settings = Application::default().system_settings();
            let system_settings_handler = system_settings.connect_clock_format_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.update_last_seen_ts();
                }
            ));
            self.system_settings_handler
                .replace(Some(system_settings_handler));
        }

        fn dispose(&self) {
            if let Some(handler) = self.system_settings_handler.take() {
                Application::default().system_settings().disconnect(handler);
            }
        }
    }

    impl WidgetImpl for UserSessionRow {}
    impl ListBoxRowImpl for UserSessionRow {}

    #[gtk::template_callbacks]
    impl UserSessionRow {
        /// Set the user session displayed by this row.
        fn set_user_session(&self, user_session: UserSession) {
            let obj = self.obj();

            let session_name = user_session.display_name();
            self.display_name.set_label(&session_name);
            obj.set_tooltip_text(Some(user_session.device_id().as_str()));

            self.verified_icon.set_visible(user_session.verified());
            // TODO: Implement verification
            // imp.verify_button.set_visible(!device.is_verified());

            let last_seen_ip = user_session.last_seen_ip();
            if let Some(last_seen_ip) = &last_seen_ip {
                self.last_seen_ip.set_label(last_seen_ip);
            }
            self.last_seen_ip.set_visible(last_seen_ip.is_some());

            self.last_seen_ts
                .set_visible(user_session.last_seen_ts().is_some());

            let disconnect_label = if user_session.is_current() {
                gettext("Log Out")
            } else {
                gettext("Disconnect Session")
            };
            self.loading_disconnect_button
                .set_content_label(disconnect_label);

            self.user_session.replace(Some(user_session));

            obj.notify_user_session();
            self.update_last_seen_ts();
        }

        /// Update the last seen timestamp according to the current user session
        /// and clock format setting.
        fn update_last_seen_ts(&self) {
            let Some(datetime) = self
                .user_session
                .borrow()
                .as_ref()
                .and_then(UserSession::last_seen_ts)
            else {
                return;
            };

            let clock_format = Application::default().system_settings().clock_format();
            let use_24 = clock_format == ClockFormat::TwentyFourHours;

            // This was ported from Nautilus and simplified for our use case.
            // See: https://gitlab.gnome.org/GNOME/nautilus/-/blob/1c5bd3614a35cfbb49de087bc10381cdef5a218f/src/nautilus-file.c#L5001
            let now = glib::DateTime::now_local().unwrap();
            let format;
            let days_ago = {
                let today_midnight = glib::DateTime::from_local(
                    now.year(),
                    now.month(),
                    now.day_of_month(),
                    0,
                    0,
                    0f64,
                )
                .expect("constructing GDateTime works");

                let date = glib::DateTime::from_local(
                    datetime.year(),
                    datetime.month(),
                    datetime.day_of_month(),
                    0,
                    0,
                    0f64,
                )
                .expect("constructing GDateTime works");

                today_midnight.difference(&date).as_days()
            };

            // Show only the time if date is on today
            if days_ago == 0 {
                if use_24 {
                    // Translators: Time in 24h format, i.e. "23:04".
                    // Do not change the time format as it will follow the system settings.
                    // See `man strftime` or the documentation of g_date_time_format for the available specifiers: <https://docs.gtk.org/glib/method.DateTime.format.html>
                    format = gettext("Last seen at %H:%M");
                } else {
                    // Translators: Time in 12h format, i.e. "11:04 PM".
                    // Do not change the time format as it will follow the system settings.
                    // See `man strftime` or the documentation of g_date_time_format for the available specifiers: <https://docs.gtk.org/glib/method.DateTime.format.html>
                    format = gettext("Last seen at %I:%M %p");
                }
            }
            // Show the word "Yesterday" and time if date is on yesterday
            else if days_ago == 1 {
                if use_24 {
                    // Translators: this a time in 24h format, i.e. "Last seen yesterday at 23:04".
                    // Do not change the time format as it will follow the system settings.
                    // See `man strftime` or the documentation of g_date_time_format for the available specifiers: <https://docs.gtk.org/glib/method.DateTime.format.html>
                    // xgettext:no-c-format
                    format = gettext("Last seen yesterday at %H:%M");
                } else {
                    // Translators: this is a time in 12h format, i.e. "Last seen Yesterday at 11:04
                    // PM".
                    // Do not change the time format as it will follow the system settings.
                    // See `man strftime` or the documentation of g_date_time_format for the available specifiers: <https://docs.gtk.org/glib/method.DateTime.format.html>
                    // xgettext:no-c-format
                    format = gettext("Last seen yesterday at %I:%M %p");
                }
            }
            // Show a week day and time if date is in the last week
            else if days_ago > 1 && days_ago < 7 {
                if use_24 {
                    // Translators: this is the name of the week day followed by a time in 24h
                    // format, i.e. "Last seen Monday at 23:04".
                    // Do not change the time format as it will follow the system settings.
                    //  See `man strftime` or the documentation of g_date_time_format for the available specifiers: <https://docs.gtk.org/glib/method.DateTime.format.html>
                    // xgettext:no-c-format
                    format = gettext("Last seen %A at %H:%M");
                } else {
                    // Translators: this is the week day name followed by a time in 12h format, i.e.
                    // "Last seen Monday at 11:04 PM".
                    // Do not change the time format as it will follow the system settings.
                    // See `man strftime` or the documentation of g_date_time_format for the available specifiers: <https://docs.gtk.org/glib/method.DateTime.format.html>
                    // xgettext:no-c-format
                    format = gettext("Last seen %A at %I:%M %p");
                }
            } else if datetime.year() == now.year() {
                if use_24 {
                    // Translators: this is the month and day and the time in 24h format, i.e. "Last
                    // seen February 3 at 23:04".
                    // Do not change the time format as it will follow the system settings.
                    // See `man strftime` or the documentation of g_date_time_format for the available specifiers: <https://docs.gtk.org/glib/method.DateTime.format.html>
                    // xgettext:no-c-format
                    format = gettext("Last seen %B %-e at %H:%M");
                } else {
                    // Translators: this is the month and day and the time in 12h format, i.e. "Last
                    // seen February 3 at 11:04 PM".
                    // Do not change the time format as it will follow the system settings.
                    // See `man strftime` or the documentation of g_date_time_format for the available specifiers: <https://docs.gtk.org/glib/method.DateTime.format.html>
                    // xgettext:no-c-format
                    format = gettext("Last seen %B %-e at %I:%M %p");
                }
            } else if use_24 {
                // Translators: this is the full date and the time in 24h format, i.e. "Last
                // seen February 3 2015 at 23:04".
                // Do not change the time format as it will follow the system settings.
                // See `man strftime` or the documentation of g_date_time_format for the available specifiers: <https://docs.gtk.org/glib/method.DateTime.format.html>
                // xgettext:no-c-format
                format = gettext("Last seen %B %-e %Y at %H:%M");
            } else {
                // Translators: this is the full date and the time in 12h format, i.e. "Last
                // seen February 3 2015 at 11:04 PM".
                // Do not change the time format as it will follow the system settings.
                // See `man strftime` or the documentation of g_date_time_format for the available specifiers: <https://docs.gtk.org/glib/method.DateTime.format.html>
                // xgettext:no-c-format
                format = gettext("Last seen %B %-e %Y at %I:%M %p");
            }

            let label = datetime
                .format(&format)
                .expect("formatting GDateTime works");
            self.last_seen_ts.set_label(&label);
        }

        /// Set the ancestor [`AccountSettings`].
        fn set_account_settings(&self, account_settings: AccountSettings) {
            let handler = account_settings.connect_account_management_url_changed(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.update_disconnect_button();
                }
            ));
            self.account_settings.set(account_settings, vec![handler]);
        }

        /// The account management URL of the authentication issuer, if any.
        fn account_management_url(&self) -> Option<Url> {
            self.account_settings.obj().account_management_url()
        }

        /// Update the visible disconnect button.
        fn update_disconnect_button(&self) {
            let Some(user_session) = self.user_session.borrow().clone() else {
                return;
            };
            let use_account_management_url =
                !user_session.is_current() && self.account_management_url().is_some();

            self.loading_disconnect_button
                .set_visible(!use_account_management_url);
            self.open_url_disconnect_button
                .set_visible(use_account_management_url);
        }

        /// Disconnect the user session by making a request to the homeserver.
        #[template_callback]
        async fn disconnect_with_request(&self) {
            let Some(user_session) = self.user_session.borrow().clone() else {
                return;
            };
            let obj = self.obj();

            if user_session.is_current() {
                let _ = obj.activate_action(
                    "account-settings.show-subpage",
                    Some(&AccountSettingsSubpage::LogOut.to_variant()),
                );
                return;
            }

            self.loading_disconnect_button.set_is_loading(true);

            match user_session.delete(&*obj).await {
                Ok(()) => obj.set_visible(false),
                Err(AuthError::UserCancelled) => {}
                Err(_) => {
                    let device_name = user_session.display_name();
                    // Translators: Do NOT translate the content between '{' and '}', this is a
                    // variable name.
                    let error_message = gettext_f(
                        "Could not disconnect device “{device_name}”",
                        &[("device_name", &device_name)],
                    );
                    toast!(obj, error_message);
                }
            }

            self.loading_disconnect_button.set_is_loading(false);
        }

        // Open the account management URL to disconnect the session.
        #[template_callback]
        async fn open_disconnect_url(&self) {
            let Some(device_id) = self
                .user_session
                .borrow()
                .as_ref()
                .map(|s| s.device_id().into())
            else {
                return;
            };
            let Some(mut url) = self.account_management_url() else {
                error!("Could not find open account management URL");
                return;
            };

            oauth::AccountManagementAction::SessionEnd { device_id }
                .add_to_account_management_url(&mut url);

            if let Err(error) = gtk::UriLauncher::new(url.as_ref())
                .launch_future(self.obj().root().and_downcast_ref::<gtk::Window>())
                .await
            {
                error!("Could not launch account management URL: {error}");
            }
        }
    }
}

glib::wrapper! {
    /// A row presenting a user session.
    pub struct UserSessionRow(ObjectSubclass<imp::UserSessionRow>)
        @extends gtk::Widget, gtk::ListBoxRow, @implements gtk::Accessible;
}

impl UserSessionRow {
    pub fn new(user_session: &UserSession, account_settings: &AccountSettings) -> Self {
        glib::Object::builder()
            .property("user-session", user_session)
            .property("account-settings", account_settings)
            .build()
    }
}
