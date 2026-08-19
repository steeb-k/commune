use adw::{prelude::*, subclass::prelude::*};
use gtk::{CompositeTemplate, glib, glib::clone};
use tracing::error;

use crate::{
    components::SwitchLoadingRow,
    gettext_f, ngettext_f,
    session::{ImagePack, ImagePackSource, ImagePacks, Room},
    spawn, toast,
};

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
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for ImagePacksSubpage {
        fn constructed(&self) {
            self.parent_constructed();

            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    imp.load().await;
                }
            ));
        }
    }

    impl WidgetImpl for ImagePacksSubpage {}
    impl NavigationPageImpl for ImagePacksSubpage {}

    impl ImagePacksSubpage {
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
