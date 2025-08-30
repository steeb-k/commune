use adw::{prelude::*, subclass::prelude::*};
use futures_channel::oneshot;
use gtk::{CompositeTemplate, gdk, gio, glib, glib::clone};
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
        cancel_button: TemplateChild<gtk::Button>,
        #[template_child]
        send_button: TemplateChild<gtk::Button>,
        #[template_child]
        media: TemplateChild<MediaContentViewer>,
        sender: RefCell<Option<oneshot::Sender<gtk::ResponseType>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AttachmentDialog {
        const NAME: &'static str = "AttachmentDialog";
        type Type = super::AttachmentDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);
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

    #[gtk::template_callbacks]
    impl AttachmentDialog {
        /// Set whether this dialog is loading.
        fn set_loading(&self, loading: bool) {
            self.send_button.set_sensitive(!loading);
            self.grab_focus();
        }

        /// Sent the given response.
        fn send_response(&self, response: gtk::ResponseType) {
            if self
                .sender
                .take()
                .is_some_and(|sender| sender.send(response).is_err())
            {
                error!("Could not send attachment dialog response {response:?}");
            }
        }

        /// Set the image to preview.
        pub(super) fn set_image(&self, image: &gdk::Texture) {
            self.media.view_image(image);
            self.set_loading(false);
        }

        /// Set the file to preview.
        pub(super) async fn set_file(&self, file: gio::File) {
            self.media.view_file(file.into(), None).await;
            self.set_loading(false);
        }

        /// Set the location to preview.
        pub(super) fn set_location(&self, geo_uri: &geo_uri::GeoUri) {
            self.media.view_location(geo_uri);
            self.set_loading(false);
        }

        /// Emit the signal that the user wants to send the attachment.
        #[template_callback]
        fn send(&self) {
            self.send_response(gtk::ResponseType::Ok);
            self.obj().close();
        }

        /// Present the dialog and wait for the user to select a response.
        ///
        /// The response is [`gtk::ResponseType::Ok`] if the user clicked on
        /// send, otherwise it is [`gtk::ResponseType::Cancel`].
        pub(super) async fn response_future(&self, parent: &gtk::Widget) -> gtk::ResponseType {
            let (sender, receiver) = oneshot::channel();
            self.sender.replace(Some(sender));

            self.obj().present(Some(parent));

            receiver.await.unwrap_or(gtk::ResponseType::Cancel)
        }
    }
}

glib::wrapper! {
    /// A dialog to preview an attachment before sending it.
    pub struct AttachmentDialog(ObjectSubclass<imp::AttachmentDialog>)
        @extends gtk::Widget, adw::Dialog;
}

impl AttachmentDialog {
    /// Create an attachment dialog with the given title.
    ///
    /// Its initial state is loading.
    pub fn new(title: &str) -> Self {
        glib::Object::builder().property("title", title).build()
    }

    /// Set the image to preview.
    pub(crate) fn set_image(&self, image: &gdk::Texture) {
        self.imp().set_image(image);
    }

    /// Set the file to preview.
    pub(crate) fn set_file(&self, file: gio::File) {
        let imp = self.imp();

        spawn!(clone!(
            #[weak]
            imp,
            async move {
                imp.set_file(file).await;
            }
        ));
    }

    /// Create an attachment dialog to preview and send a location.
    pub(crate) fn set_location(&self, geo_uri: &geo_uri::GeoUri) {
        self.imp().set_location(geo_uri);
    }

    /// Present the dialog and wait for the user to select a response.
    ///
    /// The response is [`gtk::ResponseType::Ok`] if the user clicked on send,
    /// otherwise it is [`gtk::ResponseType::Cancel`].
    pub(crate) async fn response_future(
        &self,
        parent: &impl IsA<gtk::Widget>,
    ) -> gtk::ResponseType {
        self.imp().response_future(parent.upcast_ref()).await
    }
}
