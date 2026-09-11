//! Keeping this installation current.
//!
//! The part that is the same on every platform — reading the release feed,
//! checking its signature, deciding whether what it offers is newer — is
//! [`commune_core::updates`]. This is the application's half: when to ask,
//! what to say about the answer, and how to hand the downloaded artifact to
//! the platform that knows how to install it.
//!
//! There is one of these, owned by [`crate::Application`], because there is
//! one installation. The settings row and the toast are two views of it
//! rather than two things that each check.
//!
//! # When it stays quiet
//!
//! An update that cannot be installed must not be announced, so the check is
//! not made at all unless this build is one that could replace itself:
//!
//! * Inside Flatpak, and on Linux generally, the app store owns this.
//! * On Windows, unless the executable is under `%LOCALAPPDATA%\Programs` —
//!   anywhere else is a build directory or an unpacked `.zip`, and running an
//!   installer would add a second copy rather than replace this one.
//! * On macOS, unless the bundle carries a real Team ID. An ad-hoc signature
//!   means a developer's own build, and replacing it with a release would
//!   change the identity under every Keychain item it has stored.

use std::{cell::Cell, path::Path, sync::Arc, time::Duration};

use adw::{prelude::*, subclass::prelude::*};
use commune_core::{
    UserFacingError,
    updates::{self, Channel, DownloadProgress, Release, UpdateSettings},
};
use futures_channel::mpsc;
use futures_util::StreamExt;
use gettextrs::gettext;
use gtk::{glib, glib::clone};
use tracing::{debug, error, warn};

use crate::{gettext_f, spawn, spawn_tokio, utils::DataType};

/// How long after startup the first automatic check waits.
///
/// Long enough to be past the first sync, so that a check never competes with
/// the thing the user actually opened the application for.
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(90);

/// How often the timer comes back to ask whether a check is due.
///
/// Not the interval between checks — [`UpdateSettings::is_check_due()`] owns
/// that, and it has to, because it is the only thing that survives a restart.
/// This is just how often a long-running process wakes up to look.
const TIMER_INTERVAL: Duration = Duration::from_hours(1);

/// Where an update is being downloaded to.
fn download_dir() -> std::path::PathBuf {
    DataType::Cache.dir_path().join("updates")
}

/// What the updater is doing, and what it last found.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, glib::Enum)]
#[enum_type(name = "UpdateState")]
pub enum UpdateState {
    /// Nothing has been asked yet this run.
    #[default]
    Idle,
    /// A check is in flight.
    Checking,
    /// The feed was read and this build is current.
    UpToDate,
    /// There is a newer release to install.
    Available,
    /// The artifact is being fetched.
    Downloading,
    /// The artifact is on disk and the installer is starting.
    Installing,
    /// The last attempt did not finish.
    Failed,
}

mod imp {
    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::Updates)]
    pub struct Updates {
        /// What the updater is doing.
        #[property(get, builder(UpdateState::default()))]
        pub state: Cell<UpdateState>,
        /// The version of the release that is available, when one is.
        #[property(get)]
        pub available_version: RefCell<String>,
        /// How much of the download has arrived, from 0 to 1.
        #[property(get)]
        pub progress: Cell<f64>,
        /// A sentence describing [`Self::state`], for a row's subtitle.
        #[property(get)]
        pub status: RefCell<String>,
        /// Whether this installation could install an update at all.
        #[property(get)]
        pub supported: Cell<bool>,
        /// The release the last check found.
        pub release: RefCell<Option<Release>>,
        /// The stored settings, read once.
        ///
        /// Kept here rather than loaded per call: every read of them is a
        /// `GSettings` lookup and a JSON parse, and this object is the only
        /// thing in the process that writes them.
        pub settings: RefCell<UpdateSettings>,
    }

    use std::cell::RefCell;

    #[glib::object_subclass]
    impl ObjectSubclass for Updates {
        const NAME: &'static str = "Updates";
        type Type = super::Updates;
    }

    #[glib::derived_properties]
    impl ObjectImpl for Updates {
        fn constructed(&self) {
            self.parent_constructed();

            self.settings.replace(UpdateSettings::load());
            self.supported.set(super::is_supported());
            self.obj().update_status();
        }

        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: std::sync::OnceLock<Vec<glib::subclass::Signal>> =
                std::sync::OnceLock::new();

            SIGNALS.get_or_init(|| {
                vec![
                    // Emitted once per release found by a check the user did
                    // not ask for, so that the window can mention it. A check
                    // the user made themselves is answered by the row they
                    // pressed, and does not need announcing twice.
                    glib::subclass::Signal::builder("update-announced")
                        .param_types([String::static_type()])
                        .build(),
                ]
            })
        }
    }
}

glib::wrapper! {
    /// The state of this installation with respect to the release feed.
    pub struct Updates(ObjectSubclass<imp::Updates>);
}

impl Updates {
    /// Create a new `Updates`.
    pub(crate) fn new() -> Self {
        glib::Object::new()
    }

    /// Start the timer that makes automatic checks.
    ///
    /// Does nothing when this installation cannot update itself, so a Flatpak
    /// never contacts the feed at all.
    pub(crate) fn start(&self) {
        if !self.supported() {
            debug!("This installation does not update itself; not checking");
            return;
        }

        glib::timeout_add_local_once(
            FIRST_CHECK_DELAY,
            clone!(
                #[weak(rename_to = obj)]
                self,
                move || {
                    obj.check_if_due();

                    glib::timeout_add_local(
                        TIMER_INTERVAL,
                        clone!(
                            #[weak]
                            obj,
                            #[upgrade_or]
                            glib::ControlFlow::Break,
                            move || {
                                obj.check_if_due();
                                glib::ControlFlow::Continue
                            }
                        ),
                    );
                }
            ),
        );
    }

    /// Change the stored settings and write them back.
    fn edit_settings(&self, edit: impl FnOnce(&mut UpdateSettings)) {
        let mut settings = self.imp().settings.borrow_mut();

        edit(&mut settings);
        settings.save();
    }

    /// Check, but only if the settings say one is due.
    fn check_if_due(&self) {
        if self.imp().settings.borrow().is_check_due(now()) {
            self.check(false);
        }
    }

    /// Ask the feed what the current release is.
    ///
    /// `user_initiated` says whether somebody pressed a button: an automatic
    /// check announces what it finds, and says nothing at all when it fails,
    /// because a laptop that is offline is not a problem the user needs to be
    /// told about.
    pub(crate) fn check(&self, user_initiated: bool) {
        let imp = self.imp();

        if matches!(
            self.state(),
            UpdateState::Checking | UpdateState::Downloading | UpdateState::Installing
        ) {
            return;
        }

        if !self.supported() {
            return;
        }

        self.set_state(UpdateState::Checking);

        let channel = imp.settings.borrow().effective_channel();

        spawn!(clone!(
            #[weak(rename_to = obj)]
            self,
            async move {
                let result = spawn_tokio!(async move { updates::check(channel).await })
                    .await
                    .expect("the update check task is never aborted");

                obj.edit_settings(|settings| settings.last_check = Some(now()));

                match result {
                    Ok(check) => {
                        if let Some(release) = check.available {
                            let version = release.version.clone();
                            let announce = !user_initiated
                                && !obj.imp().settings.borrow().is_skipped(&release);

                            obj.imp().release.replace(Some(release));
                            obj.imp().available_version.replace(version.clone());
                            obj.notify_available_version();
                            obj.set_state(UpdateState::Available);

                            if announce {
                                obj.emit_by_name::<()>("update-announced", &[&version]);
                            }
                        } else {
                            obj.imp().release.replace(None);
                            obj.set_state(UpdateState::UpToDate);
                        }
                    }
                    Err(check_error) => {
                        // An automatic check that could not reach the feed is
                        // an ordinary thing on a laptop, so it is a log line
                        // rather than anything the user sees; one the user
                        // asked for owes them an answer.
                        if user_initiated {
                            error!("Could not check for updates: {check_error}");
                            obj.imp().status.replace(check_error.to_user_facing());
                            obj.set_state(UpdateState::Failed);
                            obj.notify_status();
                            return;
                        }

                        warn!("Could not check for updates: {check_error}");
                        obj.set_state(UpdateState::Idle);
                    }
                }
            }
        ));
    }

    /// Download the available release and start its installer.
    ///
    /// The application quits as the last step, because that is what both
    /// platforms need: Windows cannot replace an executable that is running,
    /// and macOS has already replaced the bundle by then and wants the new
    /// one started.
    pub(crate) fn install(&self) {
        let Some(release) = self.imp().release.borrow().clone() else {
            return;
        };

        let Some(asset) = release.asset_for_current_platform().cloned() else {
            return;
        };

        if matches!(
            self.state(),
            UpdateState::Downloading | UpdateState::Installing
        ) {
            return;
        }

        self.imp().progress.set(0.0);
        self.notify_progress();
        self.set_state(UpdateState::Downloading);

        let (sender, mut receiver) = mpsc::unbounded();

        // The download runs on a tokio worker and reports from there, which
        // is no place to touch a `GObject`. The numbers come back over a
        // channel and are applied here, on the main context.
        spawn!(clone!(
            #[weak(rename_to = obj)]
            self,
            async move {
                while let Some((downloaded, total)) = receiver.next().await {
                    let fraction = if total == 0 {
                        0.0
                    } else {
                        downloaded as f64 / total as f64
                    };

                    obj.imp().progress.set(fraction);
                    obj.notify_progress();
                    obj.update_status();
                }
            }
        ));

        spawn!(clone!(
            #[weak(rename_to = obj)]
            self,
            async move {
                let progress: Arc<dyn DownloadProgress> = Arc::new(ChannelProgress { sender });
                let dir = download_dir();

                let downloaded =
                    spawn_tokio!(
                        async move { updates::download(&asset, &dir, Some(progress)).await }
                    )
                    .await
                    .expect("the download task is never aborted");

                let path = match downloaded {
                    Ok(path) => path,
                    Err(download_error) => {
                        error!("Could not download the update: {download_error}");
                        obj.imp().status.replace(download_error.to_user_facing());
                        obj.set_state(UpdateState::Failed);
                        obj.notify_status();
                        return;
                    }
                };

                obj.set_state(UpdateState::Installing);

                if let Err(install_error) = start_installer(&path) {
                    error!("Could not start the installer: {install_error}");
                    obj.imp().status.replace(gettext(
                        "The update was downloaded but could not be installed.",
                    ));
                    obj.set_state(UpdateState::Failed);
                    obj.notify_status();
                    return;
                }

                // The installer is waiting for this process to be gone.
                crate::Application::default().quit();
            }
        ));
    }

    /// Where to read what the available release changed, if there is one.
    pub(crate) fn release_notes_url(&self) -> Option<String> {
        self.imp()
            .release
            .borrow()
            .as_ref()
            .map(|release| release.notes_url.clone())
    }

    /// Stop offering the release the last check found.
    pub(crate) fn skip(&self) {
        let Some(release) = self.imp().release.borrow().clone() else {
            return;
        };

        self.edit_settings(|settings| settings.skipped_version = Some(release.version));
    }

    /// Whether automatic checks are on.
    pub(crate) fn check_automatically(&self) -> bool {
        self.imp().settings.borrow().check_automatically
    }

    /// Turn automatic checks on or off.
    pub(crate) fn set_check_automatically(&self, enabled: bool) {
        if self.check_automatically() == enabled {
            return;
        }

        self.edit_settings(|settings| settings.check_automatically = enabled);
    }

    /// The channel being followed.
    pub(crate) fn channel(&self) -> Channel {
        self.imp().settings.borrow().effective_channel()
    }

    /// Follow the given channel, and look at it straight away.
    pub(crate) fn set_channel(&self, channel: Channel) {
        if self.channel() == channel {
            return;
        }

        self.edit_settings(|settings| {
            settings.channel = Some(channel);
            // A channel the user just chose is a channel they want an answer
            // about now, not tomorrow.
            settings.last_check = None;
            settings.skipped_version = None;
        });

        self.check(true);
    }

    /// Record a new state and rewrite the status sentence for it.
    fn set_state(&self, state: UpdateState) {
        if self.state() == state {
            return;
        }

        self.imp().state.set(state);
        self.update_status();
        self.notify_state();
    }

    /// Rewrite [`Self::status`] for the current state.
    fn update_status(&self) {
        let version = self.available_version();

        let status = match self.state() {
            UpdateState::Idle if !self.supported() => gettext(
                // Translators: shown where an update button would be, in a
                // build that is managed by something else.
                "Updates for this installation arrive through your app store.",
            ),
            UpdateState::Idle => gettext("Not checked yet."),
            UpdateState::Checking => gettext("Checking…"),
            UpdateState::UpToDate => gettext("Commune is up to date."),
            UpdateState::Available => gettext_f(
                // Translators: Do NOT translate the content between '{' and '}', this is a
                // variable name. {version} is a version number, like 1.0.
                "Commune {version} is available.",
                &[("version", &version)],
            ),
            UpdateState::Downloading => gettext_f(
                // Translators: Do NOT translate the content between '{' and '}', this is a
                // variable name. {percent} is a whole number, like 40.
                "Downloading Commune {version}… {percent}%",
                &[
                    ("version", &version),
                    ("percent", &format!("{:.0}", self.progress() * 100.0)),
                ],
            ),
            UpdateState::Installing => gettext("Installing. Commune will restart."),
            // Left as whatever the failing step put there.
            UpdateState::Failed => return,
        };

        self.imp().status.replace(status);
        self.notify_status();
    }
}

impl Default for Updates {
    fn default() -> Self {
        Self::new()
    }
}

/// A [`DownloadProgress`] that forwards the numbers to the main context.
struct ChannelProgress {
    /// Where the numbers go.
    sender: mpsc::UnboundedSender<(u64, u64)>,
}

impl DownloadProgress for ChannelProgress {
    fn progress(&self, downloaded: u64, total: u64) {
        // A closed channel means the receiving task is gone, which is not
        // worth interrupting a download for.
        let _ = self.sender.unbounded_send((downloaded, total));
    }
}

/// The current time as a Unix timestamp in seconds.
fn now() -> u64 {
    u64::try_from(glib::DateTime::now_utc().map_or(0, |now| now.to_unix())).unwrap_or_default()
}

/// Whether this installation can replace itself.
fn is_supported() -> bool {
    if !updates::is_supported() {
        return false;
    }

    #[cfg(target_os = "windows")]
    {
        crate::utils::windows_update::is_installed()
    }
    #[cfg(target_os = "macos")]
    {
        crate::utils::macos_update::is_installed()
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        false
    }
}

/// Hand the downloaded artifact to the platform.
///
/// The name is checked first. The manifest is signed, so a `.txt` under the
/// `windows-x86_64-msi` key is a mistake rather than an attack — but it is a
/// mistake that would otherwise end with `msiexec` being pointed at
/// something that is not a package, and saying so plainly beats whatever the
/// installer would say about it.
#[allow(unused_variables, reason = "no platform to install on, on Linux")]
fn start_installer(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        if !crate::utils::windows_update::is_installer(path) {
            return Err(std::io::Error::other(
                "the release does not offer a Windows installer package",
            ));
        }

        crate::utils::windows_update::install(path)
    }
    #[cfg(target_os = "macos")]
    {
        if !crate::utils::macos_update::is_installer(path) {
            return Err(std::io::Error::other(
                "the release does not offer a macOS application archive",
            ));
        }

        crate::utils::macos_update::install(path)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        Err(std::io::Error::other(
            "this platform has no installer to hand the update to",
        ))
    }
}
