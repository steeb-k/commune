use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{CompositeTemplate, glib, glib::clone};
use tracing::error;

use crate::{
    components::{ImagePackEditor, SwitchLoadingRow},
    gettext_f, ngettext_f,
    session::{ImagePack, ImagePackSource, ImagePacks, Room, RoomPackKind},
    spawn, toast,
};

/// The state key of the first pack of a room.
///
/// The specification does not reserve it, but the clients in the wild use the
/// empty state key for the pack of a room, so a new pack takes it when it is
/// free.
const FIRST_STATE_KEY: &str = "";

/// The prefix of the state keys of the packs after the first one.
const STATE_KEY_PREFIX: &str = "pack";

/// A state key that no pack of the room uses yet.
fn unused_state_key(taken: &[String]) -> String {
    if !taken.iter().any(|key| key == FIRST_STATE_KEY) {
        return FIRST_STATE_KEY.to_owned();
    }

    // Among the state keys that are taken, at most all of them can collide, so
    // one of this many is free.
    (2..=taken.len() + 2)
        .map(|index| format!("{STATE_KEY_PREFIX}-{index}"))
        .find(|key| !taken.contains(key))
        .expect("an unused state key should be found")
}

mod imp {
    use std::cell::RefCell;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[properties(wrapper_type = super::ImagePacksSubpage)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session_view/room_details/image_packs_subpage/mod.ui"
    )]
    pub struct ImagePacksSubpage {
        #[template_child]
        stack: TemplateChild<gtk::Stack>,
        #[template_child]
        packs_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        create_button: TemplateChild<gtk::Button>,
        #[template_child]
        empty_create_button: TemplateChild<gtk::Button>,
        /// The room that the packs are defined in.
        #[property(get, construct_only)]
        pub(super) room: glib::WeakRef<Room>,
        /// The rows that are presented, to be able to remove them.
        rows: RefCell<Vec<SwitchLoadingRow>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ImagePacksSubpage {
        const NAME: &'static str = "ImagePacksSubpage";
        type Type = super::ImagePacksSubpage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for ImagePacksSubpage {
        fn constructed(&self) {
            self.parent_constructed();

            let Some(room) = self.room.upgrade() else {
                return;
            };

            room.permissions()
                .connect_can_change_image_packs_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_create_buttons();
                    }
                ));
            self.update_create_buttons();

            // A pack that was saved from this page, or from the editor that it
            // opens, should be presented without having to leave the room.
            if let Some(session) = room.session() {
                session.image_packs().connect_changed(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.reload();
                    }
                ));
            }

            self.reload();
        }
    }

    impl WidgetImpl for ImagePacksSubpage {}
    impl NavigationPageImpl for ImagePacksSubpage {}

    #[gtk::template_callbacks]
    impl ImagePacksSubpage {
        /// Whether the user can define a pack in the room.
        fn can_change_packs(&self) -> bool {
            self.room
                .upgrade()
                .is_some_and(|room| room.permissions().can_change_image_packs())
        }

        /// Update whether the buttons to create a pack are presented.
        fn update_create_buttons(&self) {
            let can_change_packs = self.can_change_packs();

            self.create_button.set_visible(can_change_packs);
            self.empty_create_button.set_visible(can_change_packs);
        }

        /// Load the packs defined in the room.
        fn reload(&self) {
            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    imp.load().await;
                }
            ));
        }

        /// Load the packs defined in the room.
        async fn load(&self) {
            let Some(room) = self.room.upgrade() else {
                return;
            };

            self.stack.set_visible_child_name("loading");

            // Every pack of the room, whatever it is meant to be used for.
            let packs = ImagePacks::room_packs(&room).await;

            for row in self.rows.take() {
                self.packs_group.remove(&row);
            }

            if packs.is_empty() {
                self.stack.set_visible_child_name("empty");
                return;
            }

            let mut rows = Vec::with_capacity(packs.len());
            for pack in packs {
                let row = self.build_row(&pack);
                self.packs_group.add(&row);
                rows.push(row);
            }
            self.rows.replace(rows);

            self.stack.set_visible_child_name("packs");
        }

        /// Build the row for the given pack.
        fn build_row(&self, pack: &ImagePack) -> SwitchLoadingRow {
            let ImagePackSource::Room {
                room, state_key, ..
            } = pack.source()
            else {
                unreachable!("the packs of a room come from a room");
            };
            let Some(session) = room.session() else {
                unreachable!("the room of a pack belongs to a session");
            };

            let image_count = pack.images().n_items();
            let mut subtitle = ngettext_f(
                // Translators: Do NOT translate the content between '{' and '}',
                // this is a variable name.
                "{count} image",
                "{count} images",
                image_count,
                &[("count", &image_count.to_string())],
            );

            if let Some(attribution) = pack.attribution() {
                subtitle.push_str(" · ");
                subtitle.push_str(attribution);
            }

            let row = SwitchLoadingRow::new();
            row.set_title(&glib::markup_escape_text(&pack.display_name()));
            row.set_subtitle(&glib::markup_escape_text(&subtitle));
            row.set_is_active(
                session
                    .image_packs()
                    .is_pack_enabled(room.room_id(), state_key),
            );

            if self.can_change_packs() {
                let edit_button = gtk::Button::builder()
                    .icon_name("document-edit-symbolic")
                    .tooltip_text(gettext("Edit Pack"))
                    .valign(gtk::Align::Center)
                    .css_classes(["flat"])
                    .build();

                edit_button.connect_clicked(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    #[strong]
                    pack,
                    move |_| {
                        imp.edit_pack(&pack);
                    }
                ));

                row.add_suffix(&edit_button);
            }

            row.connect_is_active_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                #[strong]
                pack,
                move |row| {
                    spawn!(clone!(
                        #[weak]
                        imp,
                        #[weak]
                        row,
                        #[strong]
                        pack,
                        async move {
                            imp.toggle_pack(&pack, &row).await;
                        }
                    ));
                }
            ));

            row
        }

        /// Open the editor for the given pack.
        fn edit_pack(&self, pack: &ImagePack) {
            let Some(session) = self.room.upgrade().and_then(|room| room.session()) else {
                return;
            };

            self.push_subpage(&ImagePackEditor::edit(&session, pack));
        }

        /// Open the editor for a new pack in the room.
        #[template_callback]
        async fn create_pack(&self) {
            let Some(room) = self.room.upgrade() else {
                return;
            };
            let Some(session) = room.session() else {
                return;
            };

            let taken = ImagePacks::room_pack_state_keys(&room).await;
            let source = ImagePackSource::Room {
                room,
                state_key: unused_state_key(&taken),
                // A pack that we create uses the event type that we send.
                kind: RoomPackKind::Unstable,
            };

            self.push_subpage(&ImagePackEditor::create(&session, source));
        }

        /// Present the given page on top of this one.
        fn push_subpage(&self, page: &impl IsA<adw::NavigationPage>) {
            let Some(window) = self
                .obj()
                .ancestor(adw::PreferencesWindow::static_type())
                .and_downcast::<adw::PreferencesWindow>()
            else {
                error!("Could not find the window of the image packs of a room");
                return;
            };

            window.push_subpage(page);
        }

        /// Enable or disable the given pack globally, following its row.
        async fn toggle_pack(&self, pack: &ImagePack, row: &SwitchLoadingRow) {
            let ImagePackSource::Room {
                room, state_key, ..
            } = pack.source()
            else {
                return;
            };
            let Some(session) = room.session() else {
                return;
            };

            let enabled = row.is_active();
            let image_packs = session.image_packs();

            if image_packs.is_pack_enabled(room.room_id(), state_key) == enabled {
                return;
            }

            row.set_is_loading(true);
            let result = image_packs
                .set_pack_enabled(room.room_id(), state_key, enabled)
                .await;
            row.set_is_loading(false);

            if result.is_err() {
                error!("Could not change whether an image pack is enabled");
                toast!(
                    self.obj(),
                    gettext_f(
                        // Translators: Do NOT translate the content between '{' and '}',
                        // this is a variable name.
                        "Could not change whether “{pack}” is used in every room",
                        &[("pack", &pack.display_name())],
                    )
                );

                // Put the switch back where it was.
                row.set_is_active(!enabled);
            }
        }
    }
}

glib::wrapper! {
    /// A subpage to manage the image packs defined in a room.
    pub struct ImagePacksSubpage(ObjectSubclass<imp::ImagePacksSubpage>)
        @extends gtk::Widget, adw::NavigationPage,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ImagePacksSubpage {
    /// Create a new `ImagePacksSubpage` for the given room.
    pub(crate) fn new(room: &Room) -> Self {
        glib::Object::builder().property("room", room).build()
    }
}
