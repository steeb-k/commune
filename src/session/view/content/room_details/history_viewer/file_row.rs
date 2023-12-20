use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{gio, glib, CompositeTemplate};
use matrix_sdk::ruma::events::{room::message::MessageType, AnyMessageLikeEventContent};
use tracing::error;

use super::HistoryViewerEvent;
use crate::{prelude::*, toast};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_details/history_viewer/file_row.ui"
    )]
    #[properties(wrapper_type = super::FileRow)]
    pub struct FileRow {
        /// The file event.
        #[property(get, set = Self::set_event, explicit_notify, nullable)]
        pub event: RefCell<Option<HistoryViewerEvent>>,
        pub file: RefCell<Option<gio::File>>,
        #[template_child]
        pub button: TemplateChild<gtk::Button>,
        #[template_child]
        pub title_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub size_label: TemplateChild<gtk::Label>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FileRow {
        const NAME: &'static str = "ContentFileHistoryViewerRow";
        type Type = super::FileRow;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);

            klass.install_action_async(
                "file-row.save-file",
                None,
                move |widget, _, _| async move {
                    widget.save_file().await;
                },
            );
            klass.install_action("file-row.open-file", None, move |widget, _, _| {
                widget.open_file();
            });
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for FileRow {}

    impl WidgetImpl for FileRow {}
    impl BinImpl for FileRow {}

    impl FileRow {
        /// Set the file event.
        fn set_event(&self, event: Option<HistoryViewerEvent>) {
            if *self.event.borrow() == event {
                return;
            }

            if let Some(event) = &event {
                if let Some(AnyMessageLikeEventContent::RoomMessage(content)) =
                    event.original_content()
                {
                    if let MessageType::File(file) = content.msgtype {
                        self.title_label.set_label(&file.body);

                        if let Some(size) = file.info.and_then(|i| i.size) {
                            let size = glib::format_size(size.into());
                            self.size_label.set_label(&size);
                        } else {
                            self.size_label.set_label(&gettext("Unknown size"));
                        }
                    }
                }
            }

            self.event.replace(event);
            self.obj().notify_event();
        }
    }
}

glib::wrapper! {
    /// A row presenting a file event.
    pub struct FileRow(ObjectSubclass<imp::FileRow>)
        @extends gtk::Widget, adw::Bin;
}

impl FileRow {
    async fn save_file(&self) {
        let (filename, data) = match self.event().unwrap().get_file_content().await {
            Ok(res) => res,
            Err(err) => {
                error!("Could not get file: {}", err);
                toast!(self, err.to_user_facing());

                return;
            }
        };

        let parent_window = self.root().and_downcast::<gtk::Window>().unwrap();
        let dialog = gtk::FileDialog::builder()
            .title(gettext("Save File"))
            .accept_label(gettext("Save"))
            .initial_name(filename)
            .build();

        if let Ok(file) = dialog.save_future(Some(&parent_window)).await {
            file.replace_contents(
                &data,
                None,
                false,
                gio::FileCreateFlags::REPLACE_DESTINATION,
                gio::Cancellable::NONE,
            )
            .unwrap();

            let imp = self.imp();

            imp.file.replace(Some(file));
            imp.button.set_icon_name("document-symbolic");
            imp.button.set_action_name(Some("file-row.open-file"));
        }
    }

    fn open_file(&self) {
        if let Some(file) = self.imp().file.borrow().as_ref() {
            if let Err(e) =
                gio::AppInfo::launch_default_for_uri(&file.uri(), gio::AppLaunchContext::NONE)
            {
                error!("Error: {e}");
            }
        }
    }
}
