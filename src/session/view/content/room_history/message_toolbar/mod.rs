use std::{collections::HashMap, fmt::Write, io::Cursor};

use adw::{prelude::*, subclass::prelude::*};
use futures_util::{future, pin_mut, StreamExt};
use gettextrs::{gettext, pgettext};
use gtk::{
    gdk, gio,
    glib::{self, clone},
    CompositeTemplate,
};
use image::ImageFormat;
use matrix_sdk::{
    attachment::{
        generate_image_thumbnail, AttachmentConfig, AttachmentInfo, BaseFileInfo, BaseImageInfo,
        ThumbnailFormat,
    },
    room::edit::EditedContent,
};
use matrix_sdk_ui::timeline::{RepliedToInfo, TimelineItemContent};
use ruma::{
    events::{
        room::message::{
            EmoteMessageEventContent, FormattedBody, ForwardThread, LocationMessageEventContent,
            MessageType, RoomMessageEventContent, RoomMessageEventContentWithoutRelation,
        },
        Mentions,
    },
    OwnedRoomId, OwnedUserId,
};
use tracing::{debug, error, warn};

mod attachment_dialog;
mod completion;
mod composer_state;

pub use self::composer_state::{ComposerState, RelationInfo};
use self::{attachment_dialog::AttachmentDialog, completion::CompletionPopover};
use super::message_row::MessageContent;
use crate::{
    components::{AtRoom, CustomEntry, LabelWithWidgets, Pill, PillSource},
    gettext_f,
    prelude::*,
    session::model::{Event, Member, Room},
    spawn, spawn_tokio, toast,
    utils::{
        matrix::AT_ROOM,
        media::{filename_for_mime, get_audio_info, get_image_info, get_video_info, load_file},
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
        /// The room to send messages in.
        #[property(get, set = Self::set_room, explicit_notify, nullable)]
        pub room: glib::WeakRef<Room>,
        pub can_send_message_handler: RefCell<Option<glib::SignalHandlerId>>,
        /// Whether outgoing messages should be interpreted as markdown.
        #[property(get, set)]
        pub markdown_enabled: Cell<bool>,
        pub completion: CompletionPopover,
        #[template_child]
        pub main_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub message_entry: TemplateChild<sourceview::View>,
        #[template_child]
        pub related_event_header: TemplateChild<LabelWithWidgets>,
        #[template_child]
        pub related_event_content: TemplateChild<MessageContent>,
        /// The current composer state.
        #[property(get = Self::current_composer_state)]
        pub current_composer_state: PhantomData<ComposerState>,
        composer_state_handler: RefCell<Option<glib::SignalHandlerId>>,
        buffer_handlers: RefCell<Option<(glib::SignalHandlerId, glib::Binding)>>,
        /// The composer states, per-session and per-room.
        ///
        /// The fallback composer state has the `None` key.
        pub composer_states: RefCell<ComposerStatesMap>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MessageToolbar {
        const NAME: &'static str = "MessageToolbar";
        type Type = super::MessageToolbar;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            CustomEntry::ensure_type();

            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);
            TemplateCallbacks::bind_template_callbacks(klass);

            klass.install_action_async(
                "message-toolbar.send-text-message",
                None,
                |widget, _, _| async move {
                    widget.send_text_message().await;
                },
            );

            klass.install_action_async(
                "message-toolbar.select-file",
                None,
                |widget, _, _| async move {
                    widget.select_file().await;
                },
            );

            klass.install_action("message-toolbar.open-emoji", None, |widget, _, _| {
                widget.open_emoji();
            });

            klass.install_action_async(
                "message-toolbar.send-location",
                None,
                |widget, _, _| async move {
                    widget.send_location().await;
                },
            );

            klass.install_property_action("message-toolbar.markdown", "markdown-enabled");

            klass.install_action("message-toolbar.clear-related-event", None, |obj, _, _| {
                obj.current_composer_state().set_related_to(None);
            });
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

            // Clipboard.
            self.message_entry.connect_paste_clipboard(clone!(
                #[weak]
                obj,
                move |entry| {
                    if !obj.imp().can_send_message() {
                        return;
                    }

                    let formats = obj.clipboard().formats();

                    // We only handle files and supported images.
                    if formats.contains_type(gio::File::static_type())
                        || formats.contains_type(gdk::Texture::static_type())
                    {
                        entry.stop_signal_emission_by_name("paste-clipboard");
                        spawn!(async move {
                            obj.read_clipboard_file().await;
                        });
                    }
                }
            ));
            self.message_entry.connect_copy_clipboard(clone!(
                #[weak]
                obj,
                move |entry| {
                    entry.stop_signal_emission_by_name("copy-clipboard");

                    spawn!(async move {
                        obj.copy_buffer_selection_to_clipboard().await;
                    });
                }
            ));
            self.message_entry.connect_cut_clipboard(clone!(
                #[weak]
                obj,
                move |entry| {
                    entry.stop_signal_emission_by_name("cut-clipboard");

                    spawn!(clone!(
                        #[weak]
                        entry,
                        async move {
                            obj.copy_buffer_selection_to_clipboard().await;
                            entry.buffer().delete_selection(true, true);
                        }
                    ));
                }
            ));

            // Key bindings.
            let key_events = gtk::EventControllerKey::new();
            key_events.connect_key_pressed(clone!(
                #[weak]
                obj,
                #[upgrade_or]
                glib::Propagation::Proceed,
                move |_, key, _, modifier| {
                    if modifier.is_empty() && (key == gdk::Key::Return || key == gdk::Key::KP_Enter)
                    {
                        spawn!(async move {
                            obj.send_text_message().await;
                        });
                        glib::Propagation::Stop
                    } else if modifier.is_empty()
                        && key == gdk::Key::Escape
                        && obj.current_composer_state().has_relation()
                    {
                        obj.current_composer_state().set_related_to(None);
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                }
            ));
            self.message_entry.add_controller(key_events);

            // Markdown highlighting.
            let settings = Application::default().settings();
            settings
                .bind("markdown-enabled", &*obj, "markdown-enabled")
                .build();

            // Tab auto-completion.
            self.completion.set_parent(&*self.message_entry);
            obj.bind_property("room", &self.completion, "room")
                .sync_create()
                .build();

            // Location.
            let location = Location::new();
            obj.action_set_enabled("message-toolbar.send-location", location.is_available());
        }

        fn dispose(&self) {
            self.completion.unparent();

            if let Some(room) = self.room.upgrade() {
                if let Some(handler) = self.can_send_message_handler.take() {
                    room.permissions().disconnect(handler);
                }
            }
        }
    }

    impl WidgetImpl for MessageToolbar {}
    impl BinImpl for MessageToolbar {}

    impl MessageToolbar {
        /// Set the room currently displayed.
        fn set_room(&self, room: Option<Room>) {
            let old_room = self.room.upgrade();
            if old_room == room {
                return;
            }
            let obj = self.obj();

            if let Some(room) = &old_room {
                if let Some(handler) = self.can_send_message_handler.take() {
                    room.permissions().disconnect(handler);
                }
            }

            if let Some(room) = &room {
                let can_send_message_handler =
                    room.permissions().connect_can_send_message_notify(clone!(
                        #[weak(rename_to = imp)]
                        self,
                        move |_| {
                            imp.can_send_message_updated();
                        }
                    ));
                self.can_send_message_handler
                    .replace(Some(can_send_message_handler));
            }

            self.room.set(room.as_ref());

            self.can_send_message_updated();
            self.message_entry.grab_focus();

            obj.notify_room();
            self.update_current_composer_state(old_room.as_ref());
        }

        /// Whether our own user can send a message in the current room.
        pub(super) fn can_send_message(&self) -> bool {
            self.room
                .upgrade()
                .is_some_and(|r| r.permissions().can_send_message())
        }

        /// Update whether our own user can send a message in the current room.
        fn can_send_message_updated(&self) {
            let page = if self.can_send_message() {
                "enabled"
            } else {
                "disabled"
            };
            self.main_stack.set_visible_child_name(page);
        }

        /// Get the current composer state.
        fn current_composer_state(&self) -> ComposerState {
            let room = self.room.upgrade();
            self.composer_state(room.as_ref())
        }

        /// Get the composer state for the given room.
        ///
        /// If the composer state doesn't exist, it is created.
        fn composer_state(&self, room: Option<&Room>) -> ComposerState {
            self.composer_states
                .borrow_mut()
                .entry(
                    room.and_then(|r| r.session())
                        .map(|s| s.session_id().to_owned()),
                )
                .or_default()
                .entry(room.map(|r| r.room_id().to_owned()))
                .or_insert_with(|| ComposerState::new(room))
                .clone()
        }

        /// Update the current composer state.
        fn update_current_composer_state(&self, old_room: Option<&Room>) {
            let old_composer_state = self.composer_state(old_room);
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
                #[weak]
                obj,
                move |buffer| {
                    let (start_iter, end_iter) = buffer.bounds();
                    let is_empty = start_iter == end_iter;
                    obj.action_set_enabled("message-toolbar.send-text-message", !is_empty);
                    obj.send_typing_notification(!is_empty);
                }
            ));

            let (start_iter, end_iter) = buffer.bounds();
            obj.action_set_enabled("message-toolbar.send-text-message", start_iter != end_iter);

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
                    self.update_for_reply(info);
                }
                Some(RelationInfo::Edit(_)) => {
                    self.update_for_edit();
                }
                None => {}
            }
        }

        /// Update the displayed related event for the given reply.
        fn update_for_reply(&self, info: RepliedToInfo) {
            let Some(room) = self.room.upgrade() else {
                return;
            };

            let sender = room
                .get_or_create_members()
                .get_or_create(info.sender().to_owned());

            self.related_event_header
                .set_widgets(vec![Pill::new(&sender)]);
            self.related_event_header
                // Translators: Do NOT translate the content between '{' and '}',
                // this is a variable name. In this string, 'Reply' is a noun.
                .set_label(Some(gettext_f("Reply to {user}", &[("user", "<widget>")])));

            self.related_event_content
                .update_for_related_event(info, sender);
            self.related_event_content.set_visible(true);
        }

        /// Update the displayed related event for the given edit.
        fn update_for_edit(&self) {
            self.related_event_header.set_widgets::<gtk::Widget>(vec![]);
            self.related_event_header
                // Translators: In this string, 'Edit' is a noun.
                .set_label(Some(pgettext("room-history", "Edit")));

            self.related_event_content.set_visible(false);
        }
    }
}

glib::wrapper! {
    /// A toolbar with different actions to send messages.
    pub struct MessageToolbar(ObjectSubclass<imp::MessageToolbar>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl MessageToolbar {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Add a mention of the given member to the message composer.
    pub fn mention_member(&self, member: &Member) {
        let view = &*self.imp().message_entry;
        let buffer = view.buffer();

        let mut insert = buffer.iter_at_mark(&buffer.get_insert());

        let pill = member.to_pill();
        self.current_composer_state().add_widget(pill, &mut insert);

        view.grab_focus();
    }

    /// Set the event to reply to.
    pub fn set_reply_to(&self, event: Event) {
        let imp = self.imp();
        if !imp.can_send_message() {
            return;
        }

        let Ok(info) = event.item().replied_to_info() else {
            warn!("Unsupported event type for reply");
            return;
        };

        self.current_composer_state()
            .set_related_to(Some(RelationInfo::Reply(info)));

        imp.message_entry.grab_focus();
    }

    /// Set the event to edit.
    pub fn set_edit(&self, event: Event) {
        let imp = self.imp();
        if !imp.can_send_message() {
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

        imp.message_entry.grab_focus();
    }

    /// Send the text message that is currently in the message entry.
    async fn send_text_message(&self) {
        let imp = self.imp();
        if !imp.can_send_message() {
            return;
        }
        let Some(room) = self.room() else {
            return;
        };

        let composer_state = self.current_composer_state();
        let buffer = composer_state.buffer();
        let (start_iter, end_iter) = buffer.bounds();
        let body_len = end_iter.offset() as usize;

        let is_markdown = self.markdown_enabled();
        let mut has_rich_mentions = false;
        let mut plain_body = String::with_capacity(body_len);
        // formatted_body is Markdown if is_markdown is true, and HTML if false.
        let mut formatted_body = String::with_capacity(body_len);
        let mut mentions = Mentions::new();

        let split_message = MessageBufferParser::new(&composer_state, start_iter, end_iter);
        for chunk in split_message {
            match chunk {
                MessageBufferChunk::Text(text) => {
                    plain_body.push_str(&text);
                    formatted_body.push_str(&text);
                }
                MessageBufferChunk::Mention(source) => match Mention::from_source(&source).await {
                    Mention::Rich { name, uri, user_id } => {
                        has_rich_mentions = true;
                        plain_body.push_str(&name);
                        if is_markdown {
                            let _ = write!(formatted_body, "[{name}]({uri})");
                        } else {
                            let _ = write!(formatted_body, "<a href=\"{uri}\">{name}</a>");
                        };

                        if let Some(user_id) = user_id {
                            mentions.user_ids.insert(user_id);
                        }
                    }
                    Mention::AtRoom => {
                        plain_body.push_str(AT_ROOM);
                        formatted_body.push_str(AT_ROOM);

                        mentions.room = true;
                    }
                },
            }
        }

        let is_emote = plain_body.starts_with("/me ");
        if is_emote {
            plain_body.replace_range(.."/me ".len(), "");
            formatted_body.replace_range(.."/me ".len(), "");
        }

        if plain_body.trim().is_empty() {
            // Don't send empty message.
            return;
        }

        let html_body = if is_markdown {
            FormattedBody::markdown(formatted_body).map(|b| b.body)
        } else if has_rich_mentions {
            // Already formatted with HTML.
            Some(formatted_body)
        } else {
            None
        };

        let mut content = if is_emote {
            MessageType::Emote(if let Some(html_body) = html_body {
                EmoteMessageEventContent::html(plain_body, html_body)
            } else {
                EmoteMessageEventContent::plain(plain_body)
            })
            .into()
        } else if let Some(html_body) = html_body {
            RoomMessageEventContentWithoutRelation::text_html(plain_body, html_body)
        } else {
            RoomMessageEventContentWithoutRelation::text_plain(plain_body)
        };

        // To avoid triggering legacy pushrules, we must always include the mentions,
        // even if they are empty.
        content = content.add_mentions(mentions);

        let matrix_timeline = room.timeline().matrix_timeline();

        // Send event depending on relation.
        match composer_state.related_to() {
            Some(RelationInfo::Reply(replied_to_info)) => {
                let handle = spawn_tokio!(async move {
                    matrix_timeline
                        .send_reply(content, replied_to_info, ForwardThread::Yes)
                        .await
                });
                if let Err(error) = handle.await.unwrap() {
                    error!("Could not send reply: {error}");
                    toast!(self, gettext("Could not send reply"));
                }
            }
            Some(RelationInfo::Edit(event_id)) => {
                let matrix_room = room.matrix_room().clone();
                let handle = spawn_tokio!(async move {
                    let full_content = matrix_room
                        .make_edit_event(&event_id, EditedContent::RoomMessage(content))
                        .await?;
                    let send_queue = matrix_room.send_queue();
                    send_queue.send(full_content).await?;
                    Ok::<(), matrix_sdk_ui::timeline::Error>(())
                });
                if let Err(error) = handle.await.unwrap() {
                    error!("Could not send edit: {error}");
                    toast!(self, gettext("Could not send edit"));
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
                    toast!(self, gettext("Could not send message"));
                }
            }
        }

        // Clear the composer state.
        composer_state.clear();
    }

    /// Open the emoji chooser in the message entry.
    fn open_emoji(&self) {
        let imp = self.imp();
        if !imp.can_send_message() {
            return;
        }
        imp.message_entry.emit_insert_emoji();
    }

    /// Send the current location of the user.
    ///
    /// Shows a preview of the location first and asks the user to confirm the
    /// action.
    async fn send_location(&self) {
        if !self.imp().can_send_message() {
            return;
        }
        let Some(room) = self.room() else {
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
        let dialog = AttachmentDialog::new(&gettext("Your Location"));
        let response_fut = dialog.response_future(self);
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
                    } else {
                        return;
                    }
                }
            };
        }

        let Some(geo_uri) = geo_uri_wrapper else {
            return;
        };

        let geo_uri_string = geo_uri.to_string();
        let timestamp =
            glib::DateTime::now_local().expect("Should be able to get the local timestamp");
        let location_body = gettext_f(
            // Translators: Do NOT translate the content between '{' and '}', this is a variable
            // name.
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

        let matrix_timeline = room.timeline().matrix_timeline();
        let handle = spawn_tokio!(async move { matrix_timeline.send(content.into()).await });

        if let Err(error) = handle.await.unwrap() {
            error!("Could not send location: {error}");
            toast!(self, gettext("Could not send location"))
        }
    }

    /// Show a toast for the given location error;
    fn location_error_toast(&self, error: LocationError) {
        let msg = match error {
            LocationError::Cancelled => gettext("The location request has been cancelled"),
            LocationError::Disabled => gettext("The location services are disabled"),
            LocationError::Other => gettext("Could not retrieve current location"),
        };

        toast!(self, msg);
    }

    /// Send the attachment with the given data.
    async fn send_attachment(
        &self,
        bytes: Vec<u8>,
        mime: mime::Mime,
        body: String,
        info: AttachmentInfo,
    ) {
        let Some(room) = self.room() else {
            return;
        };

        let matrix_room = room.matrix_room().clone();

        let handle = spawn_tokio!(async move {
            // The method will filter compatible mime types so we don't need to,
            // since we ignore errors.
            let thumbnail = generate_image_thumbnail(
                &mime,
                Cursor::new(&bytes),
                None,
                ThumbnailFormat::Fallback(ImageFormat::Jpeg),
            )
            .ok();

            let config = if let Some(thumbnail) = thumbnail {
                AttachmentConfig::with_thumbnail(thumbnail)
            } else {
                AttachmentConfig::new()
            }
            .info(info);

            matrix_room
                .send_attachment(&body, &mime, bytes, config)
                .await
        });

        if let Err(error) = handle.await.unwrap() {
            error!("Could not send file: {error}");
            toast!(self, gettext("Could not send file"));
        }
    }

    /// Send the given texture as an image.
    ///
    /// Shows a preview of the image first and asks the user to confirm the
    /// action.
    async fn send_image(&self, image: gdk::Texture) {
        if !self.imp().can_send_message() {
            return;
        }

        let filename = filename_for_mime(Some(mime::IMAGE_PNG.as_ref()), None);
        let dialog = AttachmentDialog::new(&filename);
        dialog.set_image(&image);

        if dialog.response_future(self).await != gtk::ResponseType::Ok {
            return;
        }

        let bytes = image.save_to_png_bytes();
        let info = AttachmentInfo::Image(BaseImageInfo {
            width: Some((image.width() as u32).into()),
            height: Some((image.height() as u32).into()),
            size: Some((bytes.len() as u32).into()),
            blurhash: None,
        });

        self.send_attachment(bytes.to_vec(), mime::IMAGE_PNG, filename, info)
            .await;
    }

    /// Select a file to send.
    pub async fn select_file(&self) {
        if !self.imp().can_send_message() {
            return;
        }

        let dialog = gtk::FileDialog::builder()
            .title(gettext("Select File"))
            .modal(true)
            .accept_label(gettext("Select"))
            .build();

        match dialog
            .open_future(self.root().and_downcast_ref::<gtk::Window>())
            .await
        {
            Ok(file) => {
                self.send_file(file).await;
            }
            Err(error) => {
                if error.matches(gtk::DialogError::Dismissed) {
                    debug!("File dialog dismissed by user");
                } else {
                    error!("Could not open file: {error:?}");
                    toast!(self, gettext("Could not open file"));
                }
            }
        };
    }

    /// Send the given file.
    ///
    /// Shows a preview of the file first, if possible, and asks the user to
    /// confirm the action.
    pub async fn send_file(&self, file: gio::File) {
        if !self.imp().can_send_message() {
            return;
        }

        let (bytes, file_info) = match load_file(&file).await {
            Ok(data) => data,
            Err(error) => {
                warn!("Could not read file: {error}");
                toast!(self, gettext("Error reading file"));
                return;
            }
        };

        let dialog = AttachmentDialog::new(&file_info.filename);
        dialog.set_file(&file);

        if dialog.response_future(self).await != gtk::ResponseType::Ok {
            return;
        }

        let size = file_info.size.map(Into::into);
        let info = match file_info.mime.type_() {
            mime::IMAGE => {
                let mut info = get_image_info(file).await;
                info.size = size;
                AttachmentInfo::Image(info)
            }
            mime::VIDEO => {
                let mut info = get_video_info(&file).await;
                info.size = size;
                AttachmentInfo::Video(info)
            }
            mime::AUDIO => {
                let mut info = get_audio_info(&file).await;
                info.size = size;
                AttachmentInfo::Audio(info)
            }
            _ => AttachmentInfo::File(BaseFileInfo { size }),
        };

        self.send_attachment(bytes, file_info.mime, file_info.filename, info)
            .await;
    }

    /// Read the file data from the clipboard and send it.
    async fn read_clipboard_file(&self) {
        let clipboard = self.clipboard();
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
                    Err(error) => warn!("Could not get GdkTexture from value: {error:?}"),
                },
                Err(error) => warn!("Could not get GdkTexture from the clipboard: {error:?}"),
            }

            toast!(self, gettext("Error getting image from clipboard"));
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
                    Err(error) => warn!("Could not get file from value: {error:?}"),
                },
                Err(error) => warn!("Could not get file from the clipboard: {error:?}"),
            }

            toast!(self, gettext("Error getting file from clipboard"));
        }
    }

    /// Handle a click on the related event.
    ///
    /// Scrolls to the corresponding event.
    #[template_callback]
    fn handle_related_event_click(&self) {
        if let Some(related_to) = self.current_composer_state().related_to() {
            self.activate_action(
                "room-history.scroll-to-event",
                Some(&related_to.key().to_variant()),
            )
            .unwrap();
        }
    }

    /// Handle a paste action.
    pub fn handle_paste_action(&self) {
        if !self.imp().can_send_message() {
            return;
        }

        spawn!(clone!(
            #[weak(rename_to = obj)]
            self,
            async move {
                obj.read_clipboard_file().await;
            }
        ));
    }

    // Copy the selection in the message entry to the clipboard while replacing
    // mentions.
    async fn copy_buffer_selection_to_clipboard(&self) {
        let buffer = self.imp().message_entry.buffer();
        let Some((start, end)) = buffer.selection_bounds() else {
            return;
        };

        let composer_state = self.current_composer_state();
        let body_len = end.offset().saturating_sub(start.offset()) as usize;
        let mut body = String::with_capacity(body_len);

        let split_message = MessageBufferParser::new(&composer_state, start, end);
        for chunk in split_message {
            match chunk {
                MessageBufferChunk::Text(text) => {
                    body.push_str(&text);
                }
                MessageBufferChunk::Mention(source) => {
                    if let Some(user) = source.downcast_ref::<Member>() {
                        body.push_str(&user.display_name());
                    } else if let Some(room) = source.downcast_ref::<Room>() {
                        body.push_str(
                            room.aliases()
                                .alias()
                                .as_ref()
                                .map(AsRef::as_ref)
                                .unwrap_or_else(|| room.room_id().as_ref()),
                        );
                    } else if source.is::<AtRoom>() {
                        body.push_str(AT_ROOM);
                    } else {
                        unreachable!()
                    }
                }
            }
        }

        self.clipboard().set_text(&body);
    }

    /// Send a typing notification for the given typing state.
    fn send_typing_notification(&self, typing: bool) {
        let Some(room) = self.room() else {
            return;
        };
        let Some(session) = room.session() else {
            return;
        };

        if !session.settings().typing_enabled() {
            return;
        }

        room.send_typing_notification(typing);
    }
}

/// A chunk of a message in a buffer.
enum MessageBufferChunk {
    /// Some text.
    Text(String),
    /// A mention as a `Pill`.
    Mention(PillSource),
}

/// A mention that can be sent in a message.
enum Mention {
    /// A mention that has a HTML representation.
    Rich {
        /// The string representation of the mention.
        name: String,
        /// The URI of the mention.
        uri: String,
        /// The user ID, if this is a user mention.
        user_id: Option<OwnedUserId>,
    },
    /// An `@room` mention.
    AtRoom,
}

impl Mention {
    /// Construct a `Mention` from the given pill source.
    async fn from_source(source: &PillSource) -> Self {
        if let Some(user) = source.downcast_ref::<Member>() {
            Self::Rich {
                name: user.display_name(),
                uri: user.matrix_to_uri().to_string(),
                user_id: Some(user.user_id().clone()),
            }
        } else if let Some(room) = source.downcast_ref::<Room>() {
            let matrix_to_uri = room.matrix_to_uri().await;
            let string_repr = room
                .aliases()
                .alias_string()
                .unwrap_or_else(|| room.room_id_string());

            Self::Rich {
                name: string_repr,
                uri: matrix_to_uri.to_string(),
                user_id: None,
            }
        } else if source.is::<AtRoom>() {
            Self::AtRoom
        } else {
            unreachable!()
        }
    }
}

/// An iterator over the chunks of a message in a `GtkTextBuffer`.
struct MessageBufferParser<'a> {
    /// The composer state associated with the buffer.
    composer_state: &'a ComposerState,
    /// The current position of the iterator in the buffer.
    iter: gtk::TextIter,
    /// The position of the end of the buffer.
    end: gtk::TextIter,
}

impl<'a> MessageBufferParser<'a> {
    /// Construct a `MessageBufferParser` to iterate between the given start and
    /// end in a buffer.
    fn new(composer_state: &'a ComposerState, start: gtk::TextIter, end: gtk::TextIter) -> Self {
        Self {
            composer_state,
            iter: start,
            end,
        }
    }
}

impl<'a> Iterator for MessageBufferParser<'a> {
    type Item = MessageBufferChunk;

    fn next(&mut self) -> Option<Self::Item> {
        if self.iter == self.end {
            // We reached the end.
            return None;
        }

        if let Some(source) = self
            .iter
            .child_anchor()
            .and_then(|anchor| self.composer_state.widget_at_anchor(&anchor))
            .and_then(|widget| widget.downcast::<Pill>().ok())
            .and_then(|p| p.source())
        {
            self.iter.forward_cursor_position();

            return Some(MessageBufferChunk::Mention(source));
        }

        // This chunk is not a mention. Go forward until the next mention or the
        // end and return the text in between.
        let start = self.iter;
        while self.iter.forward_cursor_position() && self.iter != self.end {
            if self
                .iter
                .child_anchor()
                .and_then(|anchor| self.composer_state.widget_at_anchor(&anchor))
                .and_then(|widget| widget.downcast::<Pill>().ok())
                .is_some()
            {
                break;
            }
        }

        let text = self.iter.buffer().text(&start, &self.iter, false);
        // We might somehow have an empty string before the end, or at the end,
        // because of hidden `char`s in the buffer, so we must only return
        // `None` when we have an empty string at the end.
        if self.iter == self.end && text.is_empty() {
            None
        } else {
            Some(MessageBufferChunk::Text(text.into()))
        }
    }
}
