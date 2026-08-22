use adw::prelude::*;
use gtk::{
    CompositeTemplate, glib,
    glib::{clone, closure_local},
    pango,
    subclass::prelude::*,
};

mod gif_button;
mod gif_page;
mod pack_image_button;

use self::{gif_page::GifPage, pack_image_button::PackImageButton};
use crate::{
    Application,
    session::{ImagePack, ImagePacks, PackImage, PackUsage, Room, Session},
    spawn,
    utils::klipy::{self, SelectedGif},
};

mod imp {
    use std::{
        cell::{Cell, RefCell},
        sync::LazyLock,
    };

    use glib::subclass::Signal;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[properties(wrapper_type = super::StickerPicker)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session_view/room_history/message_toolbar/sticker_picker/mod.ui"
    )]
    pub struct StickerPicker {
        #[template_child]
        switcher: TemplateChild<adw::InlineViewSwitcher>,
        #[template_child]
        stack: TemplateChild<gtk::Stack>,
        #[template_child]
        packs_box: TemplateChild<gtk::Box>,
        #[template_child]
        gif_view_page: TemplateChild<adw::ViewStackPage>,
        #[template_child]
        gif_page: TemplateChild<GifPage>,
        /// The room that the stickers are sent to.
        #[property(get, set = Self::set_room, explicit_notify, nullable)]
        room: glib::WeakRef<Room>,
        /// Whether the packs were loaded for the current room.
        loaded: Cell<bool>,
        /// The handler watching the packs for changes.
        image_packs_handler: RefCell<Option<(ImagePacks, glib::SignalHandlerId)>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for StickerPicker {
        const NAME: &'static str = "StickerPicker";
        type Type = super::StickerPicker;
        type ParentType = gtk::Popover;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for StickerPicker {
        fn signals() -> &'static [Signal] {
            static SIGNALS: LazyLock<Vec<Signal>> = LazyLock::new(|| {
                vec![
                    Signal::builder("sticker-selected")
                        .param_types([PackImage::static_type()])
                        .build(),
                    Signal::builder("gif-selected")
                        .param_types([SelectedGif::static_type()])
                        .build(),
                ]
            });
            SIGNALS.as_ref()
        }

        fn constructed(&self) {
            self.parent_constructed();

            // The GIF search is only built when this build has an API key, and it is
            // only presented when the user has turned it on: searching sends the
            // search text and the IP address of this device to a third party.
            if klipy::is_available() {
                Application::default()
                    .settings()
                    .bind("gif-search-enabled", &*self.gif_view_page, "visible")
                    .get_only()
                    .build();

                // A switcher between one page is noise.
                self.gif_view_page
                    .bind_property("visible", &*self.switcher, "visible")
                    .sync_create()
                    .build();
            }

            self.gif_page.connect_gif_selected(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_, gif| {
                    let obj = imp.obj();
                    obj.emit_by_name::<()>("gif-selected", &[&gif]);
                    obj.popdown();
                }
            ));
        }
    }

    impl WidgetImpl for StickerPicker {
        fn map(&self) {
            self.parent_map();

            if !self.loaded.get() {
                spawn!(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    async move {
                        imp.load().await;
                    }
                ));
            }
        }
    }

    impl PopoverImpl for StickerPicker {}

    impl StickerPicker {
        /// Set the room that the stickers are sent to.
        fn set_room(&self, room: Option<&Room>) {
            if self.room.upgrade().as_ref() == room {
                return;
            }

            if let Some((image_packs, handler)) = self.image_packs_handler.take() {
                image_packs.disconnect(handler);
            }

            self.room.set(room);
            self.invalidate();

            // The packs are loaded once per room, so watch for the changes
            // that would make them stale.
            if let Some(image_packs) = room.and_then(Room::session).map(|s| s.image_packs()) {
                let handler = image_packs.connect_changed(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.invalidate();
                    }
                ));
                self.image_packs_handler
                    .replace(Some((image_packs, handler)));
            }

            self.obj().notify_room();
        }

        /// Forget the packs that are presented, and load them again if we are
        /// presented.
        fn invalidate(&self) {
            self.loaded.set(false);
            self.clear();

            if self.obj().is_mapped() {
                spawn!(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    async move {
                        imp.load().await;
                    }
                ));
            }
        }

        /// Remove the presented packs.
        fn clear(&self) {
            while let Some(child) = self.packs_box.first_child() {
                self.packs_box.remove(&child);
            }
        }

        /// Load the packs that can be used in the current room.
        async fn load(&self) {
            let Some(room) = self.room.upgrade() else {
                self.stack.set_visible_child_name("empty");
                return;
            };
            let Some(session) = room.session() else {
                self.stack.set_visible_child_name("empty");
                return;
            };

            self.stack.set_visible_child_name("loading");

            let packs = session
                .image_packs()
                .packs_for_room(&room, Some(&PackUsage::Sticker))
                .await;

            // The room might have changed while we were loading.
            if self.room.upgrade().as_ref() != Some(&room) {
                return;
            }

            self.clear();

            if packs.is_empty() {
                self.stack.set_visible_child_name("empty");
                self.loaded.set(true);
                return;
            }

            for pack in packs {
                self.packs_box.append(&self.build_pack(&session, &pack));
            }

            self.stack.set_visible_child_name("packs");
            self.loaded.set(true);
        }

        /// Build the presentation of the given pack, as a card that can be
        /// collapsed.
        fn build_pack(&self, session: &Session, pack: &ImagePack) -> gtk::Widget {
            let name = gtk::Label::builder()
                .label(pack.display_name())
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(pango::EllipsizeMode::End)
                .build();
            name.add_css_class("heading");

            let images_box = gtk::FlowBox::builder()
                .selection_mode(gtk::SelectionMode::None)
                .homogeneous(true)
                .min_children_per_line(4)
                .max_children_per_line(10)
                .row_spacing(6)
                .column_spacing(6)
                // Separate the images from the title of the pack.
                .margin_top(6)
                .build();

            let images = pack.images();
            for position in 0..images.n_items() {
                let Some(image) = images.item(position).and_downcast::<PackImage>() else {
                    continue;
                };

                let button = PackImageButton::new(session, &image);
                button.connect_clicked(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    #[strong]
                    image,
                    move |_| {
                        let obj = imp.obj();
                        obj.emit_by_name::<()>("sticker-selected", &[&image]);
                        obj.popdown();
                    }
                ));

                images_box.append(&button);
            }

            let expander = gtk::Expander::builder()
                .label_widget(&name)
                .child(&images_box)
                .expanded(true)
                .build();

            let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
            card.add_css_class("card");
            card.add_css_class("sticker-pack");
            card.append(&expander);

            card.upcast()
        }
    }
}

glib::wrapper! {
    /// A popover to send a sticker from the image packs available in a room.
    pub struct StickerPicker(ObjectSubclass<imp::StickerPicker>)
        @extends gtk::Widget, gtk::Popover,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Native,
            gtk::ShortcutManager;
}

impl StickerPicker {
    /// Connect to the signal emitted when a sticker is selected.
    pub(crate) fn connect_sticker_selected<F: Fn(&Self, PackImage) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "sticker-selected",
            true,
            closure_local!(move |obj: Self, image: PackImage| {
                f(&obj, image);
            }),
        )
    }

    /// Connect to the signal emitted when a GIF is selected.
    pub(crate) fn connect_gif_selected<F: Fn(&Self, SelectedGif) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "gif-selected",
            true,
            closure_local!(move |obj: Self, gif: SelectedGif| {
                f(&obj, gif);
            }),
        )
    }
}
