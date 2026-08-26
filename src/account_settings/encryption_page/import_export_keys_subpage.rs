use std::path::{Path, PathBuf};

use adw::{prelude::*, subclass::prelude::*};
use gettextrs::{gettext, ngettext};
use gtk::{gio, glib};
use matrix_sdk::encryption::{KeyExportError, RoomKeyImportError};
use tracing::{debug, error, warn};

use crate::{
    RUNTIME,
    components::LoadingButtonRow,
    session::Session,
    spawn_tokio, toast,
    utils::{File, local_path, save_data_to_tmp_file, tmp_file_path},
};

#[derive(Debug, Default, Hash, Eq, PartialEq, Clone, Copy, glib::Enum)]
#[repr(u32)]
#[enum_type(name = "ImportExportKeysSubpageMode")]
pub enum ImportExportKeysSubpageMode {
    #[default]
    Export = 0,
    Import = 1,
}

mod imp {
    use std::{
        cell::{Cell, RefCell},
        marker::PhantomData,
    };

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/account_settings/encryption_page/import_export_keys_subpage.ui"
    )]
    #[properties(wrapper_type = super::ImportExportKeysSubpage)]
    pub struct ImportExportKeysSubpage {
        #[template_child]
        description: TemplateChild<gtk::Label>,
        #[template_child]
        instructions: TemplateChild<gtk::Label>,
        #[template_child]
        passphrase: TemplateChild<adw::PasswordEntryRow>,
        #[template_child]
        confirm_passphrase_box: TemplateChild<gtk::Box>,
        #[template_child]
        confirm_passphrase: TemplateChild<adw::PasswordEntryRow>,
        #[template_child]
        confirm_passphrase_error_revealer: TemplateChild<gtk::Revealer>,
        #[template_child]
        confirm_passphrase_error: TemplateChild<gtk::Label>,
        #[template_child]
        file_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        file_button: TemplateChild<gtk::Button>,
        #[template_child]
        proceed_button: TemplateChild<LoadingButtonRow>,
        /// The current session.
        #[property(get, set, nullable)]
        session: glib::WeakRef<Session>,
        /// The path of the file for the encryption keys.
        #[property(get)]
        file_path: RefCell<Option<gio::File>>,
        /// The path of the file for the encryption keys, as a string.
        #[property(get = Self::file_path_string)]
        file_path_string: PhantomData<Option<String>>,
        /// The export/import mode of the subpage.
        #[property(get, set = Self::set_mode, explicit_notify, builder(ImportExportKeysSubpageMode::default()))]
        mode: Cell<ImportExportKeysSubpageMode>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ImportExportKeysSubpage {
        const NAME: &'static str = "ImportExportKeysSubpage";
        type Type = super::ImportExportKeysSubpage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            Self::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for ImportExportKeysSubpage {
        fn constructed(&self) {
            self.parent_constructed();
            self.update_for_mode();
        }
    }

    impl WidgetImpl for ImportExportKeysSubpage {}
    impl NavigationPageImpl for ImportExportKeysSubpage {}

    #[gtk::template_callbacks]
    impl ImportExportKeysSubpage {
        /// Set the export/import mode of the subpage.
        fn set_mode(&self, mode: ImportExportKeysSubpageMode) {
            if self.mode.get() == mode {
                return;
            }

            self.mode.set(mode);
            self.update_for_mode();
            self.clear();
            self.obj().notify_mode();
        }

        /// The path to export the keys to, as a string.
        fn file_path_string(&self) -> Option<String> {
            self.file_path
                .borrow()
                .as_ref()
                .and_then(gio::File::path)
                .map(|path| path.to_string_lossy().to_string())
        }

        /// Whether the subpage is in export mode.
        fn is_export(&self) -> bool {
            self.mode.get() == ImportExportKeysSubpageMode::Export
        }

        /// Set the path of the file for the encryption keys.
        fn set_file_path(&self, path: Option<gio::File>) {
            if *self.file_path.borrow() == path {
                return;
            }

            self.file_path.replace(path);
            self.update_button();

            let obj = self.obj();
            obj.notify_file_path();
            obj.notify_file_path_string();
        }

        /// Reset the subpage's fields.
        fn clear(&self) {
            self.set_file_path(None);
            self.passphrase.set_text("");
            self.confirm_passphrase.set_text("");
        }

        /// Update the UI for the current mode.
        fn update_for_mode(&self) {
            let obj = self.obj();

            if self.is_export() {
                // Translators: 'Room encryption keys' are encryption keys for all rooms.
                obj.set_title(&gettext("Export Room Encryption Keys"));
                self.description.set_label(&gettext(
                        // Translators: 'Room encryption keys' are encryption keys for all rooms.
                        "Exporting your room encryption keys allows you to make a backup to be able to decrypt your messages in end-to-end encrypted rooms on another device or with another Matrix client.",
                    ));
                self.instructions.set_label(&gettext(
                        "The backup must be stored in a safe place and must be protected with a strong passphrase that will be used to encrypt the data.",
                    ));
                self.confirm_passphrase_box.set_visible(true);
                self.proceed_button.set_title(&gettext("Export Keys"));
            } else {
                // Translators: 'Room encryption keys' are encryption keys for all rooms.
                obj.set_title(&gettext("Import Room Encryption Keys"));
                self.description.set_label(&gettext(
                        // Translators: 'Room encryption keys' are encryption keys for all rooms.
                        "Importing your room encryption keys allows you to decrypt your messages in end-to-end encrypted rooms with a previous backup from a Matrix client.",
                    ));
                self.instructions.set_label(&gettext(
                    "Enter the passphrase provided when the backup file was created.",
                ));
                self.confirm_passphrase_box.set_visible(false);
                self.proceed_button.set_title(&gettext("Import Keys"));
            }

            self.update_button();
        }

        /// Open a dialog to choose the file.
        #[template_callback]
        async fn choose_file(&self) {
            let is_export = self.is_export();

            let dialog = gtk::FileDialog::builder()
                .modal(true)
                .accept_label(gettext("Choose"))
                .build();

            if let Some(file) = self.file_path.borrow().as_ref() {
                dialog.set_initial_file(Some(file));
            } else if is_export {
                // Translators: Do not translate "commune" as it is the application
                // name.
                dialog
                    .set_initial_name(Some(&format!("{}.txt", gettext("commune-encryption-keys"))));
            }

            let obj = self.obj();
            let parent_window = obj.root().and_downcast::<gtk::Window>();
            let res = if is_export {
                dialog.set_title(&gettext("Save Encryption Keys To…"));
                dialog.save_future(parent_window.as_ref()).await
            } else {
                dialog.set_title(&gettext("Import Encryption Keys From…"));
                dialog.open_future(parent_window.as_ref()).await
            };

            match res {
                Ok(file) => {
                    self.set_file_path(Some(file));
                }
                Err(error) => {
                    if error.matches(gtk::DialogError::Dismissed) {
                        debug!("File dialog dismissed by user");
                    } else {
                        error!("Could not access file: {error:?}");
                        toast!(obj, gettext("Could not access file"));
                    }
                }
            }
        }

        /// Validate the passphrase confirmation.
        #[template_callback]
        fn validate_passphrase_confirmation(&self) {
            let entry = &self.confirm_passphrase;
            let revealer = &self.confirm_passphrase_error_revealer;
            let label = &self.confirm_passphrase_error;
            let passphrase = self.passphrase.text();
            let confirmation = entry.text();

            if !self.is_export() || confirmation.is_empty() {
                revealer.set_reveal_child(false);
                entry.remove_css_class("success");
                entry.remove_css_class("warning");

                self.update_button();
                return;
            }

            if passphrase == confirmation {
                revealer.set_reveal_child(false);
                entry.add_css_class("success");
                entry.remove_css_class("warning");
            } else {
                label.set_label(&gettext("Passphrases do not match"));
                revealer.set_reveal_child(true);
                entry.remove_css_class("success");
                entry.add_css_class("warning");
            }

            self.update_button();
        }

        /// Update the state of the button.
        fn update_button(&self) {
            self.proceed_button.set_sensitive(self.can_proceed());
        }

        /// Whether we can proceed to the import/export.
        fn can_proceed(&self) -> bool {
            let has_file = self.file_path.borrow().is_some();
            let passphrase = self.passphrase.text();

            let mut can_proceed = has_file && !passphrase.is_empty();

            if self.is_export() {
                let confirmation = self.confirm_passphrase.text();
                can_proceed &= passphrase == confirmation;
            }

            can_proceed
        }

        /// Set whether the import/export is in progress.
        fn set_proceeding(&self, in_progress: bool) {
            self.proceed_button.set_is_loading(in_progress);
            self.file_button.set_sensitive(!in_progress);
            self.passphrase.set_sensitive(!in_progress);
            self.confirm_passphrase.set_sensitive(!in_progress);
        }

        /// Resolve a local path to hand to `matrix-sdk` for the import/export,
        /// showing an error toast and returning `None` if that is not possible.
        ///
        /// Both directions of this dialog hand `matrix-sdk` a `PathBuf`, and on
        /// Android the chosen file may have none: `local_path` cannot tell a
        /// save destination apart from a `content://` file, since neither
        /// exists yet. So export always writes through a temporary file of its
        /// own, which the caller reads back afterwards to copy into the file
        /// the user chose. Import reuses the chosen file's real path when it
        /// has one, and copies it to a temporary file otherwise -- the
        /// composer's problem again, see `doc/android-attachments-plan.md`.
        ///
        /// The two guards in the result are kept alive by the caller until the
        /// import/export using the path is done: dropping either deletes the
        /// file it names.
        async fn resolve_source_path(
            &self,
            chosen_file: &gio::File,
            is_export: bool,
        ) -> Option<(PathBuf, Option<tempfile::TempPath>, Option<File>)> {
            let obj = self.obj();

            if is_export {
                match tmp_file_path().await {
                    Ok(temp_path) => {
                        let path = temp_path.to_path_buf();
                        Some((path, Some(temp_path), None))
                    }
                    Err(error) => {
                        error!("Could not create a temporary file: {error}");
                        toast!(obj, gettext("Could not export the keys"));
                        None
                    }
                }
            } else if let Some(path) = local_path(chosen_file) {
                Some((path, None, None))
            } else {
                let data = match chosen_file.load_contents_future().await {
                    Ok((data, _)) => data.to_vec(),
                    Err(error) => {
                        warn!("Could not read file: {error}");
                        toast!(obj, gettext("Could not import the keys"));
                        return None;
                    }
                };

                match save_data_to_tmp_file(data).await {
                    Ok(file) => {
                        let path = file.as_gfile().path().expect("temporary file has a path");
                        Some((path, None, Some(file)))
                    }
                    Err(error) => {
                        warn!("Could not copy the file to a temporary file: {error}");
                        toast!(obj, gettext("Could not import the keys"));
                        None
                    }
                }
            }
        }

        /// Copy the just-exported keys from the temporary file at `temp_path`
        /// into `chosen_file`, showing an error toast on failure.
        ///
        /// Returns whether it succeeded.
        async fn finish_export(&self, chosen_file: &gio::File, temp_path: &Path) -> bool {
            let obj = self.obj();

            // `tokio::fs::read` needs an ambient runtime, which this template
            // callback -- running on the GLib main context, not inside
            // `spawn_tokio!` -- does not have. `RUNTIME.spawn_blocking` carries
            // its own runtime handle, so it works from here the same way
            // `save_data_to_tmp_file`/`tmp_file_path` already do.
            let temp_path = temp_path.to_path_buf();
            let data = match RUNTIME
                .spawn_blocking(move || std::fs::read(temp_path))
                .await
                .expect("task was not aborted")
            {
                Ok(data) => data,
                Err(error) => {
                    error!("Could not read the exported keys: {error}");
                    toast!(obj, gettext("Could not export the keys"));
                    return false;
                }
            };

            if let Err(error) = chosen_file.replace_contents(
                &data,
                None,
                false,
                gio::FileCreateFlags::REPLACE_DESTINATION,
                gio::Cancellable::NONE,
            ) {
                error!("Could not save the keys: {error}");
                toast!(obj, gettext("Could not export the keys"));
                return false;
            }

            true
        }

        /// Proceed to the import/export.
        #[template_callback]
        async fn proceed(&self) {
            if !self.can_proceed() {
                return;
            }

            let Some(chosen_file) = self.file_path.borrow().clone() else {
                return;
            };
            let Some(session) = self.session.upgrade() else {
                return;
            };

            let obj = self.obj();
            let passphrase = self.passphrase.text();
            let is_export = self.is_export();

            self.set_proceeding(true);

            // `temp_path`/`_temp_file` are held for the rest of this function:
            // dropping either deletes the file it names, and the path is still
            // needed below, first by `export_room_keys`/`import_room_keys`, and
            // for export, again by `finish_export` once the temporary file has
            // been written. `_temp_file` itself is never read again.
            let Some((path, temp_path, _temp_file)) =
                self.resolve_source_path(&chosen_file, is_export).await
            else {
                self.set_proceeding(false);
                return;
            };

            let encryption = session.client().encryption();

            let handle = spawn_tokio!(async move {
                if is_export {
                    encryption
                        .export_room_keys(path, passphrase.as_str(), |_| true)
                        .await
                        .map(|()| 0usize)
                        .map_err::<Box<dyn std::error::Error + Send>, _>(|error| Box::new(error))
                } else {
                    encryption
                        .import_room_keys(path, passphrase.as_str())
                        .await
                        .map(|res| res.imported_count)
                        .map_err::<Box<dyn std::error::Error + Send>, _>(|error| Box::new(error))
                }
            });

            match handle.await.expect("task was not aborted") {
                Ok(nb) => {
                    if is_export {
                        let temp_path = temp_path
                            .as_deref()
                            .expect("export always writes to a temporary file");

                        if !self.finish_export(&chosen_file, temp_path).await {
                            self.set_proceeding(false);
                            return;
                        }

                        toast!(obj, gettext("Room encryption keys exported successfully"));
                    } else {
                        let n = nb.try_into().unwrap_or(u32::MAX);
                        toast!(
                            obj,
                            ngettext(
                                "Imported 1 room encryption key",
                                "Imported {n} room encryption keys",
                                n,
                            ),
                            n,
                        );
                    }

                    self.clear();
                    let _ = obj.activate_action("account-settings.close-subpage", None);
                }
                Err(error) => {
                    if is_export {
                        error!("Could not export the keys: {error}");
                        toast!(obj, gettext("Could not export the keys"));
                    } else if error
                        .downcast_ref::<RoomKeyImportError>()
                        .is_some_and(|error| {
                            matches!(
                                error,
                                RoomKeyImportError::Export(KeyExportError::InvalidMac)
                            )
                        })
                    {
                        toast!(
                            obj,
                            gettext(
                                "The passphrase doesn't match the one used to export the keys."
                            ),
                        );
                    } else {
                        error!("Could not import the keys: {error}");
                        toast!(obj, gettext("Could not import the keys"));
                    }
                }
            }

            self.set_proceeding(false);
        }
    }
}

glib::wrapper! {
    /// Subpage to import or export room encryption keys for backup.
    pub struct ImportExportKeysSubpage(ObjectSubclass<imp::ImportExportKeysSubpage>)
        @extends gtk::Widget, adw::NavigationPage,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ImportExportKeysSubpage {
    pub fn new(session: &Session, mode: ImportExportKeysSubpageMode) -> Self {
        glib::Object::builder()
            .property("session", session)
            .property("mode", mode)
            .build()
    }
}
