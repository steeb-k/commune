use adw::{prelude::*, subclass::prelude::*};
use futures_channel::oneshot;
use gtk::{gdk, gio, glib, glib::clone, CompositeTemplate};
use tracing::error;

use crate::{components::MediaContentViewer, spawn};

mod imp {
    use std::cell::RefCell;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_history/message_toolbar/attachment_dialog.ui"
    )]
    pub struct AttachmentDialog {
        #[template_child]
        pub cancel_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub send_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub media: TemplateChild<MediaContentViewer>,
        pub sender: RefCell<Option<oneshot::Sender<gtk::ResponseType>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AttachmentDialog {
        const NAME: &'static str = "AttachmentDialog";
        type Type = super::AttachmentDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for AttachmentDialog {
        fn constructed(&self) {
            self.parent_constructed();

            self.set_loading(true);
        }
    }

    impl WidgetImpl for AttachmentDialog {
        fn grab_focus(&self) -> bool {
            let loading = !self.send_button.is_sensitive();

            if loading {
                self.cancel_button.grab_focus()
            } else {
                self.send_button.grab_focus()
            }
        }
    }

    impl AdwDialogImpl for AttachmentDialog {
        fn closed(&self) {
            self.send_response(gtk::ResponseType::Cancel);
        }
    }

    impl AttachmentDialog {
        /// Set whether this dialog is loading.
        pub(super) fn set_loading(&self, loading: bool) {
            self.send_button.set_sensitive(!loading);
            self.grab_focus();
        }

        /// Sent the given response.
        pub(super) fn send_response(&self, response: gtk::ResponseType) {
            if let Some(sender) = self.sender.take() {
                if sender.send(response).is_err() {
                    error!("Could not send attachment dialog response {response:?}");
                }
            }
        }
    }
}

glib::wrapper! {
    /// A dialog to preview an attachment before sending it.
    pub struct AttachmentDialog(ObjectSubclass<imp::AttachmentDialog>)
        @extends gtk::Widget, adw::Dialog;
}

#[gtk::template_callbacks]
impl AttachmentDialog {
    /// Create an attachment dialog with the given title.
    ///
    /// Its initial state is loading.
    pub fn new(title: &str) -> Self {
        glib::Object::builder().property("title", title).build()
    }

    /// Set the image to preview.
    pub fn set_image(&self, image: &gdk::Texture) {
        let imp = self.imp();
        imp.media.view_image(image);
        imp.set_loading(false);
    }

    /// Set the file to preview.
    pub fn set_file(&self, file: gio::File) {
        let imp = self.imp();

        spawn!(clone!(
            #[weak]
            imp,
            async move {
                imp.media.view_file(file, None).await;
                imp.set_loading(false);
            }
        ));
    }

    /// Create an attachment dialog to preview and send a location.
    pub fn set_location(&self, geo_uri: &geo_uri::GeoUri) {
        let imp = self.imp();
        imp.media.view_location(geo_uri);
        imp.set_loading(false);
    }

    /// Emit the signal that the user wants to send the attachment.
    #[template_callback]
    fn send(&self) {
        self.imp().send_response(gtk::ResponseType::Ok);
        self.close();
    }

    /// Present the dialog and wait for the user to select a response.
    ///
    /// The response is [`gtk::ResponseType::Ok`] if the user clicked on send,
    /// otherwise it is [`gtk::ResponseType::Cancel`].
    pub async fn response_future(&self, parent: &impl IsA<gtk::Widget>) -> gtk::ResponseType {
        let (sender, receiver) = oneshot::channel();
        self.imp().sender.replace(Some(sender));

        self.present(Some(parent));

        receiver.await.unwrap_or(gtk::ResponseType::Cancel)
    }
}
