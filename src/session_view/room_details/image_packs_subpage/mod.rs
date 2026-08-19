use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{CompositeTemplate, glib, glib::clone};

use crate::{
    gettext_f, ngettext_f,
    prelude::*,
    session::{ImagePack, Room},
    spawn,
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
        /// The room that the packs can be used in.
        #[property(get, construct_only)]
        pub(super) room: glib::WeakRef<Room>,
        /// The rows that are presented, to be able to remove them.
        rows: RefCell<Vec<adw::ActionRow>>,
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

            let Some(room) = self.room.upgrade() else {
                return;
            };

            // A pack that was created, edited or turned on elsewhere should be
            // presented without having to leave the room.
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

    impl ImagePacksSubpage {
        /// Load the packs that can be used in the room.
        fn reload(&self) {
            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    imp.load().await;
                }
            ));
        }

        /// Load the packs that can be used in the room.
        async fn load(&self) {
            let Some(room) = self.room.upgrade() else {
                return;
            };
            let Some(session) = room.session() else {
                return;
            };

            self.stack.set_visible_child_name("loading");

            // Every pack usable here, whatever it is meant to be used for, in
            // the order the specification asks for.
            let packs = session.image_packs().packs_for_room(&room, None).await;

            for row in self.rows.take() {
                self.packs_group.remove(&row);
            }

            if packs.is_empty() {
                self.stack.set_visible_child_name("empty");
                return;
            }

            let mut rows = Vec::with_capacity(packs.len());
            for pack in packs {
                let row = build_row(&pack, &room);
                self.packs_group.add(&row);
                rows.push(row);
            }
            self.rows.replace(rows);

            self.stack.set_visible_child_name("packs");
        }
    }
}

/// Build the row for the given pack, as it applies to the given room.
fn build_row(pack: &ImagePack, room: &Room) -> adw::ActionRow {
    let source = pack.source();
    let image_count = pack.images().n_items();

    let mut subtitle = ngettext_f(
        // Translators: Do NOT translate the content between '{' and '}', this
        // is a variable name.
        "{count} image",
        "{count} images",
        image_count,
        &[("count", &image_count.to_string())],
    );

    subtitle.push_str(" · ");

    if source.room.room_id() == room.room_id() {
        subtitle.push_str(&gettext("defined in this room"));
    } else {
        subtitle.push_str(&gettext_f(
            // Translators: Do NOT translate the content between '{' and '}',
            // this is a variable name. This says where a pack that is used in
            // every room comes from.
            "used everywhere, from {room}",
            &[("room", &source.room.display_name())],
        ));
    }

    if let Some(attribution) = pack.attribution() {
        subtitle.push_str(" · ");
        subtitle.push_str(attribution);
    }

    adw::ActionRow::builder()
        .title(glib::markup_escape_text(&pack.display_name()))
        .subtitle(glib::markup_escape_text(&subtitle))
        .selectable(false)
        .activatable(false)
        .build()
}

glib::wrapper! {
    /// A subpage listing the image packs that can be used in a room.
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
