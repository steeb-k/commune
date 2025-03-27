use std::cell::Cell;

use adw::{prelude::*, subclass::prelude::*};
use gtk::{self, gdk, gio, glib, glib::clone, CompositeTemplate};
use ruma::RoomId;
use tracing::{error, warn};

use crate::{
    account_chooser_dialog::AccountChooserDialog,
    account_switcher::{AccountSwitcherButton, AccountSwitcherPopover},
    components::OfflineBanner,
    error_page::ErrorPage,
    intent::SessionIntent,
    login::Login,
    prelude::*,
    secret::SESSION_ID_LENGTH,
    session::{
        model::{IdentityVerification, Session, SessionState},
        view::{AccountSettings, SessionView},
    },
    session_list::{FailedSession, SessionInfo},
    toast,
    utils::LoadingState,
    Application, APP_ID, PROFILE, SETTINGS_KEY_CURRENT_SESSION,
};

/// A page of the main window stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumString, strum::AsRefStr)]
#[strum(serialize_all = "kebab-case")]
pub enum WindowPage {
    /// The loading page.
    Loading,
    /// The login view.
    Login,
    /// The session view.
    Session,
    /// The error page.
    Error,
}

mod imp {
    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, CompositeTemplate, Default, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/window.ui")]
    #[properties(wrapper_type = super::Window)]
    pub struct Window {
        #[template_child]
        pub main_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub loading: TemplateChild<gtk::WindowHandle>,
        #[template_child]
        pub login: TemplateChild<Login>,
        #[template_child]
        pub error_page: TemplateChild<ErrorPage>,
        #[template_child]
        pub session: TemplateChild<SessionView>,
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        /// Whether the window should be in compact view.
        ///
        /// It means that the horizontal size is not large enough to hold all
        /// the content.
        #[property(get, set = Self::set_compact, explicit_notify)]
        pub compact: Cell<bool>,
        /// The selection of the logged-in sessions.
        ///
        /// The one that is selected being the one that is visible.
        #[property(get)]
        pub session_selection: gtk::SingleSelection,
        pub account_switcher: AccountSwitcherPopover,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Window {
        const NAME: &'static str = "Window";
        type Type = super::Window;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            AccountSwitcherButton::ensure_type();
            OfflineBanner::ensure_type();

            Self::bind_template(klass);

            klass.add_binding_action(gdk::Key::v, gdk::ModifierType::CONTROL_MASK, "win.paste");
            klass.add_binding_action(gdk::Key::Insert, gdk::ModifierType::SHIFT_MASK, "win.paste");
            klass.install_action("win.paste", None, |obj, _, _| {
                obj.imp().session.handle_paste_action();
            });

            klass.install_action(
                "win.open-account-settings",
                Some(&String::static_variant_type()),
                |obj, _, variant| {
                    if let Some(session_id) = variant.and_then(glib::Variant::get::<String>) {
                        obj.open_account_settings(&session_id);
                    }
                },
            );

            klass.install_action("win.new-session", None, |obj, _, _| {
                obj.set_visible_page(WindowPage::Login);
            });
            klass.install_action("win.show-session", None, |obj, _, _| {
                obj.show_selected_session();
            });

            klass.install_action("win.toggle-fullscreen", None, |obj, _, _| {
                if obj.is_fullscreen() {
                    obj.unfullscreen();
                } else {
                    obj.fullscreen();
                }
            });
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for Window {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            let builder = gtk::Builder::from_resource("/org/gnome/Fractal/ui/shortcuts.ui");
            let shortcuts = builder.object("shortcuts").unwrap();
            obj.set_help_overlay(Some(&shortcuts));

            // Development Profile
            if PROFILE.should_use_devel_class() {
                obj.add_css_class("devel");
            }

            self.load_window_size();

            self.main_stack.connect_transition_running_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |stack| if !stack.is_transition_running() {
                    // Focus the default widget when the transition has ended.
                    imp.grab_focus();
                }
            ));

            self.account_switcher
                .set_session_selection(Some(self.session_selection.clone()));

            self.session_selection.connect_selected_item_notify(clone!(
                #[weak]
                obj,
                move |_| {
                    obj.show_selected_session();
                }
            ));
            self.session_selection.connect_items_changed(clone!(
                #[weak]
                obj,
                move |session_selection, pos, removed, added| {
                    let n_items = session_selection.n_items();
                    obj.action_set_enabled("win.show-session", n_items > 0);

                    if removed > 0 && n_items == 0 {
                        // There are no more sessions.
                        obj.set_visible_page(WindowPage::Login);
                        return;
                    }

                    if added == 0 {
                        return;
                    }

                    let settings = Application::default().settings();
                    let mut current_session_setting =
                        settings.string(SETTINGS_KEY_CURRENT_SESSION).to_string();

                    // Session IDs have been truncated in version 6 of StoredSession.
                    if current_session_setting.len() > SESSION_ID_LENGTH {
                        current_session_setting.truncate(SESSION_ID_LENGTH);

                        if let Err(error) = settings
                            .set_string(SETTINGS_KEY_CURRENT_SESSION, &current_session_setting)
                        {
                            warn!("Could not save current session: {error}");
                        }
                    }

                    for i in pos..pos + added {
                        let Some(session) = session_selection.item(i).and_downcast::<SessionInfo>()
                        else {
                            continue;
                        };

                        if let Some(failed) = session.downcast_ref::<FailedSession>() {
                            toast!(obj, failed.error().to_user_facing());
                        }

                        if session.session_id() == current_session_setting {
                            session_selection.set_selected(i);
                        }
                    }
                }
            ));

            let app = Application::default();
            let session_list = app.session_list();

            self.session_selection.set_model(Some(session_list));

            if session_list.state() == LoadingState::Ready {
                if session_list.is_empty() {
                    obj.set_visible_page(WindowPage::Login);
                }
            } else {
                session_list.connect_state_notify(clone!(
                    #[weak]
                    obj,
                    move |session_list| {
                        if session_list.state() == LoadingState::Ready && session_list.is_empty() {
                            obj.set_visible_page(WindowPage::Login);
                        }
                    }
                ));
            }
        }
    }

    impl WindowImpl for Window {
        // save window state on delete event
        fn close_request(&self) -> glib::Propagation {
            if let Err(error) = self.save_window_size() {
                warn!("Could not save window state: {error}");
            }
            if let Err(error) = self.save_current_visible_session() {
                warn!("Could not save current session: {error}");
            }

            glib::Propagation::Proceed
        }
    }

    impl WidgetImpl for Window {
        fn grab_focus(&self) -> bool {
            match self.visible_page() {
                WindowPage::Loading => false,
                WindowPage::Login => self.login.grab_focus(),
                WindowPage::Session => self.session.grab_focus(),
                WindowPage::Error => self.error_page.grab_focus(),
            }
        }
    }

    impl ApplicationWindowImpl for Window {}
    impl AdwApplicationWindowImpl for Window {}

    impl Window {
        /// Set whether the window should be in compact view.
        fn set_compact(&self, compact: bool) {
            if compact == self.compact.get() {
                return;
            }

            self.compact.set(compact);
            self.obj().notify_compact();
        }

        /// Load the window size from the settings.
        fn load_window_size(&self) {
            let obj = self.obj();
            let settings = Application::default().settings();

            let width = settings.int("window-width");
            let height = settings.int("window-height");
            let is_maximized = settings.boolean("is-maximized");

            obj.set_default_size(width, height);
            obj.set_maximized(is_maximized);
        }

        /// Save the current window size to the settings.
        fn save_window_size(&self) -> Result<(), glib::BoolError> {
            let obj = self.obj();
            let settings = Application::default().settings();

            let size = obj.default_size();
            settings.set_int("window-width", size.0)?;
            settings.set_int("window-height", size.1)?;

            settings.set_boolean("is-maximized", obj.is_maximized())?;

            Ok(())
        }

        /// Save the currently visible session to the settings.
        fn save_current_visible_session(&self) -> Result<(), glib::BoolError> {
            let settings = Application::default().settings();

            settings.set_string(
                SETTINGS_KEY_CURRENT_SESSION,
                self.current_session_id().unwrap_or_default().as_str(),
            )?;

            Ok(())
        }

        /// The visible page of the window.
        pub(super) fn visible_page(&self) -> WindowPage {
            self.main_stack
                .visible_child_name()
                .and_then(|s| s.as_str().try_into().ok())
                .unwrap()
        }

        /// The ID of the currently visible session, if any.
        pub(super) fn current_session_id(&self) -> Option<String> {
            self.session_selection
                .selected_item()
                .and_downcast::<SessionInfo>()
                .map(|s| s.session_id())
        }
    }
}

glib::wrapper! {
    /// The main window.
    pub struct Window(ObjectSubclass<imp::Window>)
        @extends gtk::Widget, gtk::Window, gtk::Root, gtk::ApplicationWindow, adw::ApplicationWindow,
        @implements gtk::Accessible, gio::ActionMap, gio::ActionGroup;
}

impl Window {
    pub fn new(app: &Application) -> Self {
        glib::Object::builder()
            .property("application", Some(app))
            .property("icon-name", Some(APP_ID))
            .build()
    }

    /// Add the given session to the session list and select it.
    pub fn add_session(&self, session: Session) {
        let index = Application::default().session_list().insert(session);
        self.session_selection().set_selected(index as u32);
    }

    /// The ID of the currently visible session, if any.
    pub fn current_session_id(&self) -> Option<String> {
        self.imp().current_session_id()
    }

    /// Set the current session by its ID.
    ///
    /// Returns `true` if the session was set as the current session.
    pub fn set_current_session_by_id(&self, session_id: &str) -> bool {
        let imp = self.imp();

        let Some(index) = Application::default().session_list().index(session_id) else {
            return false;
        };

        let index = index as u32;
        let prev_selected = imp.session_selection.selected();

        if index == prev_selected {
            // Make sure the session is displayed;
            self.show_selected_session();
        } else {
            imp.session_selection.set_selected(index);
        }

        true
    }

    /// Show the selected session.
    ///
    /// The displayed view will change according to the current session.
    pub fn show_selected_session(&self) {
        let imp = self.imp();

        let Some(session) = imp
            .session_selection
            .selected_item()
            .and_downcast::<SessionInfo>()
        else {
            return;
        };

        if let Some(session) = session.downcast_ref::<Session>() {
            imp.session.set_session(Some(session));

            if session.state() == SessionState::Ready {
                self.set_visible_page(WindowPage::Session);
            } else {
                session.connect_ready(clone!(
                    #[weak(rename_to = obj)]
                    self,
                    move |_| {
                        obj.set_visible_page(WindowPage::Session);
                    }
                ));
                self.set_visible_page(WindowPage::Loading);
            }

            // We need to grab the focus so that keyboard shortcuts work.
            imp.session.grab_focus();

            return;
        }

        if let Some(failed) = session.downcast_ref::<FailedSession>() {
            imp.error_page
                .display_session_error(&failed.error().to_user_facing());
            self.set_visible_page(WindowPage::Error);
        } else {
            self.set_visible_page(WindowPage::Loading);
        }

        imp.session.set_session(None::<Session>);
    }

    /// Set the visible page of the window.
    pub fn set_visible_page(&self, name: WindowPage) {
        self.imp().main_stack.set_visible_child_name(name.as_ref());
    }

    /// This appends a new toast to the list
    pub fn add_toast(&self, toast: adw::Toast) {
        self.imp().toast_overlay.add_toast(toast);
    }

    /// The account switcher popover.
    pub fn account_switcher(&self) -> &AccountSwitcherPopover {
        &self.imp().account_switcher
    }

    /// The `SessionView` of this window.
    pub fn session_view(&self) -> &SessionView {
        &self.imp().session
    }

    /// Show the given room for the given session.
    pub fn show_room(&self, session_id: &str, room_id: &RoomId) {
        if self.set_current_session_by_id(session_id) {
            self.imp().session.select_room_by_id(room_id);

            self.present();
        }
    }

    /// Open the account settings for the session with the given ID.
    pub fn open_account_settings(&self, session_id: &str) {
        let Some(session) = Application::default()
            .session_list()
            .get(session_id)
            .and_downcast::<Session>()
        else {
            error!("Tried to open account settings of unknown session with ID '{session_id}'");
            return;
        };

        let dialog = AccountSettings::new(&session);
        dialog.present(Some(self));
    }

    /// Open the error page and display the given secret error message.
    pub fn show_secret_error(&self, message: &str) {
        self.imp().error_page.display_secret_error(message);
        self.set_visible_page(WindowPage::Error);
    }

    /// Show the given identity verification for the session with the given ID.
    pub fn show_identity_verification(&self, session_id: &str, verification: IdentityVerification) {
        if self.set_current_session_by_id(session_id) {
            self.imp()
                .session
                .select_identity_verification(verification);

            self.present();
        }
    }

    /// Ask the user to choose a session.
    ///
    /// The session list must be ready.
    ///
    /// Returns the ID of the selected session, if any.
    pub async fn ask_session(&self) -> Option<String> {
        let dialog = AccountChooserDialog::new(Application::default().session_list());
        dialog.choose_account(self).await
    }

    /// Process the given session intent.
    ///
    /// The session must be ready.
    pub fn process_session_intent(&self, session_id: &str, intent: SessionIntent) {
        if !self.set_current_session_by_id(session_id) {
            error!("Cannot switch to unknown session with ID `{session_id}`");
            return;
        }

        self.imp().session.process_intent(intent);
    }
}
