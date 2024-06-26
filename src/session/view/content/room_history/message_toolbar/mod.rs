use std::io::Cursor;

use adw::{prelude::*, subclass::prelude::*};
use futures_util::{future, pin_mut, StreamExt};
use gettextrs::{gettext, pgettext};
use gtk::{
    gdk, gio,
    glib::{self, clone},
    CompositeTemplate,
};
use matrix_sdk::attachment::{
    generate_image_thumbnail, AttachmentConfig, AttachmentInfo, BaseFileInfo, BaseImageInfo,
    Thumbnail,
};
use ruma::{
    events::{
        room::message::{
            EmoteMessageEventContent, FormattedBody, ForwardThread, LocationMessageEventContent,
            MessageFormat, MessageType, RoomMessageEventContent,
            RoomMessageEventContentWithoutRelation,
        },
        Mentions,
    },
    matrix_uri::MatrixId,
    OwnedUserId,
};
use sourceview::prelude::*;
use tracing::{debug, error, warn};

mod attachment_dialog;
mod completion;

use self::{attachment_dialog::AttachmentDialog, completion::CompletionPopover};
use super::message_row::MessageContent;
use crate::{
    components::{AtRoom, CustomEntry, LabelWithWidgets, Pill},
    gettext_f,
    prelude::*,
    session::model::{Event, Member, Room},
    spawn, spawn_tokio, toast,
    utils::{
        matrix::{find_at_room, find_html_mentions, AT_ROOM},
        media::{filename_for_mime, get_audio_info, get_image_info, get_video_info, load_file},
        template_callbacks::TemplateCallbacks,
        Location, LocationError, TokioDrop,
    },
};

#[derive(Debug, Default, Hash, Eq, PartialEq, Clone, Copy, glib::Enum)]
#[repr(i32)]
#[enum_type(name = "RelatedEventType")]
pub enum RelatedEventType {
    #[default]
    None = 0,
    Reply = 1,
    Edit = 2,
}

mod imp {
    use std::cell::{Cell, RefCell};

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
        /// The type of related event of the composer.
        #[property(get, builder(RelatedEventType::default()))]
        pub related_event_type: Cell<RelatedEventType>,
        /// The related event of the composer.
        #[property(get)]
        pub related_event: RefCell<Option<Event>>,
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

            klass.install_action(
                "message-toolbar.clear-related-event",
                None,
                |widget, _, _| widget.clear_related_event(),
            );
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
                        && obj.related_event_type() != RelatedEventType::None
                    {
                        obj.clear_related_event();
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                }
            ));
            self.message_entry.add_controller(key_events);

            let buffer = self
                .message_entry
                .buffer()
                .downcast::<sourceview::Buffer>()
                .unwrap();

            crate::utils::sourceview::setup_style_scheme(&buffer);

            // Actions on changes in message entry.
            buffer.connect_text_notify(clone!(
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
            let md_lang = sourceview::LanguageManager::default().language("markdown");
            buffer.set_language(md_lang.as_ref());
            obj.bind_property("markdown-enabled", &buffer, "highlight-syntax")
                .sync_create()
                .build();

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

            if let Some(room) = old_room {
                if let Some(handler) = self.can_send_message_handler.take() {
                    room.permissions().disconnect(handler);
                }
            }
            obj.clear_related_event();

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
        let anchor = match insert.child_anchor() {
            Some(anchor) => anchor,
            None => buffer.create_child_anchor(&mut insert),
        };

        let pill = member.to_pill();
        view.add_child_at_anchor(&pill, &anchor);

        view.grab_focus();
    }

    /// Set the type of related event of the composer.
    fn set_related_event_type(&self, related_type: RelatedEventType) {
        if self.related_event_type() == related_type {
            return;
        }

        self.imp().related_event_type.set(related_type);
        self.notify_related_event_type();
    }

    /// Set the related event of the composer.
    fn set_related_event(&self, event: Option<Event>) {
        // We shouldn't reply to events that are not sent yet.
        if let Some(event) = &event {
            if event.event_id().is_none() {
                return;
            }
        }

        let prev_event = self.related_event();

        if prev_event == event {
            return;
        }

        self.imp().related_event.replace(event);
        self.notify_related_event();
    }

    /// Remove the related event.
    pub fn clear_related_event(&self) {
        if self.related_event_type() == RelatedEventType::Edit {
            // Clean up the entry.
            self.imp().message_entry.buffer().set_text("");
        };

        self.set_related_event(None);
        self.set_related_event_type(RelatedEventType::default());
    }

    /// Set the event to reply to.
    pub fn set_reply_to(&self, event: Event) {
        let imp = self.imp();
        if !imp.can_send_message() {
            return;
        }

        imp.related_event_header
            .set_widgets(vec![Pill::new(&event.sender())]);
        imp.related_event_header
            // Translators: Do NOT translate the content between '{' and '}',
            // this is a variable name. In this string, 'Reply' is a noun.
            .set_label(Some(gettext_f("Reply to {user}", &[("user", "<widget>")])));

        imp.related_event_content.update_for_event(&event);
        imp.related_event_content.set_visible(true);

        self.set_related_event_type(RelatedEventType::Reply);
        self.set_related_event(Some(event));
        imp.message_entry.grab_focus();
    }

    /// Set the event to edit.
    pub fn set_edit(&self, event: Event) {
        let Some(room) = self.room() else {
            return;
        };

        let imp = self.imp();
        if !imp.can_send_message() {
            return;
        }

        // We don't support editing non-text messages.
        let Some((text, formatted)) = event.message().and_then(|msg| match msg {
            MessageType::Emote(emote) => Some((format!("/me {}", emote.body), emote.formatted)),
            MessageType::Text(text) => Some((text.body, text.formatted)),
            _ => None,
        }) else {
            return;
        };

        // Try to detect rich mentions.
        let mut mentions = if let Some(html) =
            formatted.and_then(|f| (f.format == MessageFormat::Html).then_some(f.body))
        {
            let mentions = find_html_mentions(&html, &event.room());
            let mut pos = 0;
            // This is looking for the mention link's inner text in the Markdown
            // so it is not super reliable: if there is other text that matches
            // a user's display name in the string it might be replaced instead
            // of the actual mention.
            // Short of an HTML to Markdown converter, it won't be a simple task
            // to locate mentions in Markdown.
            mentions
                .into_iter()
                .filter_map(|(pill, s)| {
                    text[pos..].find(s.as_ref()).map(|index| {
                        let start = pos + index;
                        let end = start + s.len();
                        pos = end;
                        DetectedMention { pill, start, end }
                    })
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        // Try to detect `@room` mentions.
        if room.permissions().can_notify_room() && event.can_contain_at_room() {
            if let Some(start) = find_at_room(&text) {
                let pill = room.at_room().to_pill();
                let end = start + AT_ROOM.len();
                mentions.push(DetectedMention { pill, start, end });

                // Make sure the list is sorted.
                mentions.sort_by(|lhs, rhs| lhs.start.cmp(&rhs.start));
            }
        }

        imp.related_event_header.set_widgets::<gtk::Widget>(vec![]);
        imp.related_event_header
            // Translators: In this string, 'Edit' is a noun.
            .set_label(Some(pgettext("room-history", "Edit")));

        imp.related_event_content.set_visible(false);

        self.set_related_event_type(RelatedEventType::Edit);
        self.set_related_event(Some(event));

        let view = &*imp.message_entry;
        let buffer = view.buffer();

        if mentions.is_empty() {
            buffer.set_text(&text);
        } else {
            // Place the pills instead of the text at the appropriate places in
            // the GtkSourceView.
            buffer.set_text("");

            let mut pos = 0;
            let mut iter = buffer.iter_at_offset(0);

            for DetectedMention { pill, start, end } in mentions {
                if pos != start {
                    buffer.insert(&mut iter, &text[pos..start]);
                }

                let anchor = buffer.create_child_anchor(&mut iter);
                view.add_child_at_anchor(&pill, &anchor);

                pos = end;
            }

            if pos != text.len() {
                buffer.insert(&mut iter, &text[pos..])
            }
        }

        imp.message_entry.grab_focus();
    }

    /// Get an iterator over chunks of the message entry's text between the
    /// given start and end, split by mentions.
    fn split_buffer_mentions(&self, start: gtk::TextIter, end: gtk::TextIter) -> SplitMentions {
        SplitMentions { iter: start, end }
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

        let buffer = imp.message_entry.buffer();
        let (start_iter, end_iter) = buffer.bounds();
        let body_len = buffer.text(&start_iter, &end_iter, true).len();

        let is_markdown = self.markdown_enabled();
        let mut has_rich_mentions = false;
        let mut plain_body = String::with_capacity(body_len);
        // formatted_body is Markdown if is_markdown is true, and HTML if false.
        let mut formatted_body = String::with_capacity(body_len);
        let mut mentions = Mentions::new();

        let mut split_mentions = self.split_buffer_mentions(start_iter, end_iter);
        while let Some(chunk) = split_mentions.next().await {
            match chunk {
                MentionChunk::Text(text) => {
                    plain_body.push_str(&text);
                    formatted_body.push_str(&text);
                }
                MentionChunk::RichMention { name, uri, user_id } => {
                    has_rich_mentions = true;
                    plain_body.push_str(&name);
                    formatted_body.push_str(&if is_markdown {
                        format!("[{name}]({uri})")
                    } else {
                        format!("<a href=\"{uri}\">{name}</a>")
                    });

                    if let Some(user_id) = user_id {
                        mentions.user_ids.insert(user_id);
                    }
                }
                MentionChunk::AtRoom => {
                    plain_body.push_str(AT_ROOM);
                    formatted_body.push_str(AT_ROOM);

                    mentions.room = true;
                }
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
        match self
            .related_event()
            .map(|event| (self.related_event_type(), event.item()))
        {
            Some((RelatedEventType::Reply, reply_item)) => {
                let handle = spawn_tokio!(async move {
                    matrix_timeline
                        .send_reply(content, &reply_item, ForwardThread::Yes)
                        .await
                });
                if let Err(error) = handle.await.unwrap() {
                    error!("Could not send reply: {error}");
                }
            }
            Some((RelatedEventType::Edit, edit_item)) => {
                let handle =
                    spawn_tokio!(async move { matrix_timeline.edit(content, &edit_item).await });
                if let Err(error) = handle.await.unwrap() {
                    error!("Could not send edit: {error}");
                }
            }
            _ => {
                spawn_tokio!(async move {
                    matrix_timeline
                        .send(content.with_relation(None).into())
                        .await
                })
                .await
                .unwrap();
            }
        }

        // Clear the message entry.
        buffer.set_text("");
        self.clear_related_event();
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
        spawn_tokio!(async move { matrix_timeline.send(content.into()).await })
            .await
            .unwrap();
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
            let thumbnail = match generate_image_thumbnail(&mime, Cursor::new(&bytes), None) {
                Ok((data, info)) => Some(Thumbnail {
                    data,
                    content_type: mime.clone(),
                    info: Some(info),
                }),
                _ => None,
            };

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
            error!("Failed to send attachment: {error}");
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
                let mut info = get_image_info(&file).await;
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
        if let Some(event) = &*self.imp().related_event.borrow() {
            self.activate_action(
                "room-history.scroll-to-event",
                Some(&event.key().to_variant()),
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

        let body_len = buffer.text(&start, &end, true).len();
        let mut body = String::with_capacity(body_len);

        let mut split_mentions = self.split_buffer_mentions(start, end);
        while let Some(chunk) = split_mentions.next().await {
            match chunk {
                MentionChunk::Text(text) => {
                    body.push_str(&text);
                }
                MentionChunk::RichMention { name, .. } => {
                    body.push_str(&name);
                }
                MentionChunk::AtRoom => {
                    body.push_str(AT_ROOM);
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

/// A mention that was detected in a message.
struct DetectedMention {
    /// The pill to represent the mention.
    pill: Pill,
    /// The start of the mention in the text.
    start: usize,
    /// The end of the mention in the text.
    end: usize,
}

/// A chunk of a message.
enum MentionChunk {
    /// Some text.
    Text(String),
    /// A rich mention (a mention that has a HTML representation).
    RichMention {
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

/// An iterator over the chunks of a message in a `GtkTextBuffer`.
struct SplitMentions {
    iter: gtk::TextIter,
    end: gtk::TextIter,
}

impl SplitMentions {
    async fn next(&mut self) -> Option<MentionChunk> {
        if self.iter == self.end {
            // We reached the end.
            return None;
        }

        if let Some(source) = self
            .iter
            .child_anchor()
            .map(|anchor| anchor.widgets())
            .as_ref()
            .and_then(|widgets| widgets.first())
            .and_then(|widget| widget.downcast_ref::<Pill>())
            .and_then(|p| p.source())
        {
            // This chunk is a mention.
            let chunk = if let Some(user) = source.downcast_ref::<Member>() {
                MentionChunk::RichMention {
                    name: user.display_name(),
                    uri: user.matrix_to_uri().to_string(),
                    user_id: Some(user.user_id().clone()),
                }
            } else if let Some(room) = source.downcast_ref::<Room>() {
                let matrix_to_uri = room.matrix_to_uri().await;
                let string_repr = match matrix_to_uri.id() {
                    MatrixId::Room(room_id) => room_id.to_string(),
                    MatrixId::RoomAlias(alias) => alias.to_string(),
                    _ => unreachable!(),
                };

                MentionChunk::RichMention {
                    name: string_repr,
                    uri: matrix_to_uri.to_string(),
                    user_id: None,
                }
            } else if source.is::<AtRoom>() {
                MentionChunk::AtRoom
            } else {
                unreachable!()
            };

            self.iter.forward_cursor_position();

            return Some(chunk);
        }

        // This chunk is not a mention. Go forward until the next mention or the
        // end and return the text in between.
        let start = self.iter;
        while self.iter.forward_cursor_position() && self.iter != self.end {
            if self
                .iter
                .child_anchor()
                .map(|anchor| anchor.widgets())
                .as_ref()
                .and_then(|widgets| widgets.first())
                .and_then(|widget| widget.downcast_ref::<Pill>())
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
            Some(MentionChunk::Text(text.into()))
        }
    }
}
