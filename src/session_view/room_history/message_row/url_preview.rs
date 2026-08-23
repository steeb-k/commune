use adw::subclass::prelude::*;
use gtk::{CompositeTemplate, gdk, gio, glib, glib::clone, prelude::*};
use ruma::api::client::media::get_content_thumbnail::v3::Method;
use tracing::warn;
use url::Url;

use crate::{
    prelude::*,
    session::{RemoteUrlPreview, Room, Session},
    spawn,
    utils::{
        BoundObjectWeakRef, LoadingState,
        media::{
            FrameDimensions,
            image::{ImageRequestPriority, ImageSource, ThumbnailDownloader, ThumbnailSettings},
        },
    },
};

/// The size of the image on a card, in logical pixels.
///
/// This matches the `pixel-size` of the image in the template.
const IMAGE_SIZE: u32 = 72;

mod imp {
    use super::*;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session_view/room_history/message_row/url_preview.ui"
    )]
    pub struct MessageUrlPreview {
        #[template_child]
        content: TemplateChild<adw::Bin>,
        #[template_child]
        card: TemplateChild<gtk::Button>,
        #[template_child]
        image: TemplateChild<gtk::Image>,
        #[template_child]
        site_name: TemplateChild<gtk::Label>,
        #[template_child]
        title: TemplateChild<gtk::Label>,
        #[template_child]
        description: TemplateChild<gtk::Label>,
        /// The preview that is presented.
        preview: BoundObjectWeakRef<RemoteUrlPreview>,
        /// The session the preview belongs to.
        session: glib::WeakRef<Session>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MessageUrlPreview {
        const NAME: &'static str = "ContentMessageUrlPreview";
        type Type = super::MessageUrlPreview;
        type ParentType = gtk::Grid;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);

            klass.set_css_name("message-url-preview");
            klass.set_accessible_role(gtk::AccessibleRole::Group);
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for MessageUrlPreview {}
    impl WidgetImpl for MessageUrlPreview {}
    impl GridImpl for MessageUrlPreview {}

    #[gtk::template_callbacks]
    impl MessageUrlPreview {
        /// The widget displaying the message that the card belongs to.
        pub(super) fn child(&self) -> Option<gtk::Widget> {
            adw::prelude::BinExt::child(&*self.content)
        }

        /// Set the widget displaying the message that the card belongs to.
        pub(super) fn set_child(&self, child: Option<&impl IsA<gtk::Widget>>) {
            adw::prelude::BinExt::set_child(&*self.content, child);
        }

        /// Present the preview of the given URL in the given room.
        pub(super) fn set_url(&self, room: &Room, url: Url) {
            if self.preview.obj().is_some_and(|p| p.url() == url) {
                // The card is already the one for this URL. This widget is
                // reused as the timeline scrolls, so this is the common case.
                return;
            }

            self.preview.disconnect_signals();
            self.reset_card();

            let Some(session) = room.session() else {
                return;
            };

            let preview = session.remote_cache().url_preview(url);
            self.session.set(Some(&session));

            let handler = preview.connect_loading_state_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |preview| {
                    imp.update_card(preview);
                }
            ));

            self.preview.set(&preview, vec![handler]);
            self.update_card(&preview);
        }

        /// Hide the card and drop what it was showing.
        fn reset_card(&self) {
            self.card.set_visible(false);
            self.image.set_visible(false);
            self.image.set_paintable(gdk::Paintable::NONE);
        }

        /// Update the card for the state of the given preview.
        fn update_card(&self, preview: &RemoteUrlPreview) {
            if preview.loading_state() != LoadingState::Ready {
                // Nothing is shown until there is something to show. A
                // placeholder that might never be replaced would make the
                // timeline jump for a card that never arrives.
                self.reset_card();
                return;
            }

            self.site_name.set_label(&preview.site_name());
            set_optional_label(&self.title, preview.title().as_deref());
            set_optional_label(&self.description, preview.description().as_deref());

            self.card.set_tooltip_text(Some(preview.url().as_str()));
            self.card.set_visible(true);

            self.load_image(preview);
        }

        /// Load the image of the given preview, if it has one.
        fn load_image(&self, preview: &RemoteUrlPreview) {
            self.image.set_visible(false);
            self.image.set_paintable(gdk::Paintable::NONE);

            let Some(image) = preview.image() else {
                return;
            };
            let Some(session) = self.session.upgrade() else {
                return;
            };

            let scale_factor = u32::try_from(self.obj().scale_factor()).unwrap_or(1);
            let dimensions = FrameDimensions {
                width: IMAGE_SIZE,
                height: IMAGE_SIZE,
            }
            .scale(scale_factor);

            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                #[strong]
                preview,
                async move {
                    let downloader = ThumbnailDownloader {
                        main: ImageSource {
                            source: (&image.uri).into(),
                            info: Some((&image.info).into()),
                        },
                        // The homeserver uploaded the image itself, so there is
                        // no lower-quality source to fall back to.
                        alt: None,
                    };
                    let settings = ThumbnailSettings {
                        dimensions,
                        // Scale, never crop. A preview image is usually a wide
                        // banner with words on it, and cropping it to a square
                        // takes the sides off — the homeserver does the
                        // cropping, so no amount of care in the widget helps.
                        method: Method::Scale,
                        animated: false,
                        prefer_thumbnail: true,
                    };

                    let result = downloader
                        .download(session.client(), settings, ImageRequestPriority::Low)
                        .await;

                    // The widget is reused, so it might be showing another
                    // message by the time the image arrives.
                    if imp.preview.obj().as_ref() != Some(&preview) {
                        return;
                    }

                    match result {
                        Ok(image) => {
                            imp.image.set_paintable(Some(&gdk::Paintable::from(image)));
                            imp.image.set_visible(true);
                        }
                        Err(error) => {
                            warn!("Could not load the image of a URL preview: {error}");
                        }
                    }
                }
            ));
        }

        /// Open the URL of the preview.
        #[template_callback]
        fn open_url(&self) {
            let Some(preview) = self.preview.obj() else {
                return;
            };

            let window = self.obj().root().and_downcast::<gtk::Window>();

            gtk::UriLauncher::new(preview.url().as_str()).launch(
                window.as_ref(),
                gio::Cancellable::NONE,
                |result| {
                    if let Err(error) = result {
                        warn!("Could not open the URL of a preview: {error}");
                    }
                },
            );
        }
    }
}

glib::wrapper! {
    /// A widget displaying a message alongside a preview of its first link.
    pub struct MessageUrlPreview(ObjectSubclass<imp::MessageUrlPreview>)
        @extends gtk::Widget, gtk::Grid,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl MessageUrlPreview {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Present the preview of the given URL in the given room.
    pub(crate) fn set_url(&self, room: &Room, url: Url) {
        self.imp().set_url(room, url);
    }
}

impl Default for MessageUrlPreview {
    fn default() -> Self {
        Self::new()
    }
}

impl ChildPropertyExt for MessageUrlPreview {
    fn child_property(&self) -> Option<gtk::Widget> {
        self.imp().child()
    }

    fn set_child_property(&self, child: Option<&impl IsA<gtk::Widget>>) {
        self.imp().set_child(child);
    }
}

/// Set the label of the given widget, hiding it when there is no text.
fn set_optional_label(label: &gtk::Label, text: Option<&str>) {
    label.set_visible(text.is_some());
    label.set_label(text.unwrap_or_default());
}
