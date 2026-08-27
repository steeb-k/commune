//! The one-time background-delivery setup, step 5 of
//! `doc/android-push-plan.md`.
//!
//! A person installing Commune should not have to know anything about
//! UnifiedPush, distributors or foreground service budgets. This dialog is
//! shown once, the first time the window is presented with a session in it,
//! and does three things in order: says why notifications matter — so the
//! `POST_NOTIFICATIONS` prompt arrives in context instead of cold — detects a
//! distributor and offers push in one tap, and offers "keep Commune running"
//! as the honest no-extra-app alternative, six-hour cap named. Declining is a
//! decision, not a dead end: dismissing the dialog keeps the defaults and the
//! question is not asked again.
//!
//! The choice lands in the `background-delivery` settings key that
//! `android_push::service_needed()` reads. "Push" writes `auto` rather than
//! `push`, on purpose: the person chose the better delivery, not a promise to
//! never fall back when their distributor breaks.
//!
//! With no distributor installed, the push row opens ntfy's page in whatever
//! store the device has — `market:` resolves to any of them — with F-Droid's
//! web page as the fallback when nothing answers. Coming back and tapping the
//! row again re-detects; the "asked once" flag is only written when the
//! dialog closes, so the round trip through the store does not lose the
//! setup.

use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{CompositeTemplate, glib};
use tracing::{debug, error, warn};

use crate::{
    Application,
    utils::{android, android_notifications, android_push},
};

/// Where to get ntfy, asked of whatever store the device has.
const NTFY_MARKET_URI: &str = "market://details?id=io.heckel.ntfy";

/// The fallback when no store answers: F-Droid's page, in the browser.
const NTFY_WEB_URI: &str = "https://f-droid.org/packages/io.heckel.ntfy/";

mod imp {
    use std::sync::atomic::AtomicBool;

    use glib::subclass::InitializingObject;

    use super::*;

    /// Whether the dialog was presented this run.
    ///
    /// Guards the presenters racing while it is open; the durable flag is
    /// written when the dialog closes, whichever way it closes.
    pub(super) static PRESENTED: AtomicBool = AtomicBool::new(false);

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/Fractal/ui/android_setup_dialog/mod.ui")]
    pub struct AndroidSetupDialog {
        #[template_child]
        push_row: TemplateChild<adw::ActionRow>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AndroidSetupDialog {
        const NAME: &'static str = "AndroidSetupDialog";
        type Type = super::AndroidSetupDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for AndroidSetupDialog {
        fn constructed(&self) {
            self.parent_constructed();
            self.update_push_row();
        }
    }

    impl WidgetImpl for AndroidSetupDialog {}

    impl AdwDialogImpl for AndroidSetupDialog {
        fn closed(&self) {
            // However it went — a choice, or a dismissal that keeps the
            // defaults — the question was asked and answered once.
            android_push::mark_setup_shown();
        }
    }

    #[gtk::template_callbacks]
    impl AndroidSetupDialog {
        /// Reflect in the push row whether a distributor is installed.
        fn update_push_row(&self) {
            let subtitle = if android_push::has_distributor() {
                gettext(
                    "New messages arrive the moment they are sent, through the notification app that is already installed.",
                )
            } else {
                gettext("Needs a small notification app. Tap to get ntfy, then come back here.")
            };
            self.push_row.set_subtitle(&subtitle);
        }

        /// The push row was chosen.
        #[template_callback]
        fn push_activated(&self) {
            if android_push::has_distributor() {
                Self::set_delivery("auto");
                self.request_permission();
                android_push::ensure_registered();
                self.obj().close();
            } else {
                self.open_store();
                // Re-detect for when they come back — and tapping the row
                // again after installing completes the setup.
                self.update_push_row();
            }
        }

        /// The foreground service row was chosen.
        #[template_callback]
        fn service_activated(&self) {
            Self::set_delivery("service");
            self.request_permission();
            self.obj().close();
        }

        /// Write the chosen `background-delivery` mode.
        fn set_delivery(mode: &str) {
            if let Err(error) = Application::default()
                .settings()
                .set_string("background-delivery", mode)
            {
                error!("Could not save the background delivery mode: {error}");
            }
        }

        /// Ask for permission to post notifications, now that the reason for
        /// them has been on screen.
        fn request_permission(&self) {
            if let Some(window) = self.obj().root().and_downcast::<gtk::Window>() {
                android_notifications::request_permission(&window);
            }
        }

        /// Open ntfy's page in a store, or in the browser when there is none.
        fn open_store(&self) {
            let Some(window) = self.obj().root().and_downcast::<gtk::Window>() else {
                return;
            };

            if let Err(error) = android::launch_uri(&window, NTFY_MARKET_URI) {
                debug!("No store answered for ntfy; opening the F-Droid page: {error}");
                if let Err(error) = android::launch_uri(&window, NTFY_WEB_URI) {
                    warn!("Could not open a page to get ntfy: {error}");
                }
            }
        }
    }
}

glib::wrapper! {
    /// The one-time background-delivery setup dialog.
    pub struct AndroidSetupDialog(ObjectSubclass<imp::AndroidSetupDialog>)
        @extends gtk::Widget, adw::Dialog,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::ShortcutManager;
}

impl AndroidSetupDialog {
    /// Present the dialog over `parent`, if it has never been shown.
    ///
    /// Quietly does nothing when it has been — the flag lives beside the push
    /// registration state, in `no_backup` — or when it is already open.
    pub(crate) fn maybe_present(parent: &gtk::Window) {
        use std::sync::atomic::Ordering;

        if !android_push::should_present_setup() {
            return;
        }
        if imp::PRESENTED.swap(true, Ordering::Relaxed) {
            return;
        }

        let dialog: Self = glib::Object::new();
        dialog.present(Some(parent));
    }
}
