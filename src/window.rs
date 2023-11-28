use std::cell::Cell;

use adw::subclass::prelude::AdwApplicationWindowImpl;
use gettextrs::gettext;
use gtk::{self, gdk, gio, glib, glib::clone, prelude::*, subclass::prelude::*, CompositeTemplate};
use ruma::RoomId;
use tracing::{error, info, warn};

use crate::{
    account_switcher::AccountSwitcherPopover,
    components::Spinner,
    error_page::ErrorPage,
    greeter::Greeter,
    login::Login,
    prelude::*,
    secret::{self, StoredSession},
    session::{
        model::{Session, SessionState},
        view::{AccountSettings, SessionView},
    },
    session_list::{FailedSession, NewSession, SessionInfo, SessionList},
    spawn, spawn_tokio, toast, Application, APP_ID, PROFILE,
};

mod imp {
    use glib::subclass::InitializingObject;
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, CompositeTemplate, Default)]
    #[template(resource = "/org/gnome/Fractal/ui/window.ui")]
    pub struct Window {
        /// Whether the window should be in compact view.
        pub compact: Cell<bool>,
        #[template_child]
        pub main_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub loading: TemplateChild<gtk::WindowHandle>,
        #[template_child]
        pub greeter: TemplateChild<Greeter>,
        #[template_child]
        pub login: TemplateChild<Login>,
        #[template_child]
        pub error_page: TemplateChild<ErrorPage>,
        #[template_child]
        pub session: TemplateChild<SessionView>,
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub offline_banner: TemplateChild<adw::Banner>,
        #[template_child]
        pub spinner: TemplateChild<Spinner>,
        /// The list of logged-in sessions.
        pub session_list: SessionList,
        /// The selection of the logged-in sessions.
        ///
        /// The one that is selected being the one that is visible.
        pub session_selection: gtk::SingleSelection,
        pub account_switcher: AccountSwitcherPopover,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Window {
        const NAME: &'static str = "Window";
        type Type = super::Window;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);

            klass.add_binding_action(
                gdk::Key::v,
                gdk::ModifierType::CONTROL_MASK,
                "win.paste",
                None,
            );
            klass.add_binding_action(
                gdk::Key::Insert,
                gdk::ModifierType::SHIFT_MASK,
                "win.paste",
                None,
            );
            klass.install_action("win.paste", None, move |obj, _, _| {
                obj.imp().session.handle_paste_action();
            });

            klass.install_action(
                "win.open-account-settings",
                Some("s"),
                move |obj, _, variant| {
                    if let Some(session_id) = variant.and_then(|v| v.get::<String>()) {
                        obj.open_account_settings(&session_id);
                    }
                },
            );

            klass.install_action("win.new-session", None, |obj, _, _| {
                obj.switch_to_greeter_page();
            });
            klass.install_action("win.show-login", None, |obj, _, _| {
                obj.switch_to_login_page();
            });
            klass.install_action("win.show-session", None, |obj, _, _| {
                obj.show_selected_session();
            });

            klass.install_action("win.toggle-fullscreen", None, |obj, _, _| {
                if obj.is_fullscreened() {
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

    impl ObjectImpl for Window {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpecBoolean::builder("compact")
                        .explicit_notify()
                        .build(),
                    glib::ParamSpecObject::builder::<SessionList>("session-list")
                        .read_only()
                        .build(),
                    glib::ParamSpecObject::builder::<gtk::SingleSelection>("session-selection")
                        .read_only()
                        .build(),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            let obj = self.obj();

            match pspec.name() {
                "compact" => obj.compact().to_value(),
                "session-list" => obj.session_list().to_value(),
                "session-selection" => obj.session_selection().to_value(),
                _ => unimplemented!(),
            }
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            let obj = self.obj();

            match pspec.name() {
                "compact" => obj.set_compact(value.get().unwrap()),
                _ => unimplemented!(),
            }
        }

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

            obj.load_window_size();

            self.session_list
                .connect_is_empty_notify(clone!(@weak obj => move |session_list| {
                    obj.action_set_enabled("win.show-session", !session_list.is_empty());
                }));
            obj.action_set_enabled("win.show-session", !self.session_list.is_empty());

            self.main_stack.connect_visible_child_notify(
                clone!(@weak obj => move |_| obj.set_default_by_child()),
            );

            obj.set_default_by_child();

            self.session_selection.set_model(Some(&self.session_list));
            self.session_selection.set_autoselect(true);

            self.session_selection
                .connect_selected_item_notify(clone!(@weak obj => move |_| {
                    obj.show_selected_session();
                }));

            spawn!(clone!(@weak obj => async move {
                obj.restore_sessions().await;
            }));

            self.account_switcher
                .set_session_selection(Some(self.session_selection.clone()));

            let monitor = gio::NetworkMonitor::default();
            monitor.connect_network_changed(clone!(@weak obj => move |_, _| {
                obj.update_network_state();
            }));

            obj.update_network_state();
        }
    }

    impl WindowImpl for Window {
        // save window state on delete event
        fn close_request(&self) -> glib::Propagation {
            if let Err(err) = self.obj().save_window_size() {
                warn!("Failed to save window state, {}", &err);
            }
            if let Err(err) = self.obj().save_current_visible_session() {
                warn!("Failed to save current session: {err}");
            }
            glib::Propagation::Proceed
        }
    }

    impl WidgetImpl for Window {}
    impl ApplicationWindowImpl for Window {}
    impl AdwApplicationWindowImpl for Window {}
}

glib::wrapper! {
    pub struct Window(ObjectSubclass<imp::Window>)
        @extends gtk::Widget, gtk::Window, gtk::Root, gtk::ApplicationWindow, adw::ApplicationWindow, @implements gtk::Accessible, gio::ActionMap, gio::ActionGroup;
}

impl Window {
    pub fn new(app: &Application) -> Self {
        glib::Object::builder()
            .property("application", Some(app))
            .property("icon-name", Some(APP_ID))
            .build()
    }

    /// Whether the window should be in compact view.
    ///
    /// It means the horizontal size is not large enough to hold all the
    /// content.
    pub fn compact(&self) -> bool {
        self.imp().compact.get()
    }

    /// Set whether the window should be in compact view.
    pub fn set_compact(&self, compact: bool) {
        if compact == self.compact() {
            return;
        }

        self.imp().compact.set(compact);
        self.notify("compact");
    }

    /// The list of logged-in sessions with a selection.
    ///
    /// The one that is selected being the one that is visible.
    pub fn session_list(&self) -> &SessionList {
        &self.imp().session_list
    }

    /// The selection of the logged-in sessions.
    ///
    /// The one that is selected being the one that is visible.
    pub fn session_selection(&self) -> &gtk::SingleSelection {
        &self.imp().session_selection
    }

    pub fn add_session(&self, session: Session) {
        let imp = &self.imp();
        let session_list = self.session_list();

        let index = session_list.insert(session.clone());
        let settings = Application::default().settings();
        if session.session_id() == settings.string("current-session")
            || !session_list.has_new_sessions()
        {
            imp.session_selection.set_selected(index as u32);
        }

        // Start listening to notifications when the session is ready.
        if session.state() == SessionState::Ready {
            spawn!(clone!(@weak session => async move {
                session.init_notifications().await
            }));
        } else {
            session.connect_ready(|session| {
                spawn!(clone!(@weak session => async move {
                    session.init_notifications().await
                }));
            });
        }

        session.connect_logged_out(clone!(@weak self as obj => move |session| {
            obj.remove_session(session)
        }));
    }

    fn remove_session(&self, session: &Session) {
        let imp = self.imp();

        imp.session_list.remove(session.session_id());

        if imp.session_list.is_empty() {
            self.switch_to_greeter_page();
        }
    }

    pub async fn restore_sessions(&self) {
        let imp = self.imp();
        let handle = spawn_tokio!(secret::restore_sessions());
        match handle.await.unwrap() {
            Ok(sessions) => {
                if sessions.is_empty() {
                    self.switch_to_greeter_page();
                } else {
                    for stored_session in sessions {
                        info!(
                            "Restoring previous session for user: {}",
                            stored_session.user_id
                        );
                        if let Some(path) = stored_session.path.to_str() {
                            info!("Database path: {path}");
                        }
                        imp.session_list
                            .insert(NewSession::new(stored_session.clone()));

                        spawn!(
                            glib::Priority::DEFAULT_IDLE,
                            clone!(@weak self as obj => async move {
                                obj.restore_stored_session(stored_session).await;
                            })
                        );
                    }
                }
            }
            Err(error) => {
                error!("Failed to restore previous sessions: {error}");

                let message = format!(
                    "{}\n\n{}",
                    gettext("Failed to restore previous sessions"),
                    error.to_user_facing(),
                );

                imp.error_page.display_secret_error(&message);
                imp.main_stack.set_visible_child(&*imp.error_page);
            }
        }
    }

    /// Restore a stored session.
    async fn restore_stored_session(&self, session_info: StoredSession) {
        match Session::restore(session_info.clone()).await {
            Ok(session) => {
                session.prepare().await;
                self.add_session(session);
            }
            Err(error) => {
                warn!("Failed to restore previous session: {error}");
                toast!(self, error.to_user_facing());

                self.session_list()
                    .insert(FailedSession::new(session_info, error));
            }
        }
    }

    /// The ID of the currently visible session, if any.
    pub fn current_session_id(&self) -> Option<String> {
        Some(
            self.imp()
                .session_selection
                .selected_item()
                .and_downcast::<SessionInfo>()?
                .session_id()
                .to_owned(),
        )
    }

    /// Set the current session by its ID.
    ///
    /// Returns `true` if the session was set as the current session.
    pub fn set_current_session_by_id(&self, session_id: &str) -> bool {
        let imp = self.imp();

        let Some(index) = imp.session_list.index(session_id) else {
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
                imp.main_stack.set_visible_child(&*imp.session);
            } else {
                session.connect_ready(clone!(@weak imp => move |_| {
                    imp.main_stack.set_visible_child(&*imp.session);
                }));
                self.switch_to_loading_page();
            }

            // We need to grab the focus so that keyboard shortcuts work.
            imp.session.grab_focus();

            return;
        }

        if let Some(failed) = session.downcast_ref::<FailedSession>() {
            imp.error_page
                .display_session_error(&failed.error().to_user_facing());
            imp.main_stack.set_visible_child(&*imp.error_page);
        } else {
            self.switch_to_loading_page();
        }

        imp.session.set_session(None);
    }

    pub fn save_window_size(&self) -> Result<(), glib::BoolError> {
        let settings = Application::default().settings();

        let size = self.default_size();

        settings.set_int("window-width", size.0)?;
        settings.set_int("window-height", size.1)?;

        settings.set_boolean("is-maximized", self.is_maximized())?;

        Ok(())
    }

    fn load_window_size(&self) {
        let settings = Application::default().settings();

        let width = settings.int("window-width");
        let height = settings.int("window-height");
        let is_maximized = settings.boolean("is-maximized");

        self.set_default_size(width, height);
        self.set_property("maximized", is_maximized);
    }

    /// Change the default widget of the window based on the visible child.
    ///
    /// These are the default widgets:
    /// - `Greeter` screen => `Login` button.
    fn set_default_by_child(&self) {
        let imp = self.imp();

        if imp.main_stack.visible_child() == Some(imp.greeter.get().upcast()) {
            self.set_default_widget(Some(&imp.greeter.default_widget()));
        } else {
            self.set_default_widget(gtk::Widget::NONE);
        }
    }

    pub fn switch_to_loading_page(&self) {
        let imp = self.imp();
        imp.main_stack.set_visible_child(&*imp.loading);
    }

    pub fn switch_to_login_page(&self) {
        let imp = self.imp();
        imp.main_stack.set_visible_child(&*imp.login);
        imp.login.focus_default();
    }

    pub fn switch_to_greeter_page(&self) {
        let imp = self.imp();
        imp.main_stack.set_visible_child(&*imp.greeter);
    }

    /// This appends a new toast to the list
    pub fn add_toast(&self, toast: adw::Toast) {
        self.imp().toast_overlay.add_toast(toast);
    }

    pub fn account_switcher(&self) -> &AccountSwitcherPopover {
        &self.imp().account_switcher
    }

    /// The `SessionView` of this window.
    pub fn session_view(&self) -> &SessionView {
        &self.imp().session
    }

    fn update_network_state(&self) {
        let imp = self.imp();
        let monitor = gio::NetworkMonitor::default();

        let is_network_available = monitor.is_network_available();
        self.action_set_enabled("win.show-login", is_network_available);

        if !is_network_available {
            imp.offline_banner
                .set_title(&gettext("No network connection"));
            imp.offline_banner.set_revealed(true);
        } else if monitor.connectivity() < gio::NetworkConnectivity::Full {
            imp.offline_banner
                .set_title(&gettext("No Internet connection"));
            imp.offline_banner.set_revealed(true);
        } else {
            imp.offline_banner.set_revealed(false);
        }
    }

    /// Show the given room for the given session.
    pub fn show_room(&self, session_id: &str, room_id: &RoomId) {
        if self.set_current_session_by_id(session_id) {
            self.imp().session.select_room_by_id(room_id);

            self.present();
        }
    }

    pub fn save_current_visible_session(&self) -> Result<(), glib::BoolError> {
        let settings = Application::default().settings();

        settings.set_string(
            "current-session",
            self.current_session_id().unwrap_or_default().as_str(),
        )?;

        Ok(())
    }

    /// Open the account settings for the session with the given ID.
    pub fn open_account_settings(&self, session_id: &str) {
        let Some(session) = self
            .session_list()
            .get(session_id)
            .and_downcast::<Session>()
        else {
            error!("Tried to open account settings of unknown session with ID '{session_id}'");
            return;
        };

        let window = AccountSettings::new(Some(self), &session);
        window.present();
    }
}
