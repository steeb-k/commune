use adw::{prelude::*, subclass::prelude::*};
use gtk::{CompositeTemplate, glib, glib::clone};
use tracing::error;

use crate::{
    session::{PackImage, PackImageData, Session, is_valid_shortcode},
    spawn,
};

/// The size at which the image of a row is presented, in pixels.
const IMAGE_SIZE: u32 = 48;

mod imp {
    use std::cell::OnceCell;

    use glib::subclass::{InitializingObject, Signal};

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/image_pack_editor/image_row.ui")]
    #[properties(wrapper_type = super::PackImageRow)]
    pub struct PackImageRow {
        #[template_child]
        picture: TemplateChild<gtk::Image>,
        /// The session that the image belongs to.
        #[property(get, construct_only)]
        pub(super) session: OnceCell<Session>,
        /// The image of this row.
        #[property(get, construct_only)]
        pub(super) image: OnceCell<PackImage>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PackImageRow {
        const NAME: &'static str = "PackImageRow";
        type Type = super::PackImageRow;
        type ParentType = adw::EntryRow;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for PackImageRow {
        fn signals() -> &'static [Signal] {
            static SIGNALS: std::sync::LazyLock<Vec<Signal>> =
                std::sync::LazyLock::new(|| vec![Signal::builder("removed").build()]);
            SIGNALS.as_ref()
        }

        fn constructed(&self) {
            self.parent_constructed();

            let obj = self.obj();
            let image = self.image.get().expect("image should be initialized");

            self.picture.set_pixel_size(IMAGE_SIZE.cast_signed());

            obj.set_text(&image.shortcode());

            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    imp.load().await;
                }
            ));
        }
    }

    impl WidgetImpl for PackImageRow {}
    impl ListBoxRowImpl for PackImageRow {}
    impl PreferencesRowImpl for PackImageRow {}
    impl ActionRowImpl for PackImageRow {}
    impl EntryRowImpl for PackImageRow {}

    #[gtk::template_callbacks]
    impl PackImageRow {
        /// Load the image of this row.
        async fn load(&self) {
            let obj = self.obj();
            let image = self.image.get().expect("image should be initialized");
            let session = self.session.get().expect("session should be initialized");

            match image
                .download_thumbnail(session, IMAGE_SIZE, obj.scale_factor())
                .await
            {
                Ok(paintable) => self.picture.set_paintable(Some(&paintable)),
                Err(error) => {
                    // The image is still presented, so that the pack does not
                    // silently lose it.
                    error!("Could not load the image of a pack: {error}");
                    self.picture.set_icon_name(Some("image-missing-symbolic"));
                }
            }
        }

        /// Update whether the shortcode of this row is valid.
        #[template_callback]
        fn update_validity(&self) {
            // Being a duplicate of another row is the editor's business, so it
            // only clears the state that this row can decide on its own.
            if self.obj().is_shortcode_valid() {
                self.obj().remove_css_class("error");
            } else {
                self.obj().add_css_class("error");
            }
        }

        /// Remove this row from its pack.
        #[template_callback]
        fn remove(&self) {
            self.obj().emit_by_name::<()>("removed", &[]);
        }
    }
}

glib::wrapper! {
    /// A row to edit the shortcode of an image of a pack.
    pub struct PackImageRow(ObjectSubclass<imp::PackImageRow>)
        @extends gtk::Widget, gtk::ListBoxRow, adw::PreferencesRow, adw::ActionRow, adw::EntryRow,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Actionable,
        gtk::Editable;
}

impl PackImageRow {
    /// Create a new `PackImageRow` for the given image.
    pub(crate) fn new(session: &Session, image: &PackImage) -> Self {
        glib::Object::builder()
            .property("session", session)
            .property("image", image)
            .build()
    }

    /// The shortcode currently in this row.
    pub(crate) fn shortcode(&self) -> String {
        self.text().trim().to_owned()
    }

    /// Whether the shortcode currently in this row matches the grammar.
    pub(crate) fn is_shortcode_valid(&self) -> bool {
        is_valid_shortcode(&self.shortcode())
    }

    /// Mark the shortcode of this row as a duplicate of another one.
    pub(crate) fn set_shortcode_duplicate(&self, duplicate: bool) {
        if duplicate {
            self.add_css_class("error");
        } else if self.is_shortcode_valid() {
            self.remove_css_class("error");
        }
    }

    /// The data of the image of this row.
    pub(crate) fn data(&self) -> PackImageData {
        self.image().data()
    }

    /// Connect to the signal emitted when this row is removed.
    pub(crate) fn connect_removed<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "removed",
            true,
            glib::closure_local!(move |obj: Self| {
                f(&obj);
            }),
        )
    }
}
