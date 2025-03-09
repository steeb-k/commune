use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{gio, glib, CompositeTemplate};
use matrix_sdk::encryption::{KeyExportError, RoomKeyImportError};
use tracing::{debug, error};

use crate::{
    components::LoadingButtonRow, ngettext_f, session::model::Session, spawn_tokio, toast,
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

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/account_settings/security_page/import_export_keys_subpage.ui"
    )]
    #[properties(wrapper_type = super::ImportExportKeysSubpage)]
    pub struct ImportExportKeysSubpage {
        /// The current session.
        #[property(get, set, nullable)]
        pub session: glib::WeakRef<Session>,
        #[template_child]
        pub description: TemplateChild<gtk::Label>,
        #[template_child]
        pub instructions: TemplateChild<gtk::Label>,
        #[template_child]
        pub passphrase: TemplateChild<adw::PasswordEntryRow>,
        #[template_child]
        pub confirm_passphrase_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub confirm_passphrase: TemplateChild<adw::PasswordEntryRow>,
        #[template_child]
        pub confirm_passphrase_error_revealer: TemplateChild<gtk::Revealer>,
        #[template_child]
        pub confirm_passphrase_error: TemplateChild<gtk::Label>,
        #[template_child]
        pub file_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub file_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub proceed_button: TemplateChild<LoadingButtonRow>,
        /// The path of the file for the encryption keys.
        #[property(get)]
        pub file_path: RefCell<Option<gio::File>>,
        /// The path of the file for the encryption keys, as a string.
        #[property(get = Self::file_path_string)]
        pub file_path_string: PhantomData<Option<String>>,
        /// The export/import mode of the subpage.
        #[property(get, set = Self::set_mode, explicit_notify, builder(ImportExportKeysSubpageMode::default()))]
        pub mode: Cell<ImportExportKeysSubpageMode>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ImportExportKeysSubpage {
        const NAME: &'static str = "ImportExportKeysSubpage";
        type Type = super::ImportExportKeysSubpage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
            Self::Type::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for ImportExportKeysSubpage {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().update_for_mode();
        }
    }

    impl WidgetImpl for ImportExportKeysSubpage {}
    impl NavigationPageImpl for ImportExportKeysSubpage {}

    impl ImportExportKeysSubpage {
        /// Set the export/import mode of the subpage.
        fn set_mode(&self, mode: ImportExportKeysSubpageMode) {
            if self.mode.get() == mode {
                return;
            }
            let obj = self.obj();

            self.mode.set(mode);
            obj.update_for_mode();
            obj.clear();
            obj.notify_mode();
        }

        /// The path to export the keys to, as a string.
        fn file_path_string(&self) -> Option<String> {
            self.file_path
                .borrow()
                .as_ref()
                .and_then(gio::File::path)
                .map(|path| path.to_string_lossy().to_string())
        }
    }
}

glib::wrapper! {
    /// Subpage to export room encryption keys for backup.
    pub struct ImportExportKeysSubpage(ObjectSubclass<imp::ImportExportKeysSubpage>)
        @extends gtk::Widget, adw::NavigationPage, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl ImportExportKeysSubpage {
    pub fn new(session: &Session, mode: ImportExportKeysSubpageMode) -> Self {
        glib::Object::builder()
            .property("session", session)
            .property("mode", mode)
            .build()
    }

    /// Whether the subpage is in export mode.
    fn is_export(&self) -> bool {
        self.mode() == ImportExportKeysSubpageMode::Export
    }

    /// Set the path of the file for the encryption keys.
    fn set_file_path(&self, path: Option<gio::File>) {
        let imp = self.imp();
        if *imp.file_path.borrow() == path {
            return;
        }

        imp.file_path.replace(path);
        self.update_button();
        self.notify_file_path();
        self.notify_file_path_string();
    }

    /// Reset the subpage's fields.
    fn clear(&self) {
        let imp = self.imp();

        self.set_file_path(None);
        imp.passphrase.set_text("");
        imp.confirm_passphrase.set_text("");
    }

    /// Update the UI for the current mode.
    fn update_for_mode(&self) {
        let imp = self.imp();

        if self.is_export() {
            // Translators: 'Room encryption keys' are encryption keys for all rooms.
            self.set_title(&gettext("Export Room Encryption Keys"));
            imp.description.set_label(&gettext(
                // Translators: 'Room encryption keys' are encryption keys for all rooms.
                "Exporting your room encryption keys allows you to make a backup to be able to decrypt your messages in end-to-end encrypted rooms on another device or with another Matrix client.",
            ));
            imp.instructions.set_label(&gettext(
                "The backup must be stored in a safe place and must be protected with a strong passphrase that will be used to encrypt the data.",
            ));
            imp.confirm_passphrase_box.set_visible(true);
            imp.proceed_button.set_title(&gettext("Export Keys"));
        } else {
            // Translators: 'Room encryption keys' are encryption keys for all rooms.
            self.set_title(&gettext("Import Room Encryption Keys"));
            imp.description.set_label(&gettext(
                // Translators: 'Room encryption keys' are encryption keys for all rooms.
                "Importing your room encryption keys allows you to decrypt your messages in end-to-end encrypted rooms with a previous backup from a Matrix client.",
            ));
            imp.instructions.set_label(&gettext(
                "Enter the passphrase provided when the backup file was created.",
            ));
            imp.confirm_passphrase_box.set_visible(false);
            imp.proceed_button.set_title(&gettext("Import Keys"));
        }

        self.update_button();
    }

    /// Open a dialog to choose the file.
    #[template_callback]
    async fn choose_file(&self) {
        let is_export = self.mode() == ImportExportKeysSubpageMode::Export;

        let dialog = gtk::FileDialog::builder()
            .modal(true)
            .accept_label(gettext("Choose"))
            .build();

        if let Some(file) = self.file_path() {
            dialog.set_initial_file(Some(&file));
        } else if is_export {
            // Translators: Do no translate "fractal" as it is the application
            // name.
            dialog.set_initial_name(Some(&format!("{}.txt", gettext("fractal-encryption-keys"))));
        }

        let parent_window = self.root().and_downcast::<gtk::Window>();
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
                    toast!(self, gettext("Could not access file"));
                }
            }
        }
    }

    /// Validate the passphrase confirmation.
    #[template_callback]
    fn validate_passphrase_confirmation(&self) {
        let imp = self.imp();
        let entry = &imp.confirm_passphrase;
        let revealer = &imp.confirm_passphrase_error_revealer;
        let label = &imp.confirm_passphrase_error;
        let passphrase = imp.passphrase.text();
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
        self.imp().proceed_button.set_sensitive(self.can_proceed());
    }

    /// Whether we can proceed to the import/export.
    fn can_proceed(&self) -> bool {
        let imp = self.imp();
        let file_path = imp.file_path.borrow();
        let passphrase = imp.passphrase.text();

        let mut res = file_path
            .as_ref()
            .filter(|file| file.path().is_some())
            .is_some()
            && !passphrase.is_empty();

        if self.is_export() {
            let confirmation = imp.confirm_passphrase.text();
            res = res && passphrase == confirmation;
        }

        res
    }

    /// Proceed to the import/export.
    #[template_callback]
    async fn proceed(&self) {
        if !self.can_proceed() {
            return;
        }

        let imp = self.imp();
        let file_path = self.file_path().and_then(|file| file.path()).unwrap();
        let passphrase = imp.passphrase.text();
        let is_export = self.is_export();

        imp.proceed_button.set_is_loading(true);
        imp.file_button.set_sensitive(false);
        imp.passphrase.set_sensitive(false);
        imp.confirm_passphrase.set_sensitive(false);

        let encryption = self.session().unwrap().client().encryption();

        let handle = spawn_tokio!(async move {
            if is_export {
                encryption
                    .export_room_keys(file_path, passphrase.as_str(), |_| true)
                    .await
                    .map(|()| 0usize)
                    .map_err::<Box<dyn std::error::Error + Send>, _>(|error| Box::new(error))
            } else {
                encryption
                    .import_room_keys(file_path, passphrase.as_str())
                    .await
                    .map(|res| res.imported_count)
                    .map_err::<Box<dyn std::error::Error + Send>, _>(|error| Box::new(error))
            }
        });

        match handle.await.unwrap() {
            Ok(nb) => {
                if is_export {
                    toast!(self, gettext("Room encryption keys exported successfully"));
                } else {
                    let n = nb.try_into().unwrap_or(u32::MAX);
                    toast!(
                        self,
                        ngettext_f(
                            "Imported 1 room encryption key",
                            "Imported {n} room encryption keys",
                            n,
                            &[("n", &n.to_string())]
                        )
                    );
                }
                self.clear();
                self.activate_action("account-settings.close-subpage", None)
                    .unwrap();
            }
            Err(err) => {
                if is_export {
                    error!("Could not export the keys: {err:?}");
                    toast!(self, gettext("Could not export the keys"));
                } else if err
                    .downcast_ref::<RoomKeyImportError>()
                    .filter(|err| {
                        matches!(err, RoomKeyImportError::Export(KeyExportError::InvalidMac))
                    })
                    .is_some()
                {
                    toast!(
                        self,
                        gettext("The passphrase doesn't match the one used to export the keys.")
                    );
                } else {
                    error!("Could not import the keys: {err:?}");
                    toast!(self, gettext("Could not import the keys"));
                }
            }
        }
        imp.proceed_button.set_is_loading(false);
        imp.file_button.set_sensitive(true);
        imp.passphrase.set_sensitive(true);
        imp.confirm_passphrase.set_sensitive(true);
    }
}
