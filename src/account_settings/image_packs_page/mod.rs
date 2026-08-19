use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{glib, glib::clone};
use ruma::RoomId;
use tracing::error;

use crate::{
    components::{ImagePackEditor, SwitchLoadingRow},
    gettext_f, ngettext_f,
    prelude::*,
    session::{EnabledPack, ImagePack, ImagePackSource, ImagePacks, Session},
    spawn, toast,
};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/account_settings/image_packs_page/mod.ui")]
    #[properties(wrapper_type = super::ImagePacksPage)]
    pub struct ImagePacksPage {
        #[template_child]
        personal_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        enabled_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        enabled_placeholder_row: TemplateChild<adw::ActionRow>,
        /// The current session.
        #[property(get, set = Self::set_session, explicit_notify, nullable)]
        session: glib::WeakRef<Session>,
        /// The rows of the enabled packs, to be able to remove them.
        rows: RefCell<Vec<SwitchLoadingRow>>,
        image_packs_handler: RefCell<Option<(ImagePacks, glib::SignalHandlerId)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ImagePacksPage {
        const NAME: &'static str = "ImagePacksPage";
        type Type = super::ImagePacksPage;
        type ParentType = adw::PreferencesPage;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for ImagePacksPage {}

    impl WidgetImpl for ImagePacksPage {}
    impl PreferencesPageImpl for ImagePacksPage {}

    #[gtk::template_callbacks]
    impl ImagePacksPage {
        /// Set the current session.
        fn set_session(&self, session: Option<&Session>) {
            if self.session.upgrade().as_ref() == session {
                return;
            }

            if let Some((image_packs, handler)) = self.image_packs_handler.take() {
                image_packs.disconnect(handler);
            }

            self.session.set(session);

            if let Some(image_packs) = session.map(Session::image_packs) {
                let handler = image_packs.connect_changed(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.load();
                    }
                ));
                self.image_packs_handler
                    .replace(Some((image_packs, handler)));
            }

            self.load();
            self.obj().notify_session();
        }

        /// Load the packs of the session.
        fn load(&self) {
            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    imp.load_personal_pack();
                    imp.load_enabled_packs().await;
                }
            ));
        }

        /// Present the personal pack of the user.
        fn load_personal_pack(&self) {
            let Some(session) = self.session.upgrade() else {
                return;
            };

            let Some(pack) = session.image_packs().user_pack() else {
                self.personal_row.set_title(&gettext("No Personal Pack"));
                self.personal_row
                    .set_subtitle(&gettext("Create one to use your own images in every room"));
                return;
            };

            self.personal_row
                .set_title(&glib::markup_escape_text(&pack.display_name()));
            self.personal_row
                .set_subtitle(&glib::markup_escape_text(&image_count(&pack)));
        }

        /// Open the editor for the personal pack of the user.
        #[template_callback]
        fn edit_personal_pack(&self) {
            let Some(session) = self.session.upgrade() else {
                return;
            };

            let editor = match session.image_packs().user_pack() {
                Some(pack) => ImagePackEditor::edit(&session, &pack),
                None => ImagePackEditor::create(&session, ImagePackSource::User),
            };

            let Some(dialog) = self
                .obj()
                .ancestor(adw::PreferencesDialog::static_type())
                .and_downcast::<adw::PreferencesDialog>()
            else {
                error!("Could not find the dialog of the image packs page");
                return;
            };

            dialog.push_subpage(&editor);
        }

        /// Present the packs that are enabled globally.
        async fn load_enabled_packs(&self) {
            let Some(session) = self.session.upgrade() else {
                return;
            };

            let packs = session.image_packs().enabled_packs().await;

            for row in self.rows.take() {
                self.enabled_group.remove(&row);
            }

            self.enabled_placeholder_row.set_visible(packs.is_empty());

            let mut rows = Vec::with_capacity(packs.len());
            for pack in packs {
                let row = self.build_row(&pack);
                self.enabled_group.add(&row);
                rows.push(row);
            }
            self.rows.replace(rows);
        }

        /// Build the row for the given enabled pack.
        fn build_row(&self, pack: &EnabledPack) -> SwitchLoadingRow {
            let row = SwitchLoadingRow::new();
            row.set_is_active(true);

            let (room_id, state_key) = match pack {
                EnabledPack::Available(pack) => {
                    let ImagePackSource::Room {
                        room, state_key, ..
                    } = pack.source()
                    else {
                        unreachable!("an enabled pack comes from a room");
                    };

                    row.set_title(&glib::markup_escape_text(&pack.display_name()));
                    row.set_subtitle(&glib::markup_escape_text(&gettext_f(
                        // Translators: Do NOT translate the content between '{' and '}',
                        // this is a variable name.
                        "{count}, from {room}",
                        &[
                            ("count", &image_count(pack)),
                            ("room", &room.display_name()),
                        ],
                    )));

                    (room.room_id().to_owned(), state_key.clone())
                }
                EnabledPack::Unavailable { room_id, state_key } => {
                    row.set_title(&glib::markup_escape_text(state_key));
                    row.set_subtitle(&glib::markup_escape_text(&gettext_f(
                        // Translators: Do NOT translate the content between '{' and '}',
                        // this is a variable name.
                        "You are not in {room} anymore, so this pack cannot be used",
                        &[("room", room_id.as_str())],
                    )));

                    (room_id.clone(), state_key.clone())
                }
            };

            row.connect_is_active_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |row| {
                    let room_id = room_id.clone();
                    let state_key = state_key.clone();

                    spawn!(clone!(
                        #[weak]
                        imp,
                        #[weak]
                        row,
                        async move {
                            imp.toggle_pack(&room_id, &state_key, &row).await;
                        }
                    ));
                }
            ));

            row
        }

        /// Enable or disable the given pack globally, following its row.
        async fn toggle_pack(&self, room_id: &RoomId, state_key: &str, row: &SwitchLoadingRow) {
            let Some(session) = self.session.upgrade() else {
                return;
            };

            let enabled = row.is_active();
            let image_packs = session.image_packs();

            if image_packs.is_pack_enabled(room_id, state_key) == enabled {
                return;
            }

            row.set_is_loading(true);
            let result = image_packs
                .set_pack_enabled(room_id, state_key, enabled)
                .await;
            row.set_is_loading(false);

            if result.is_err() {
                error!("Could not change whether an image pack is enabled");
                toast!(
                    self.obj(),
                    gettext("Could not change whether the pack is used in every room")
                );

                // Put the switch back where it was.
                row.set_is_active(!enabled);
            }
        }
    }
}

/// The number of images of the given pack, as text.
fn image_count(pack: &ImagePack) -> String {
    let count = pack.images().n_items();

    ngettext_f(
        // Translators: Do NOT translate the content between '{' and '}', this
        // is a variable name.
        "{count} image",
        "{count} images",
        count,
        &[("count", &count.to_string())],
    )
}

glib::wrapper! {
    /// A page to manage the image packs of the user.
    pub struct ImagePacksPage(ObjectSubclass<imp::ImagePacksPage>)
        @extends gtk::Widget, adw::PreferencesPage,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ImagePacksPage {
    /// Create a new `ImagePacksPage`.
    pub fn new(session: &Session) -> Self {
        glib::Object::builder().property("session", session).build()
    }
}
