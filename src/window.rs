use std::cell::Cell;

use adw::{prelude::*, subclass::prelude::*};
#[cfg(target_os = "windows")]
use gettextrs::gettext;
use gtk::{gdk, gio, glib, glib::clone};
use tracing::{error, warn};

#[cfg(target_os = "windows")]
use crate::utils::windows_frame;
use crate::{
    APP_ID, Application, PROFILE, SETTINGS_KEY_CURRENT_SESSION,
    account_chooser_dialog::AccountChooserDialog,
    account_settings::AccountSettings,
    account_switcher::{AccountSwitcherButton, AccountSwitcherPopover},
    components::OfflineBanner,
    error_page::ErrorPage,
    intent::SessionIntent,
    login::Login,
    prelude::*,
    secret::SESSION_ID_LENGTH,
    session::{Session, SessionState},
    session_list::{FailedSession, SessionInfo},
    session_view::SessionView,
    toast,
    utils::{FixedSelection, LoadingState, key_bindings},
};

/// A page of the main window stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowPage {
    /// The loading page.
    Loading,
    /// The login view.
    Login,
    /// The session view.
    Session,
    /// The error page.
    Error,
}

/// The `SessionView` actions that the window forwards to, as
/// `(the name under win., the name under session.)`.
///
/// The macOS menu bar can only reach `app.` and `win.` actions -- GTK's quartz
/// backend builds it from a muxer holding the application's actions and the
/// active window's, and nothing else -- while everything the main menu offers
/// is installed on the `SessionView` widget. These are the way across.
const FORWARDED_SESSION_ACTIONS: &[(&str, &str)] = &[
    ("close-room", "close-room"),
    ("create-direct-chat", "create-direct-chat"),
    ("create-room", "create-room"),
    ("join-room", "join-room"),
    ("select-next-room", "select-next-room"),
    ("select-next-unread-room", "select-next-unread-room"),
    ("select-prev-room", "select-prev-room"),
    ("select-prev-unread-room", "select-prev-unread-room"),
    ("select-unread-room", "select-unread-room"),
    // The obvious name is taken by the parameterized action that the session
    // view forwards to in the other direction.
    ("show-image-packs", "open-image-packs"),
    ("toggle-room-search", "toggle-room-search"),
];

impl WindowPage {
    /// Get the name of this page.
    const fn name(self) -> &'static str {
        match self {
            Self::Loading => "loading",
            Self::Login => "login",
            Self::Session => "session",
            Self::Error => "error",
        }
    }

    /// Get the page matching the given name.
    ///
    /// Panics if the name does not match any of the variants.
    fn from_name(name: &str) -> Self {
        match name {
            "loading" => Self::Loading,
            "login" => Self::Login,
            "session" => Self::Session,
            "error" => Self::Error,
            _ => panic!("Unknown WindowPage: {name}"),
        }
    }
}

mod imp {
    use std::{cell::RefCell, rc::Rc};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[cfg_attr(
        not(target_os = "windows"),
        template(resource = "/org/gnome/Fractal/ui/window.ui")
    )]
    #[cfg_attr(
        target_os = "windows",
        template(resource = "/org/gnome/Fractal/ui/window-windows.ui")
    )]
    #[properties(wrapper_type = super::Window)]
    pub struct Window {
        #[template_child]
        main_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        loading: TemplateChild<gtk::WindowHandle>,
        #[template_child]
        login: TemplateChild<Login>,
        #[template_child]
        error_page: TemplateChild<ErrorPage>,
        #[template_child]
        pub(super) session_view: TemplateChild<SessionView>,
        #[template_child]
        toast_overlay: TemplateChild<adw::ToastOverlay>,
        /// Whether the window should be in compact view.
        ///
        /// It means that the horizontal size is not large enough to hold all
        /// the content.
        #[property(get, set = Self::set_compact, explicit_notify)]
        compact: Cell<bool>,
        /// The selection of the logged-in sessions.
        ///
        /// The one that is selected being the one that is visible.
        #[property(get)]
        session_selection: FixedSelection,
        /// The account switcher popover.
        pub(super) account_switcher: AccountSwitcherPopover,
        /// The native frame subclass that stands in for CSD.
        ///
        /// See `doc/windows-snapping-plan.md`. Kept only to be dropped with
        /// the window, which removes the subclass.
        #[cfg(target_os = "windows")]
        frame: RefCell<Option<windows_frame::NativeFrame>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Window {
        const NAME: &'static str = "Window";
        type Type = super::Window;
        // See `doc/windows-snapping-plan.md`: `AdwApplicationWindow` cannot
        // be given server-side decoration, so Windows gets a plain
        // `gtk::ApplicationWindow` and a native frame subclass instead.
        #[cfg(not(target_os = "windows"))]
        type ParentType = adw::ApplicationWindow;
        #[cfg(target_os = "windows")]
        type ParentType = gtk::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            AccountSwitcherButton::ensure_type();
            OfflineBanner::ensure_type();

            Self::bind_template(klass);

            klass.add_binding_action(gdk::Key::v, key_bindings::PRIMARY_MASK, "win.paste");
            klass.add_binding_action(gdk::Key::Insert, gdk::ModifierType::SHIFT_MASK, "win.paste");
            klass.install_action("win.paste", None, |obj, _, _| {
                obj.imp().session_view.handle_paste_action();
            });

            klass.install_action(
                "win.open-account-settings",
                Some(&String::static_variant_type()),
                |obj, _, variant| {
                    if let Some(session_id) = variant.and_then(glib::Variant::get::<String>) {
                        obj.imp().open_account_settings(&session_id);
                    }
                },
            );

            klass.install_action(
                "win.open-image-packs",
                Some(&String::static_variant_type()),
                |obj, _, variant| {
                    if let Some(session_id) = variant.and_then(glib::Variant::get::<String>) {
                        obj.imp().open_image_packs(&session_id);
                    }
                },
            );

            klass.install_action("win.new-session", None, |obj, _, _| {
                obj.imp().set_visible_page(WindowPage::Login);
            });
            klass.install_action("win.show-session", None, |obj, _, _| {
                obj.imp().show_session();
            });

            klass.install_action("win.toggle-fullscreen", None, |obj, _, _| {
                if obj.is_fullscreen() {
                    obj.unfullscreen();
                } else {
                    obj.fullscreen();
                }
            });

            for (window_action, _) in FORWARDED_SESSION_ACTIONS {
                klass.install_action(
                    &format!("win.{window_action}"),
                    None,
                    |obj, action_name, _| {
                        obj.imp().forward_to_session_view(action_name);
                    },
                );
            }
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for Window {
        fn constructed(&self) {
            self.parent_constructed();

            // Development Profile
            if PROFILE.should_use_devel_class() {
                self.obj().add_css_class("devel");
            }

            // Windows draws its own window controls in a way libadwaita's
            // stylesheet does not describe, so the stylesheet says what they
            // should look like here and this is what turns those rules on.
            #[cfg(target_os = "windows")]
            self.obj().add_css_class("windows");

            // The native frame needs an HWND, which does not exist until
            // realize -- and has to be installed before the window is shown,
            // so the native caption never appears.
            #[cfg(target_os = "windows")]
            self.obj().connect_realize(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_window| imp.install_native_frame()
            ));

            self.load_window_size();
            self.update_forwarded_session_actions();

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

            self.session_selection.set_item_equivalence_fn(|lhs, rhs| {
                let lhs = lhs
                    .downcast_ref::<SessionInfo>()
                    .expect("session selection item should be a SessionInfo");
                let rhs = rhs
                    .downcast_ref::<SessionInfo>()
                    .expect("session selection item should be a SessionInfo");

                lhs.session_id() == rhs.session_id()
            });
            self.session_selection.connect_selected_item_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.update_selected_session();
                }
            ));
            self.session_selection.connect_is_empty_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |session_selection| {
                    imp.obj()
                        .action_set_enabled("win.show-session", !session_selection.is_empty());
                }
            ));

            let app = Application::default();
            let session_list = app.session_list();

            self.session_selection.set_model(Some(session_list.clone()));

            if session_list.state() == LoadingState::Ready {
                self.finish_session_selection_init();
            } else {
                session_list.connect_state_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |session_list| {
                        if session_list.state() == LoadingState::Ready {
                            imp.finish_session_selection_init();
                        }
                    }
                ));
            }

            // Coming back to the foreground is the moment every session's
            // connectivity knowledge went stale: the background cut the
            // process off the network, froze it mid-claim, and the claim —
            // usually "offline", since the syncs died first — would otherwise
            // greet the user as if it were news.
            #[cfg(target_os = "android")]
            self.obj().connect_is_active_notify(|window| {
                if !window.is_active() {
                    return;
                }

                let app = Application::default();
                let session_list = app.session_list();
                for position in 0..session_list.n_items() {
                    if let Some(session) = session_list.item(position).and_downcast::<Session>() {
                        session.recheck_connectivity();
                    }
                }
            });
        }
    }

    impl WindowImpl for Window {
        fn close_request(&self) -> glib::Propagation {
            // Android has no window to close: the system's back gesture and
            // back button arrive here as a close request, because a delete
            // event is all GDK's Android backend has to turn them into. Back
            // means "leave the thing I am looking at", and only leaves the app
            // when there is nothing left to leave, so this has to be answered
            // by unwinding the view one step at a time and letting the close
            // through only once there is nothing to unwind.
            #[cfg(target_os = "android")]
            if self.handle_back_navigation() {
                return glib::Propagation::Stop;
            }

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
                WindowPage::Session => self.session_view.grab_focus(),
                WindowPage::Error => self.error_page.grab_focus(),
            }
        }
    }

    impl ApplicationWindowImpl for Window {}
    #[cfg(not(target_os = "windows"))]
    impl AdwApplicationWindowImpl for Window {}

    impl Window {
        /// Go back one step through what is on screen, if there is a step to
        /// go back through.
        ///
        /// Returns whether anything was closed, which is whether the close
        /// request that asked should be refused.
        ///
        /// The order is what is on top: a popover is above a dialog, a dialog
        /// is above the page under it, and the page decides for itself.
        #[cfg(target_os = "android")]
        fn handle_back_navigation(&self) -> bool {
            let obj = self.obj();

            // A popover holds the focus while it is up, which is the only
            // handle on it from here -- GTK exposes no list of open popovers.
            // Spelled out because `GtkWindow` and `GtkRoot` both have a
            // `focus` getter and the compiler cannot pick between them.
            let focus = gtk::prelude::GtkWindowExt::focus(&*obj);
            if let Some(popover) = focus
                .and_then(|widget| widget.ancestor(gtk::Popover::static_type()))
                .and_downcast::<gtk::Popover>()
            {
                popover.popdown();
                return true;
            }

            // A dialog is not handled here and cannot be: `close-request` is
            // `G_SIGNAL_RUN_LAST` with a boolean accumulator, and
            // `AdwDialogHost` connects a handler that closes the visible dialog
            // and stops the emission. Connected handlers run before the class
            // closure, so by the time this vfunc could look, libadwaita has
            // already closed the dialog and this was never called. Which is the
            // behaviour we want anyway; a dialog that wants back to mean
            // something else says so in its own `close_attempt`.

            match self.visible_page() {
                WindowPage::Login => self.login.handle_back_navigation(),
                WindowPage::Session => self.session_view.handle_back_navigation(),
                WindowPage::Loading | WindowPage::Error => false,
            }
        }

        /// Set whether the window should be in compact view.
        fn set_compact(&self, compact: bool) {
            if compact == self.compact.get() {
                return;
            }

            self.compact.set(compact);
            self.obj().notify_compact();
        }

        /// Finish the initialization of the session selection, when the session
        /// list is ready.
        fn finish_session_selection_init(&self) {
            for item in self.session_selection.iter::<glib::Object>() {
                if let Some(failed) = item.ok().and_downcast_ref::<FailedSession>() {
                    toast!(self.obj(), failed.error().to_user_facing());
                }
            }

            self.restore_current_visible_session();

            self.session_selection.connect_selected_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |session_selection| {
                    if session_selection.selected() == gtk::INVALID_LIST_POSITION {
                        imp.select_first_session();
                    }
                }
            ));

            if self.session_selection.selected() == gtk::INVALID_LIST_POSITION {
                self.select_first_session();
            }
        }

        /// Select the first session in the session list.
        ///
        /// To be used when there is no current selection.
        fn select_first_session(&self) {
            // Select the first session in the list.
            let selected_session = self.session_selection.item(0);

            if selected_session.is_none() {
                // There are no more sessions.
                self.set_visible_page(WindowPage::Login);
            }

            self.session_selection.set_selected_item(selected_session);
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

        /// Restore the currently visible session from the settings.
        fn restore_current_visible_session(&self) {
            let settings = Application::default().settings();
            let mut current_session_setting =
                settings.string(SETTINGS_KEY_CURRENT_SESSION).to_string();

            // Session IDs have been truncated in version 6 of StoredSession.
            if current_session_setting.len() > SESSION_ID_LENGTH {
                current_session_setting.truncate(SESSION_ID_LENGTH);

                if let Err(error) =
                    settings.set_string(SETTINGS_KEY_CURRENT_SESSION, &current_session_setting)
                {
                    warn!("Could not save current session: {error}");
                }
            }

            if let Some(session) = Application::default()
                .session_list()
                .get(&current_session_setting)
            {
                self.session_selection.set_selected_item(Some(session));
            }
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
            WindowPage::from_name(
                &self
                    .main_stack
                    .visible_child_name()
                    .expect("stack should always have a visible child name"),
            )
        }

        /// The ID of the currently visible session, if any.
        pub(super) fn current_session_id(&self) -> Option<String> {
            self.session_selection
                .selected_item()
                .and_downcast::<SessionInfo>()
                .map(|s| s.session_id())
        }

        /// Set the current session by its ID.
        ///
        /// Returns `true` if the session was set as the current session.
        pub(super) fn set_current_session_by_id(&self, session_id: &str) -> bool {
            let Some(index) = Application::default().session_list().index(session_id) else {
                return false;
            };

            let index = index as u32;
            let prev_selected = self.session_selection.selected();

            if index == prev_selected {
                // Make sure the session is displayed;
                self.show_session();
            } else {
                self.session_selection.set_selected(index);
            }

            true
        }

        /// Update the selected session in the session view.
        fn update_selected_session(&self) {
            let Some(selected_session) = self
                .session_selection
                .selected_item()
                .and_downcast::<SessionInfo>()
            else {
                return;
            };

            let session = selected_session.downcast_ref::<Session>();
            self.session_view.set_session(session);

            // Show the selected session automatically only if we are not showing a more
            // important view.
            if matches!(
                self.visible_page(),
                WindowPage::Session | WindowPage::Loading
            ) {
                self.show_session();
            }
        }

        /// Show the selected session.
        ///
        /// The displayed view will change according to the current session.
        pub(super) fn show_session(&self) {
            let Some(selected_session) = self
                .session_selection
                .selected_item()
                .and_downcast::<SessionInfo>()
            else {
                return;
            };

            if let Some(session) = selected_session.downcast_ref::<Session>() {
                if session.state() == SessionState::Ready {
                    self.set_visible_page(WindowPage::Session);
                } else {
                    let ready_handler_cell: Rc<RefCell<Option<glib::SignalHandlerId>>> =
                        Rc::default();
                    let ready_handler = session.connect_ready(clone!(
                        #[weak(rename_to = imp)]
                        self,
                        #[strong]
                        ready_handler_cell,
                        move |session| {
                            if let Some(handler) = ready_handler_cell.take() {
                                session.disconnect(handler);
                            }

                            imp.update_selected_session();
                        }
                    ));
                    ready_handler_cell.replace(Some(ready_handler));

                    self.set_visible_page(WindowPage::Loading);
                }

                // We need to grab the focus so that keyboard shortcuts work.
                self.session_view.grab_focus();
            } else if let Some(failed) = selected_session.downcast_ref::<FailedSession>() {
                self.error_page
                    .display_session_error(&failed.error().to_user_facing());
                self.set_visible_page(WindowPage::Error);
            } else {
                self.set_visible_page(WindowPage::Loading);
            }
        }

        /// Set the visible page of the window.
        fn set_visible_page(&self, page: WindowPage) {
            self.main_stack.set_visible_child_name(page.name());
            self.update_forwarded_session_actions();
        }

        /// Activate the `SessionView` action that the given window action
        /// stands in for.
        fn forward_to_session_view(&self, window_action: &str) {
            let name = window_action.trim_start_matches("win.");

            let Some((_, session_action)) =
                FORWARDED_SESSION_ACTIONS.iter().find(|(n, _)| *n == name)
            else {
                error!("Tried to forward unknown window action `{window_action}`");
                return;
            };

            let session_action = format!("session.{session_action}");
            if self
                .session_view
                .activate_action(&session_action, None)
                .is_err()
            {
                error!("Could not activate action `{session_action}`");
            }
        }

        /// Enable the forwarded actions only while a session is on screen.
        ///
        /// Nothing else needs this -- the widget they lead to is not reachable
        /// otherwise -- but the macOS menu bar is always on screen, and an item
        /// that does nothing is worse than one that is visibly unavailable.
        fn update_forwarded_session_actions(&self) {
            let enabled = self.visible_page() == WindowPage::Session;
            let obj = self.obj();

            for (window_action, _) in FORWARDED_SESSION_ACTIONS {
                obj.action_set_enabled(&format!("win.{window_action}"), enabled);
            }
        }

        /// Open the error page and display the given secret error message.
        pub(super) fn show_secret_error(&self, message: &str) {
            self.error_page.display_secret_error(message);
            self.set_visible_page(WindowPage::Error);
        }

        /// Add the given toast to the queue.
        pub(super) fn add_toast(&self, toast: adw::Toast) {
            self.toast_overlay.add_toast(toast);
        }

        /// Open the account settings for the session with the given ID.
        pub(super) fn open_account_settings(&self, session_id: &str) {
            let Some(session) = Application::default()
                .session_list()
                .get(session_id)
                .and_downcast::<Session>()
            else {
                error!("Tried to open account settings of unknown session with ID '{session_id}'");
                return;
            };

            let dialog = AccountSettings::new(&session);
            dialog.present(Some(&*self.obj()));
        }

        /// Open the image packs of the session with the given ID.
        ///
        /// The page is in the account settings, because the state it needs is
        /// account data, but managing image packs is not something a user
        /// looks for there, so it has its own way in.
        fn open_image_packs(&self, session_id: &str) {
            let Some(session) = Application::default()
                .session_list()
                .get(session_id)
                .and_downcast::<Session>()
            else {
                error!("Tried to open the image packs of unknown session with ID '{session_id}'");
                return;
            };

            let dialog = AccountSettings::new(&session);
            dialog.show_image_packs_tab();
            dialog.present(Some(&*self.obj()));
        }

        /// Install the native Win32 frame subclass that stands in for CSD.
        ///
        /// See `doc/windows-snapping-plan.md`. Called from `realize`, once
        /// the window has an `HWND` and before it is shown, so the native
        /// caption never appears.
        #[cfg(target_os = "windows")]
        fn install_native_frame(&self) {
            let obj = self.obj();
            let Some(surface) = obj.surface() else {
                warn!("Could not install the native window frame: no surface yet");
                return;
            };
            let hwnd = gdk4_win32::Win32Surface::impl_hwnd(&surface);
            if hwnd.0.is_null() {
                warn!("Could not install the native window frame: no HWND yet");
                return;
            }

            // The button `frame_hit` last resolved a point to, so
            // `show_maximize_state` can light up (or clear) the one the
            // pointer is actually over. Every page of `main_stack` has its
            // own header bar, and so its own maximize button; searching the
            // whole window for `.maximize` -- as this used to -- can find a
            // hidden page's button instead of the visible one.
            let hovered_button: Rc<RefCell<Option<gtk::Button>>> = Rc::default();

            let hit_test_window = obj.downgrade();
            let hit_test_button = Rc::clone(&hovered_button);
            let hit_test: windows_frame::HitTester = Box::new(move |x, y| {
                hit_test_window
                    .upgrade()
                    .map_or(windows_frame::FrameHit::Client, |window| {
                        frame_hit(&window, x, y, &hit_test_button)
                    })
            });

            let maximize: windows_frame::MaximizeWatcher = Box::new(move |state| {
                show_maximize_state(&hovered_button, state);
            });

            let Some(frame) = windows_frame::install(hwnd, hit_test, maximize) else {
                warn!("Could not install the native window frame");
                return;
            };
            self.frame.replace(Some(frame));

            // `GtkWindowControls` is supposed to swap the maximize button's
            // icon and tooltip on its own when `maximized` changes, but
            // that only holds for a window it drew the decoration of --
            // untested, and not to be trusted, on a server-side-decorated
            // toplevel. Done by hand instead, and once now for a window
            // restored already maximized, since `load_window_size` set
            // that before this handler existed to hear about it.
            self.obj().connect_maximized_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |window| imp.update_maximize_icon(window.is_maximized())
            ));
            self.update_maximize_icon(self.obj().is_maximized());
        }

        /// Set the visible page's maximize button to look like a maximize or
        /// a restore button, matching `maximized`.
        #[cfg(target_os = "windows")]
        fn update_maximize_icon(&self, maximized: bool) {
            let Some(visible_page) = self.main_stack.visible_child() else {
                return;
            };
            let Some(button) = find_maximize_button(&visible_page) else {
                return;
            };
            button.set_icon_name(if maximized {
                "window-restore-symbolic"
            } else {
                "window-maximize-symbolic"
            });
            button.set_tooltip_text(Some(&if maximized {
                gettext("Restore")
            } else {
                gettext("Maximize")
            }));
        }
    }

    /// What is under a client-area point (physical pixels from its top-left):
    /// the header bar background if nothing interactive is there, the
    /// maximize window control, or ordinary content.
    ///
    /// The window switches between several pages, each with its own header
    /// bar (or none, for the session page's own layout), so this looks for
    /// *a* header bar in the picked widget's ancestry rather than a single
    /// one fixed at construction. A resolved maximize button is stashed in
    /// `hovered` -- see `Window::install_native_frame`.
    #[cfg(target_os = "windows")]
    fn frame_hit(
        window: &super::Window,
        x: i32,
        y: i32,
        hovered: &RefCell<Option<gtk::Button>>,
    ) -> windows_frame::FrameHit {
        let Some(surface) = window.surface() else {
            return windows_frame::FrameHit::Client;
        };
        let scale = surface.scale();
        let (tx, ty) = window.surface_transform();
        let lx = f64::from(x) / scale - tx;
        let ly = f64::from(y) / scale - ty;
        let Some(picked) = window.pick(lx, ly, gtk::PickFlags::DEFAULT) else {
            return windows_frame::FrameHit::Client;
        };

        // Walk up to a header bar: any interactive control on the way is
        // content.
        let mut widget = Some(picked);
        while let Some(w) = widget {
            if let Some(button) = w.downcast_ref::<gtk::Button>() {
                if button.has_css_class("maximize") {
                    hovered.replace(Some(button.clone()));
                    return windows_frame::FrameHit::MaximizeButton;
                }
                return windows_frame::FrameHit::Client;
            }
            if w.is::<gtk::Entry>()
                || w.is::<gtk::Text>()
                || w.is::<gtk::MenuButton>()
                || w.is::<gtk::SearchEntry>()
            {
                return windows_frame::FrameHit::Client;
            }
            if w.is::<adw::HeaderBar>() || w.is::<gtk::HeaderBar>() {
                return windows_frame::FrameHit::Caption;
            }
            widget = w.parent();
        }
        windows_frame::FrameHit::Client
    }

    /// Lights the maximize button up, or puts it out, on word from the frame.
    ///
    /// Also clears whatever the toolkit still thinks is hovered: the pointer
    /// crossing into a rectangle Windows owns leaves GTK believing it never
    /// left the button next door, and that button stays lit.
    #[cfg(target_os = "windows")]
    fn show_maximize_state(
        hovered: &RefCell<Option<gtk::Button>>,
        state: windows_frame::MaximizeState,
    ) {
        let Some(button) = hovered.borrow().clone() else {
            return;
        };
        button.unset_state_flags(gtk::StateFlags::PRELIGHT | gtk::StateFlags::ACTIVE);
        let flags = match state {
            windows_frame::MaximizeState::Away => return,
            windows_frame::MaximizeState::Hover => gtk::StateFlags::PRELIGHT,
            windows_frame::MaximizeState::Pressed => {
                gtk::StateFlags::PRELIGHT | gtk::StateFlags::ACTIVE
            }
        };
        button.set_state_flags(flags, false);
        // The pointer arriving here just crossed out of client territory --
        // minimize and close are ordinary client-area buttons, and moving
        // straight from one of them into this non-client rectangle does not
        // reliably deliver GTK the crossing it would need to un-light
        // whichever one the pointer left.
        clear_sibling_button_state(&button);
    }

    /// Clears the prelight/active flags of every window-control button next
    /// to `button` (minimize, close) other than `button` itself.
    ///
    /// See `show_maximize_state`.
    #[cfg(target_os = "windows")]
    fn clear_sibling_button_state(button: &gtk::Button) {
        let widget = button.clone().upcast::<gtk::Widget>();
        let Some(parent) = widget.parent() else {
            return;
        };
        let mut child = parent.first_child();
        while let Some(w) = child {
            if w != widget
                && let Some(sibling) = w.downcast_ref::<gtk::Button>()
            {
                sibling.unset_state_flags(gtk::StateFlags::PRELIGHT | gtk::StateFlags::ACTIVE);
            }
            child = w.next_sibling();
        }
    }

    /// Depth-first search for a descendant `GtkButton` with the `maximize`
    /// CSS class, starting from `widget`.
    ///
    /// `GtkWindowControls` draws the window buttons without exposing a
    /// direct handle to them. Called with `main_stack`'s visible page as
    /// `widget` -- not the window -- so a hidden page's own maximize button
    /// is never the one found.
    #[cfg(target_os = "windows")]
    fn find_maximize_button(widget: &gtk::Widget) -> Option<gtk::Button> {
        if let Some(button) = widget.downcast_ref::<gtk::Button>()
            && button.has_css_class("maximize")
        {
            return Some(button.clone());
        }

        let mut child = widget.first_child();
        while let Some(w) = child {
            if let Some(found) = find_maximize_button(&w) {
                return Some(found);
            }
            child = w.next_sibling();
        }
        None
    }
}

#[cfg(not(target_os = "windows"))]
glib::wrapper! {
    /// The main window.
    pub struct Window(ObjectSubclass<imp::Window>)
        @extends gtk::Widget, gtk::Window, gtk::ApplicationWindow, adw::ApplicationWindow,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Root, gtk::Native,
                    gtk::ShortcutManager, gio::ActionMap, gio::ActionGroup;
}

// See `doc/windows-snapping-plan.md`: on Windows the window is a plain
// `gtk::ApplicationWindow` rather than an `adw::ApplicationWindow`, so it is
// not in `@extends` here.
#[cfg(target_os = "windows")]
glib::wrapper! {
    /// The main window.
    pub struct Window(ObjectSubclass<imp::Window>)
        @extends gtk::Widget, gtk::Window, gtk::ApplicationWindow,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Root, gtk::Native,
                    gtk::ShortcutManager, gio::ActionMap, gio::ActionGroup;
}

impl Window {
    pub fn new(app: &Application) -> Self {
        glib::Object::builder()
            .property("application", Some(app))
            .property("icon-name", Some(APP_ID))
            .build()
    }

    /// Add the given session to the session list and select it.
    pub(crate) fn add_session(&self, session: Session) {
        let index = Application::default().session_list().insert(session);
        self.session_selection().set_selected(index as u32);
        self.imp().show_session();
    }

    /// The ID of the currently visible session, if any.
    pub(crate) fn current_session_id(&self) -> Option<String> {
        self.imp().current_session_id()
    }

    /// Add the given toast to the queue.
    pub(crate) fn add_toast(&self, toast: adw::Toast) {
        self.imp().add_toast(toast);
    }

    /// The account switcher popover.
    pub(crate) fn account_switcher(&self) -> &AccountSwitcherPopover {
        &self.imp().account_switcher
    }

    /// The `SessionView` of this window.
    pub(crate) fn session_view(&self) -> &SessionView {
        &self.imp().session_view
    }

    /// Open the account settings of the session with the given ID.
    pub(crate) fn open_account_settings(&self, session_id: &str) {
        self.imp().open_account_settings(session_id);
    }

    /// Open the error page and display the given secret error message.
    pub(crate) fn show_secret_error(&self, message: &str) {
        self.imp().show_secret_error(message);
    }

    /// Ask the user to choose a session.
    ///
    /// The session list must be ready.
    ///
    /// Returns the ID of the selected session, if any.
    pub(crate) async fn ask_session(&self) -> Option<String> {
        let dialog = AccountChooserDialog::new(Application::default().session_list());
        dialog.choose_account(self).await
    }

    /// Process the given session intent.
    ///
    /// The session must be ready.
    pub(crate) fn process_session_intent(&self, session_id: &str, intent: SessionIntent) {
        if !self.imp().set_current_session_by_id(session_id) {
            error!("Cannot switch to unknown session with ID `{session_id}`");
            return;
        }

        self.session_view().process_intent(intent);
    }
}
