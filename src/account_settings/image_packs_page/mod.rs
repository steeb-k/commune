use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{gio, glib, glib::clone};
use ruma::RoomId;
use tracing::error;

use crate::{
    components::{
        ImagePackEditor, LoadingButton, SwitchLoadingRow, confirm_delete_image_pack_dialog,
    },
    gettext_f, ngettext_f,
    prelude::*,
    session::{ImagePack, ImagePackSource, ImagePacks, RoomPackKind, Session},
    spawn, toast,
};

/// What a menu item names: the room a pack lives in, and its state key there.
type PackId = (String, String);

/// A menu item activating the given action on the given pack.
fn menu_item(label: &str, action: &str, id: &PackId) -> gio::MenuItem {
    let item = gio::MenuItem::new(Some(label), None);
    item.set_action_and_target_value(Some(action), Some(&id.to_variant()));
    item
}

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/account_settings/image_packs_page/mod.ui")]
    #[properties(wrapper_type = super::ImagePacksPage)]
    pub struct ImagePacksPage {
        #[template_child]
        packs_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        placeholder_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        create_button: TemplateChild<LoadingButton>,
        #[template_child]
        unavailable_group: TemplateChild<adw::PreferencesGroup>,
        /// The current session.
        #[property(get, set = Self::set_session, explicit_notify, nullable)]
        session: glib::WeakRef<Session>,
        /// The rows that are presented, to be able to remove them.
        rows: RefCell<Vec<(adw::PreferencesGroup, SwitchLoadingRow)>>,
        /// The packs that are presented, to be able to act on one from its
        /// menu.
        packs: RefCell<Vec<ImagePack>>,
        image_packs_handler: RefCell<Option<(ImagePacks, glib::SignalHandlerId)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ImagePacksPage {
        const NAME: &'static str = "ImagePacksPage";
        type Type = super::ImagePacksPage;
        type ParentType = adw::PreferencesPage;

        fn class_init(klass: &mut Self::Class) {
            LoadingButton::ensure_type();

            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);

            // The menu of a row names its pack by where it is, which is what
            // identifies it, rather than by its position in the list.
            klass.install_action(
                "image-packs.edit",
                Some(&PackId::static_variant_type()),
                |obj, _, variant| {
                    if let Some(id) = variant.and_then(glib::Variant::get::<PackId>) {
                        obj.imp().edit_pack(&id);
                    }
                },
            );
            klass.install_action_async(
                "image-packs.delete",
                Some(&PackId::static_variant_type()),
                |obj, _, variant: Option<glib::Variant>| async move {
                    if let Some(id) = variant.and_then(|v| v.get::<PackId>()) {
                        obj.imp().delete_pack(&id).await;
                    }
                },
            );
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
                    imp.load_packs().await;
                }
            ));
        }

        /// Present every pack that the user has, and the ones that they cannot
        /// reach anymore.
        async fn load_packs(&self) {
            let Some(session) = self.session.upgrade() else {
                return;
            };
            let image_packs = session.image_packs();

            for (group, row) in self.rows.take() {
                group.remove(&row);
            }

            let packs = image_packs.all_packs().await;
            self.placeholder_row.set_visible(packs.is_empty());
            self.packs.replace(packs.clone());

            let mut rows = Vec::new();
            for pack in &packs {
                let row = self.build_row(pack);
                self.packs_group.add(&row);
                rows.push((self.packs_group.clone(), row));
            }

            // A pack that is used everywhere but whose room the user has left
            // cannot be loaded. The specification asks clients to handle that,
            // and the only thing left to do with it is to stop using it.
            let unavailable = image_packs.unavailable_packs().await;
            self.unavailable_group.set_visible(!unavailable.is_empty());

            for pack in unavailable {
                let (room_id, state_key) = (pack.room_id, pack.state_key);
                let row = SwitchLoadingRow::new();
                row.set_is_active(true);
                row.set_title(&glib::markup_escape_text(&state_key));
                row.set_subtitle(&glib::markup_escape_text(&gettext_f(
                    // Translators: Do NOT translate the content between '{' and '}',
                    // this is a variable name.
                    "You are not in {room} anymore, so this pack cannot be used",
                    &[("room", room_id.as_str())],
                )));

                self.connect_switch(&row, room_id, state_key, None);

                self.unavailable_group.add(&row);
                rows.push((self.unavailable_group.clone(), row));
            }

            self.rows.replace(rows);
        }

        /// Build the row for the given pack.
        fn build_row(&self, pack: &ImagePack) -> SwitchLoadingRow {
            let source = pack.source();
            let image_count = pack.images().n_items();

            let mut subtitle = ngettext_f(
                // Translators: Do NOT translate the content between '{' and '}',
                // this is a variable name.
                "{count} image",
                "{count} images",
                image_count,
                &[("count", &image_count.to_string())],
            );

            subtitle.push_str(" · ");
            subtitle.push_str(&gettext_f(
                // Translators: Do NOT translate the content between '{' and '}',
                // this is a variable name.
                "in {room}",
                &[("room", &source.room.display_name())],
            ));

            if let Some(attribution) = pack.attribution() {
                subtitle.push_str(" · ");
                subtitle.push_str(attribution);
            }

            let row = SwitchLoadingRow::new();
            row.set_title(&glib::markup_escape_text(&pack.display_name()));
            row.set_subtitle(&glib::markup_escape_text(&subtitle));

            // A pack lives in the state of a room, so changing or removing
            // one needs the power level to send that state there. Without it
            // the only thing left is to stop using it, which is the switch.
            if source.room.permissions().can_change_image_packs() {
                let id = (
                    source.room.room_id().as_str().to_owned(),
                    source.state_key.clone(),
                );

                let menu = gio::Menu::new();
                menu.append_item(&menu_item(&gettext("_Edit…"), "image-packs.edit", &id));
                menu.append_item(&menu_item(&gettext("_Delete…"), "image-packs.delete", &id));

                let menu_button = gtk::MenuButton::builder()
                    .icon_name("view-more-symbolic")
                    .tooltip_text(gettext("Pack Options"))
                    .valign(gtk::Align::Center)
                    .menu_model(&menu)
                    .css_classes(["flat"])
                    .build();

                row.add_suffix(&menu_button);
            }

            self.connect_switch(
                &row,
                source.room.room_id().to_owned(),
                source.state_key.clone(),
                Some(pack.display_name()),
            );

            row
        }

        /// Make the switch of the given row use the given pack everywhere.
        fn connect_switch(
            &self,
            row: &SwitchLoadingRow,
            room_id: ruma::OwnedRoomId,
            state_key: String,
            name: Option<String>,
        ) {
            let Some(session) = self.session.upgrade() else {
                return;
            };

            row.set_is_active(session.image_packs().is_pack_enabled(&room_id, &state_key));

            row.connect_is_active_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |row| {
                    let room_id = room_id.clone();
                    let state_key = state_key.clone();
                    let name = name.clone();

                    spawn!(clone!(
                        #[weak]
                        imp,
                        #[weak]
                        row,
                        async move {
                            imp.toggle_pack(&room_id, &state_key, name.as_deref(), &row)
                                .await;
                        }
                    ));
                }
            ));
        }

        /// Enable or disable the given pack everywhere, following its row.
        async fn toggle_pack(
            &self,
            room_id: &RoomId,
            state_key: &str,
            name: Option<&str>,
            row: &SwitchLoadingRow,
        ) {
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
                error!("Could not change whether an image pack is used everywhere");

                let message = match name {
                    Some(name) => gettext_f(
                        // Translators: Do NOT translate the content between '{' and '}',
                        // this is a variable name.
                        "Could not change whether “{pack}” is used in every room",
                        &[("pack", name)],
                    ),
                    None => gettext("Could not change whether the pack is used in every room"),
                };
                toast!(self.obj(), message);

                // Put the switch back where it was.
                row.set_is_active(!enabled);
            }
        }

        /// The pack that the given identifier names.
        fn pack(&self, id: &PackId) -> Option<ImagePack> {
            let (room_id, state_key) = id;

            self.packs
                .borrow()
                .iter()
                .find(|pack| {
                    let source = pack.source();
                    source.room.room_id().as_str() == room_id && &source.state_key == state_key
                })
                .cloned()
        }

        /// Open the editor for the pack that the given identifier names.
        fn edit_pack(&self, id: &PackId) {
            let (Some(session), Some(pack)) = (self.session.upgrade(), self.pack(id)) else {
                return;
            };

            self.push_editor(&ImagePackEditor::edit(&session, &pack));
        }

        /// Delete the pack that the given identifier names, after asking for
        /// confirmation.
        async fn delete_pack(&self, id: &PackId) {
            let (Some(session), Some(pack)) = (self.session.upgrade(), self.pack(id)) else {
                return;
            };

            let obj = self.obj();
            if !confirm_delete_image_pack_dialog(&pack.display_name(), &*obj).await {
                return;
            }

            if session
                .image_packs()
                .delete_pack(pack.source())
                .await
                .is_err()
            {
                error!("Could not delete an image pack");
                toast!(
                    obj,
                    gettext_f(
                        // Translators: Do NOT translate the content between '{' and '}',
                        // this is a variable name.
                        "Could not delete “{pack}”",
                        &[("pack", &pack.display_name())],
                    )
                );
            }
        }

        /// Create a pack in the room that packs are created in.
        #[template_callback]
        async fn create_pack(&self) {
            let Some(session) = self.session.upgrade() else {
                return;
            };

            self.create_button.set_is_loading(true);

            // The room is created the first time a pack is, so this is where
            // the user finds out that it exists.
            let Ok(room) = session.image_packs().packs_room().await else {
                self.create_button.set_is_loading(false);
                toast!(
                    self.obj(),
                    gettext("Could not create the room to keep your packs in")
                );
                return;
            };

            let state_key = ImagePacks::unused_state_key(&room).await;
            self.create_button.set_is_loading(false);

            self.push_editor(&ImagePackEditor::create(
                &session,
                ImagePackSource {
                    room,
                    state_key,
                    // A pack that we create uses the event type that we send.
                    kind: RoomPackKind::Unstable,
                },
            ));
        }

        /// Present the given editor on top of this page.
        fn push_editor(&self, editor: &ImagePackEditor) {
            let Some(dialog) = self
                .obj()
                .ancestor(adw::PreferencesDialog::static_type())
                .and_downcast::<adw::PreferencesDialog>()
            else {
                error!("Could not find the dialog of the image packs page");
                return;
            };

            dialog.push_subpage(editor);
        }
    }
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
