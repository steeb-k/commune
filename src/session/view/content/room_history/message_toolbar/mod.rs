use std::collections::HashMap;

use adw::{prelude::*, subclass::prelude::*};
use futures_util::{future, lock::Mutex, pin_mut, StreamExt};
use gettextrs::{gettext, pgettext};
use gtk::{
    gdk, gio,
    glib::{self, clone},
    CompositeTemplate,
};
use matrix_sdk::{
    attachment::{AttachmentConfig, AttachmentInfo, BaseFileInfo, Thumbnail},
    room::edit::EditedContent,
};
use matrix_sdk_ui::timeline::{
    AttachmentSource, EnforceThread, RepliedToInfo, TimelineItemContent,
};
use ruma::{
    events::{
        room::message::{LocationMessageEventContent, MessageType, RoomMessageEventContent},
        Mentions,
    },
    OwnedRoomId,
};
use tracing::{debug, error, warn};

mod attachment_dialog;
mod completion;
mod composer_parser;
mod composer_state;

pub(crate) use self::composer_state::{ComposerState, RelationInfo};
use self::{
    attachment_dialog::AttachmentDialog, completion::CompletionPopover,
    composer_parser::ComposerParser,
};
use super::message_row::MessageContent;
use crate::{
    components::{CustomEntry, LabelWithWidgets},
    gettext_f,
    prelude::*,
    session::model::{Event, Member, Room, Timeline},
    spawn, spawn_tokio, toast,
    utils::{
        media::{
            filename_for_mime, image::ImageInfoLoader, load_audio_info, video::load_video_info,
            FileInfo,
        },
        template_callbacks::TemplateCallbacks,
        Location, LocationError, TokioDrop,
    },
};

/// A map of composer state per-session and per-room.
type ComposerStatesMap = HashMap<Option<String>, HashMap<Option<OwnedRoomId>, ComposerState>>;

mod imp {
    use std::{
        cell::{Cell, RefCell},
        marker::PhantomData,
    };

    use glib::subclass::InitializingObject;

    use super::*;
    use crate::Application;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_history/message_toolbar/mod.ui"
    )]
    #[properties(wrapper_type = super::MessageToolbar)]
    pub struct MessageToolbar {
        #[template_child]
        main_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub(super) message_entry: TemplateChild<sourceview::View>,
        #[template_child]
        send_button: TemplateChild<gtk::Button>,
        #[template_child]
        related_event_header: TemplateChild<LabelWithWidgets>,
        #[template_child]
        related_event_content: TemplateChild<MessageContent>,
        /// The timeline used to send messages.
        #[property(get, set = Self::set_timeline, explicit_notify, nullable)]
        timeline: glib::WeakRef<Timeline>,
        send_message_permission_handler: RefCell<Option<glib::SignalHandlerId>>,
        /// Whether outgoing messages should be interpreted as markdown.
        #[property(get, set)]
        markdown_enabled: Cell<bool>,
        completion: CompletionPopover,
        /// The current composer state.
        #[property(get = Self::current_composer_state)]
        current_composer_state: PhantomData<ComposerState>,
        composer_state_handler: RefCell<Option<glib::SignalHandlerId>>,
        buffer_handlers: RefCell<Option<(glib::SignalHandlerId, glib::Binding)>>,
        /// The composer states, per-session and per-room.
        ///
        /// The fallback composer state has the `None` key.
        composer_states: RefCell<ComposerStatesMap>,
        /// A guard to avoid sending several messages at once.
        send_guard: Mutex<()>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MessageToolbar {
        const NAME: &'static str = "MessageToolbar";
        type Type = super::MessageToolbar;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            CustomEntry::ensure_type();

            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);
            TemplateCallbacks::bind_template_callbacks(klass);

            // Menu actions.
            klass.install_action_async(
                "message-toolbar.send-location",
                None,
                |obj, _, _| async move {
                    obj.imp().send_location().await;
                },
            );

            klass.install_property_action("message-toolbar.markdown", "markdown-enabled");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for MessageToolbar {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            // Markdown highlighting.
            let settings = Application::default().settings();
            settings
                .bind("markdown-enabled", &*obj, "markdown-enabled")
                .build();

            // Tab auto-completion.
            self.completion.set_parent(&*self.message_entry);

            // Location.
            let location = Location::new();
            obj.action_set_enabled("message-toolbar.send-location", location.is_available());
        }

        fn dispose(&self) {
            self.completion.unparent();

            if let Some(timeline) = self.timeline.upgrade() {
                if let Some(handler) = self.send_message_permission_handler.take() {
                    timeline.room().permissions().disconnect(handler);
                }
            }
        }
    }

    impl WidgetImpl for MessageToolbar {
        fn grab_focus(&self) -> bool {
            if self
                .main_stack
                .visible_child_name()
                .is_none_or(|name| name == "disabled")
            {
                return false;
            }

            self.message_entry.grab_focus()
        }
    }

    impl BinImpl for MessageToolbar {}

    #[gtk::template_callbacks]
    impl MessageToolbar {
        /// Set the timeline used to send messages.
        fn set_timeline(&self, timeline: Option<&Timeline>) {
            let old_timeline = self.timeline.upgrade();
            if old_timeline.as_ref() == timeline {
                return;
            }
            let obj = self.obj();

            if let Some(timeline) = &old_timeline {
                if let Some(handler) = self.send_message_permission_handler.take() {
                    timeline.room().permissions().disconnect(handler);
                }
            }

            if let Some(timeline) = timeline {
                let send_message_permission_handler = timeline
                    .room()
                    .permissions()
                    .connect_can_send_message_notify(clone!(
                        #[weak(rename_to = imp)]
                        self,
                        move |_| {
                            imp.send_message_permission_updated();
                        }
                    ));
                self.send_message_permission_handler
                    .replace(Some(send_message_permission_handler));
            }

            self.completion.set_room(timeline.map(Timeline::room));
            self.timeline.set(timeline);

            self.send_message_permission_updated();

            obj.notify_timeline();
            self.update_current_composer_state(old_timeline);
        }

        /// Whether the user can compose a message.
        ///
        /// It depends on whether our own user has the permission to send a
        /// message in the current room.
        pub(super) fn can_compose_message(&self) -> bool {
            self.timeline
                .upgrade()
                .is_some_and(|timeline| timeline.room().permissions().can_send_message())
        }

        /// Handle an update of the permission to send a message in the current
        /// room.
        fn send_message_permission_updated(&self) {
            let page = if self.can_compose_message() {
                "enabled"
            } else {
                "disabled"
            };
            self.main_stack.set_visible_child_name(page);
        }

        /// Get the current composer state.
        fn current_composer_state(&self) -> ComposerState {
            let timeline = self.timeline.upgrade();
            self.composer_state(timeline)
        }

        /// Get the composer state for the given room.
        ///
        /// If the composer state doesn't exist, it is created.
        fn composer_state(&self, timeline: Option<Timeline>) -> ComposerState {
            let room = timeline.as_ref().map(Timeline::room);

            self.composer_states
                .borrow_mut()
                .entry(
                    room.as_ref()
                        .and_then(Room::session)
                        .map(|s| s.session_id().to_owned()),
                )
                .or_default()
                .entry(room.map(|room| room.room_id().to_owned()))
                .or_insert_with(|| ComposerState::new(timeline))
                .clone()
        }

        /// Update the current composer state.
        fn update_current_composer_state(&self, old_timeline: Option<Timeline>) {
            let old_composer_state = self.composer_state(old_timeline);
            old_composer_state.attach_to_view(None);

            if let Some(handler) = self.composer_state_handler.take() {
                old_composer_state.disconnect(handler);
            }
            if let Some((handler, binding)) = self.buffer_handlers.take() {
                let prev_buffer = self.message_entry.buffer();
                prev_buffer.disconnect(handler);

                binding.unbind();
            }

            let composer_state = self.current_composer_state();
            let buffer = composer_state.buffer();
            let obj = self.obj();

            composer_state.attach_to_view(Some(&self.message_entry));

            // Actions on changes in message entry.
            let text_notify_handler = buffer.connect_text_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |buffer| {
                    let (start_iter, end_iter) = buffer.bounds();
                    let is_empty = start_iter == end_iter;
                    imp.send_button.set_sensitive(!is_empty);
                    imp.send_typing_notification(!is_empty);
                }
            ));

            let (start_iter, end_iter) = buffer.bounds();
            let is_empty = start_iter == end_iter;
            self.send_button.set_sensitive(!is_empty);

            // Markdown highlighting.
            let markdown_binding = obj
                .bind_property("markdown-enabled", &buffer, "highlight-syntax")
                .sync_create()
                .build();

            self.buffer_handlers
                .replace(Some((text_notify_handler, markdown_binding)));

            // Related event.
            let composer_state_handler = composer_state.connect_related_to_changed(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.update_related_event();
                }
            ));
            self.composer_state_handler
                .replace(Some(composer_state_handler));
            self.update_related_event();

            obj.notify_current_composer_state();
        }

        /// Update the displayed related event for the current state.
        fn update_related_event(&self) {
            let composer_state = self.current_composer_state();

            match composer_state.related_to() {
                Some(RelationInfo::Reply(info)) => {
                    self.update_for_reply(&info);
                }
                Some(RelationInfo::Edit(_)) => {
                    self.update_for_edit();
                }
                None => {}
            }
        }

        /// Update the displayed related event for the given reply.
        fn update_for_reply(&self, info: &RepliedToInfo) {
            let Some(timeline) = self.timeline.upgrade() else {
                return;
            };

            let room = timeline.room();
            let sender = room
                .get_or_create_members()
                .get_or_create(info.sender().to_owned());

            let label = gettext_f(
                // Translators: Do NOT translate the content between '{' and '}',
                // this is a variable name. In this string, 'Reply' is a noun.
                "Reply to {user}",
                &[("user", LabelWithWidgets::PLACEHOLDER)],
            );
            let pill = sender.to_pill();

            self.related_event_header
                .set_label_and_widgets(label, vec![pill]);

            self.related_event_content
                .update_for_related_event(info, &sender);
            self.related_event_content.set_visible(true);
        }

        /// Update the displayed related event for the given edit.
        fn update_for_edit(&self) {
            // Translators: In this string, 'Edit' is a noun.
            let label = pgettext("room-history", "Edit");
            self.related_event_header
                .set_label_and_widgets::<gtk::Widget>(label, vec![]);

            self.related_event_content.set_visible(false);
        }

        /// Clear the related event.
        #[template_callback]
        fn clear_related_event(&self) {
            self.current_composer_state().set_related_to(None);
        }

        /// Add a mention of the given member to the message composer.
        pub(super) fn mention_member(&self, member: &Member) {
            if !self.can_compose_message() {
                return;
            }

            let buffer = self.message_entry.buffer();
            let mut insert = buffer.iter_at_mark(&buffer.get_insert());

            let pill = member.to_pill();
            self.current_composer_state().add_widget(pill, &mut insert);

            self.message_entry.grab_focus();
        }

        /// Set the event to reply to.
        pub(super) fn set_reply_to(&self, event: &Event) {
            if !self.can_compose_message() {
                return;
            }

            let Ok(info) = event.item().replied_to_info() else {
                warn!("Unsupported event type for reply");
                return;
            };

            self.current_composer_state()
                .set_related_to(Some(RelationInfo::Reply(info)));

            self.message_entry.grab_focus();
        }

        /// Set the event to edit.
        pub(super) fn set_edit(&self, event: &Event) {
            if !self.can_compose_message() {
                return;
            }

            let item = event.item();

            let Some(event_id) = item.event_id() else {
                warn!("Cannot send edit for event that is not sent yet");
                return;
            };
            let TimelineItemContent::Message(message) = item.content() else {
                warn!("Unsupported event type for edit");
                return;
            };

            self.current_composer_state()
                .set_edit_source(event_id.to_owned(), message);

            self.message_entry.grab_focus();
        }

        /// Handle when a key was pressed in the message entry.
        #[template_callback]
        fn key_pressed(
            &self,
            key: gdk::Key,
            _keycode: u32,
            modifier: gdk::ModifierType,
        ) -> glib::Propagation {
            if modifier.is_empty() && (key == gdk::Key::Return || key == gdk::Key::KP_Enter) {
                spawn!(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    async move {
                        imp.send_text_message().await;
                    }
                ));
                glib::Propagation::Stop
            } else if modifier.is_empty()
                && key == gdk::Key::Escape
                && self.current_composer_state().has_relation()
            {
                self.clear_related_event();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        }

        /// Send the text message that is currently in the message entry.
        #[template_callback]
        async fn send_text_message(&self) {
            let Some(_send_guard) = self.send_guard.try_lock() else {
                return;
            };
            if !self.can_compose_message() {
                return;
            }
            let Some(timeline) = self.timeline.upgrade() else {
                return;
            };

            let composer_state = self.current_composer_state();
            let markdown_enabled = self.markdown_enabled.get();

            let Some(content) = ComposerParser::new(&composer_state, None)
                .into_message_event_content(markdown_enabled)
                .await
            else {
                return;
            };

            let matrix_timeline = timeline.matrix_timeline();

            // Send event depending on relation.
            match composer_state.related_to() {
                Some(RelationInfo::Reply(replied_to_info)) => {
                    let handle = spawn_tokio!(async move {
                        matrix_timeline
                            .send_reply(content, replied_to_info, EnforceThread::MaybeThreaded)
                            .await
                    });
                    if let Err(error) = handle.await.unwrap() {
                        error!("Could not send reply: {error}");
                        let obj = self.obj();
                        toast!(obj, gettext("Could not send reply"));
                    }
                }
                Some(RelationInfo::Edit(event_id)) => {
                    let matrix_room = timeline.room().matrix_room().clone();
                    let handle = spawn_tokio!(async move {
                        let full_content = matrix_room
                            .make_edit_event(&event_id, EditedContent::RoomMessage(content))
                            .await
                            .map_err(matrix_sdk_ui::timeline::EditError::from)?;
                        let send_queue = matrix_room.send_queue();
                        send_queue.send(full_content).await?;
                        Ok::<(), matrix_sdk_ui::timeline::Error>(())
                    });
                    if let Err(error) = handle.await.unwrap() {
                        error!("Could not send edit: {error}");
                        let obj = self.obj();
                        toast!(obj, gettext("Could not send edit"));
                    }
                }
                _ => {
                    let handle = spawn_tokio!(async move {
                        matrix_timeline
                            .send(content.with_relation(None).into())
                            .await
                    });
                    if let Err(error) = handle.await.unwrap() {
                        error!("Could not send message: {error}");
                        let obj = self.obj();
                        toast!(obj, gettext("Could not send message"));
                    }
                }
            }

            // Clear the composer state.
            composer_state.clear();
        }

        /// Open the emoji chooser in the message entry.
        #[template_callback]
        fn open_emoji(&self) {
            if !self.can_compose_message() {
                return;
            }
            self.message_entry.emit_insert_emoji();
        }

        /// Send the current location of the user.
        ///
        /// Shows a preview of the location first and asks the user to confirm
        /// the action.
        async fn send_location(&self) {
            let Some(_send_guard) = self.send_guard.try_lock() else {
                return;
            };
            if !self.can_compose_message() {
                return;
            }
            let Some(timeline) = self.timeline.upgrade() else {
                return;
            };

            let location = Location::new();
            if !location.is_available() {
                return;
            }

            // Listen whether the user cancels before the location API is initialized.
            if let Err(error) = location.init().await {
                self.location_error_toast(error);
                return;
            }

            // Show the dialog as loading.
            let obj = self.obj();
            let dialog = AttachmentDialog::new(&gettext("Your Location"));
            let response_fut = dialog.response_future(&*obj);
            pin_mut!(response_fut);

            // Listen whether the user cancels before the location stream is ready.
            let location_stream_fut = location.updates_stream();
            pin_mut!(location_stream_fut);
            let (mut location_stream, response_fut) =
                match future::select(location_stream_fut, response_fut).await {
                    future::Either::Left((stream_res, response_fut)) => match stream_res {
                        Ok(stream) => (stream, response_fut),
                        Err(error) => {
                            dialog.close();
                            self.location_error_toast(error);
                            return;
                        }
                    },
                    future::Either::Right(_) => {
                        // The only possible response at this stage should be cancel.
                        return;
                    }
                };

            // Listen to location changes while waiting for the user's response.
            let mut response_fut_wrapper = Some(response_fut);
            let mut geo_uri_wrapper = None;
            loop {
                let response_fut = response_fut_wrapper.take().unwrap();

                match future::select(location_stream.next(), response_fut).await {
                    future::Either::Left((update, response_fut)) => {
                        if let Some(uri) = update {
                            dialog.set_location(&uri);
                            geo_uri_wrapper.replace(uri);
                        }
                        response_fut_wrapper.replace(response_fut);
                    }
                    future::Either::Right((response, _)) => {
                        // The linux location stream requires a tokio executor when dropped.
                        let stream_drop = TokioDrop::new();
                        let _ = stream_drop.set(location_stream);

                        if response == gtk::ResponseType::Ok {
                            break;
                        }

                        return;
                    }
                }
            }

            let Some(geo_uri) = geo_uri_wrapper else {
                return;
            };

            let geo_uri_string = geo_uri.to_string();
            let timestamp =
                glib::DateTime::now_local().expect("Should be able to get the local timestamp");
            let location_body = gettext_f(
                // Translators: Do NOT translate the content between '{' and '}', this is a
                // variable name.
                "User Location {geo_uri} at {iso8601_datetime}",
                &[
                    ("geo_uri", &geo_uri_string),
                    (
                        "iso8601_datetime",
                        timestamp.format_iso8601().unwrap().as_str(),
                    ),
                ],
            );

            let content = RoomMessageEventContent::new(MessageType::Location(
                LocationMessageEventContent::new(location_body, geo_uri_string),
            ))
            // To avoid triggering legacy pushrules, we must always include the mentions,
            // even if they are empty.
            .add_mentions(Mentions::default());

            let matrix_timeline = timeline.matrix_timeline();
            let handle = spawn_tokio!(async move { matrix_timeline.send(content.into()).await });

            if let Err(error) = handle.await.unwrap() {
                error!("Could not send location: {error}");
                let obj = self.obj();
                toast!(obj, gettext("Could not send location"));
            }
        }

        /// Show a toast for the given location error;
        fn location_error_toast(&self, error: LocationError) {
            let msg = match error {
                LocationError::Cancelled => gettext("The location request has been cancelled"),
                LocationError::Disabled => gettext("The location services are disabled"),
                LocationError::Other => gettext("Could not retrieve current location"),
            };

            let obj = self.obj();
            toast!(obj, msg);
        }

        /// Send the attachment with the given data.
        async fn send_attachment(
            &self,
            source: AttachmentSource,
            mime: mime::Mime,
            info: AttachmentInfo,
            thumbnail: Option<Thumbnail>,
        ) {
            let Some(timeline) = self.timeline.upgrade() else {
                return;
            };

            let config = AttachmentConfig::new().thumbnail(thumbnail).info(info);

            let matrix_timeline = timeline.matrix_timeline();

            let handle = spawn_tokio!(async move {
                matrix_timeline
                    .send_attachment(source, mime, config)
                    .use_send_queue()
                    .await
            });

            if let Err(error) = handle.await.unwrap() {
                error!("Could not send file: {error}");
                let obj = self.obj();
                toast!(obj, gettext("Could not send file"));
            }
        }

        /// Send the given texture as an image.
        ///
        /// Shows a preview of the image first and asks the user to confirm the
        /// action.
        async fn send_image(&self, image: gdk::Texture) {
            let Some(_send_guard) = self.send_guard.try_lock() else {
                return;
            };
            if !self.can_compose_message() {
                return;
            }

            let obj = self.obj();
            let filename = filename_for_mime(Some(mime::IMAGE_PNG.as_ref()), None);
            let dialog = AttachmentDialog::new(&filename);
            dialog.set_image(&image);

            if dialog.response_future(&*obj).await != gtk::ResponseType::Ok {
                return;
            }

            let bytes = image.save_to_png_bytes();
            let filesize = bytes.len().try_into().ok();

            let (mut base_info, thumbnail) = ImageInfoLoader::from(image)
                .load_info_and_thumbnail(filesize, &*obj)
                .await;
            base_info.size = filesize.map(Into::into);

            let info = AttachmentInfo::Image(base_info);
            let source = AttachmentSource::Data {
                bytes: bytes.to_vec(),
                filename,
            };
            self.send_attachment(source, mime::IMAGE_PNG, info, thumbnail)
                .await;
        }

        /// Select a file to send.
        #[template_callback]
        async fn select_file(&self) {
            let Some(_send_guard) = self.send_guard.try_lock() else {
                return;
            };
            if !self.can_compose_message() {
                return;
            }

            let obj = self.obj();
            let dialog = gtk::FileDialog::builder()
                .title(gettext("Select File"))
                .modal(true)
                .accept_label(gettext("Select"))
                .build();

            match dialog
                .open_future(obj.root().and_downcast_ref::<gtk::Window>())
                .await
            {
                Ok(file) => {
                    self.send_file_inner(file).await;
                }
                Err(error) => {
                    if error.matches(gtk::DialogError::Dismissed) {
                        debug!("File dialog dismissed by user");
                    } else {
                        error!("Could not open file: {error:?}");
                        toast!(obj, gettext("Could not open file"));
                    }
                }
            };
        }

        /// Send the given file.
        ///
        /// Shows a preview of the file first, if possible, and asks the user to
        /// confirm the action.
        pub(super) async fn send_file(&self, file: gio::File) {
            let Some(_send_guard) = self.send_guard.try_lock() else {
                return;
            };
            if !self.can_compose_message() {
                return;
            }

            self.send_file_inner(file).await;
        }

        async fn send_file_inner(&self, file: gio::File) {
            let obj = self.obj();

            let Some(path) = file.path() else {
                warn!("Could not read file: file does not have a path");
                toast!(obj, gettext("Error reading file"));
                return;
            };

            let file_info = match FileInfo::try_from_file(&file).await {
                Ok(file_info) => file_info,
                Err(error) => {
                    warn!("Could not read file info: {error}");
                    toast!(obj, gettext("Error reading file"));
                    return;
                }
            };

            let dialog = AttachmentDialog::new(&file_info.filename);
            dialog.set_file(file.clone());

            if dialog.response_future(&*obj).await != gtk::ResponseType::Ok {
                return;
            }

            let size = file_info.size.map(Into::into);
            let (info, thumbnail) = match file_info.mime.type_() {
                mime::IMAGE => {
                    let (mut info, thumbnail) = ImageInfoLoader::from(file)
                        .load_info_and_thumbnail(file_info.size, &*obj)
                        .await;
                    info.size = size;

                    (AttachmentInfo::Image(info), thumbnail)
                }
                mime::VIDEO => {
                    let (mut info, thumbnail) = load_video_info(&file, &*obj).await;
                    info.size = size;
                    (AttachmentInfo::Video(info), thumbnail)
                }
                mime::AUDIO => {
                    let mut info = load_audio_info(&file).await;
                    info.size = size;
                    (AttachmentInfo::Audio(info), None)
                }
                _ => (AttachmentInfo::File(BaseFileInfo { size }), None),
            };

            self.send_attachment(path.into(), file_info.mime, info, thumbnail)
                .await;
        }

        /// Read the file data from the clipboard and send it.
        pub(super) async fn read_clipboard_file(&self) {
            let obj = self.obj();
            let clipboard = obj.clipboard();
            let formats = clipboard.formats();

            if formats.contains_type(gdk::Texture::static_type()) {
                // There is an image in the clipboard.
                match clipboard
                    .read_value_future(gdk::Texture::static_type(), glib::Priority::DEFAULT)
                    .await
                {
                    Ok(value) => match value.get::<gdk::Texture>() {
                        Ok(texture) => {
                            self.send_image(texture).await;
                            return;
                        }
                        Err(error) => warn!("Could not get GdkTexture from value: {error}"),
                    },
                    Err(error) => warn!("Could not get GdkTexture from the clipboard: {error}"),
                }

                toast!(obj, gettext("Error getting image from clipboard"));
            } else if formats.contains_type(gio::File::static_type()) {
                // There is a file in the clipboard.
                match clipboard
                    .read_value_future(gio::File::static_type(), glib::Priority::DEFAULT)
                    .await
                {
                    Ok(value) => match value.get::<gio::File>() {
                        Ok(file) => {
                            self.send_file(file).await;
                            return;
                        }
                        Err(error) => warn!("Could not get file from value: {error}"),
                    },
                    Err(error) => warn!("Could not get file from the clipboard: {error}"),
                }

                toast!(obj, gettext("Error getting file from clipboard"));
            }
        }

        /// Handle a click on the related event.
        ///
        /// Scrolls to the corresponding event.
        #[template_callback]
        fn handle_related_event_click(&self) {
            if let Some(related_to) = self.current_composer_state().related_to() {
                self.obj()
                    .activate_action(
                        "room-history.scroll-to-event",
                        Some(&related_to.identifier().to_variant()),
                    )
                    .expect("action exists");
            }
        }

        /// Paste the content of the clipboard into the message entry.
        #[template_callback]
        fn paste_from_clipboard(&self) {
            if !self.can_compose_message() {
                return;
            }

            let formats = self.obj().clipboard().formats();

            // We only handle files and supported images.
            if formats.contains_type(gio::File::static_type())
                || formats.contains_type(gdk::Texture::static_type())
            {
                self.message_entry
                    .stop_signal_emission_by_name("paste-clipboard");
                spawn!(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    async move {
                        imp.read_clipboard_file().await;
                    }
                ));
            }
        }

        /// Copy the content of the message entry to the clipboard.
        #[template_callback]
        fn copy_to_clipboard(&self) {
            self.message_entry
                .stop_signal_emission_by_name("copy-clipboard");
            self.copy_buffer_selection_to_clipboard();
        }

        /// Cut the content of the message entry to the clipboard.
        #[template_callback]
        fn cut_to_clipboard(&self) {
            self.message_entry
                .stop_signal_emission_by_name("cut-clipboard");
            self.copy_buffer_selection_to_clipboard();
            self.message_entry.buffer().delete_selection(true, true);
        }

        // Copy the selection in the message entry to the clipboard while replacing
        // mentions.
        fn copy_buffer_selection_to_clipboard(&self) {
            let buffer = self.message_entry.buffer();
            let Some((start, end)) = buffer.selection_bounds() else {
                return;
            };

            let composer_state = self.current_composer_state();
            let body = ComposerParser::new(&composer_state, Some((start, end))).into_plain_text();

            self.obj().clipboard().set_text(&body);
        }

        /// Send a typing notification for the given typing state.
        fn send_typing_notification(&self, typing: bool) {
            let Some(timeline) = self.timeline.upgrade() else {
                return;
            };
            let room = timeline.room();

            let Some(session) = room.session() else {
                return;
            };

            if !session.settings().typing_enabled() {
                return;
            }

            room.send_typing_notification(typing);
        }
    }
}

glib::wrapper! {
    /// A toolbar with different actions to send messages.
    pub struct MessageToolbar(ObjectSubclass<imp::MessageToolbar>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl MessageToolbar {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Add a mention of the given member to the message composer.
    pub(crate) fn mention_member(&self, member: &Member) {
        self.imp().mention_member(member);
    }

    /// Set the event to reply to.
    pub(crate) fn set_reply_to(&self, event: &Event) {
        self.imp().set_reply_to(event);
    }

    /// Set the event to edit.
    pub(crate) fn set_edit(&self, event: &Event) {
        self.imp().set_edit(event);
    }

    /// Send the given file.
    ///
    /// Shows a preview of the file first, if possible, and asks the user to
    /// confirm the action.
    pub(crate) async fn send_file(&self, file: gio::File) {
        self.imp().send_file(file).await;
    }

    /// Handle a paste action.
    pub(crate) fn handle_paste_action(&self) {
        let imp = self.imp();

        if !imp.can_compose_message() {
            return;
        }

        spawn!(clone!(
            #[weak]
            imp,
            async move {
                imp.read_clipboard_file().await;
            }
        ));
    }
}
