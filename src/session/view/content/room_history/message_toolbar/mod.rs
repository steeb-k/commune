use ashpd::{
    desktop::location::{Accuracy, LocationProxy},
    WindowIdentifier,
};
use futures_util::{FutureExt, StreamExt, TryFutureExt};
use geo_uri::GeoUri;
use gettextrs::{gettext, pgettext};
use gtk::{
    gdk, gio,
    glib::{self, clone},
    prelude::*,
    subclass::prelude::*,
    CompositeTemplate,
};
use matrix_sdk::{
    attachment::{AttachmentInfo, BaseFileInfo, BaseImageInfo},
    ruma::events::{
        room::message::{EmoteMessageEventContent, FormattedBody, MessageType},
        AnySyncMessageLikeEvent, AnySyncTimelineEvent, SyncMessageLikeEvent,
    },
};
use ruma::events::{
    room::{
        message::{
            AddMentions, ForwardThread, LocationMessageEventContent, MessageFormat,
            OriginalSyncRoomMessageEvent, RoomMessageEventContent,
        },
        power_levels::PowerLevelAction,
    },
    AnyMessageLikeEventContent, MessageLikeEventType,
};
use sourceview::prelude::*;
use tracing::{debug, error, warn};

mod attachment_dialog;
mod completion;

use self::{attachment_dialog::AttachmentDialog, completion::CompletionPopover};
use super::message_row::MessageContent;
use crate::{
    components::{CustomEntry, LabelWithWidgets, Pill},
    gettext_f,
    prelude::*,
    session::model::{Event, EventKey, Member, Membership, Room},
    spawn, spawn_tokio, toast,
    utils::{
        matrix::extract_mentions,
        media::{filename_for_mime, get_audio_info, get_image_info, get_video_info, load_file},
        template_callbacks::TemplateCallbacks,
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

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_history/message_toolbar/mod.ui"
    )]
    pub struct MessageToolbar {
        pub room: glib::WeakRef<Room>,
        /// Whether our own user can send messages in the current room.
        pub can_send_messages: Cell<bool>,
        pub own_member: glib::WeakRef<Member>,
        pub power_levels_handler: RefCell<Option<glib::SignalHandlerId>>,
        pub md_enabled: Cell<bool>,
        pub completion: CompletionPopover,
        #[template_child]
        pub message_entry: TemplateChild<sourceview::View>,
        #[template_child]
        pub related_event_header: TemplateChild<LabelWithWidgets>,
        #[template_child]
        pub related_event_content: TemplateChild<MessageContent>,
        pub related_event_type: Cell<RelatedEventType>,
        pub related_event: RefCell<Option<Event>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MessageToolbar {
        const NAME: &'static str = "MessageToolbar";
        type Type = super::MessageToolbar;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            CustomEntry::static_type();

            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);
            TemplateCallbacks::bind_template_callbacks(klass);

            klass.install_action(
                "message-toolbar.send-text-message",
                None,
                move |widget, _, _| {
                    widget.send_text_message();
                },
            );

            klass.install_action("message-toolbar.select-file", None, move |widget, _, _| {
                spawn!(clone!(@weak widget => async move {
                    widget.select_file().await;
                }));
            });

            klass.install_action("message-toolbar.open-emoji", None, move |widget, _, _| {
                widget.open_emoji();
            });

            klass.install_action("message-toolbar.send-location", None, move |widget, _, _| {
                spawn!(clone!(@weak widget => async move {
                    let toast_error = match widget.send_location().await {
                        // Do nothing if the request was cancelled by the user
                        Err(ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled)) => {
                            error!("Location request was cancelled by the user");
                            Some(gettext("The location request has been cancelled."))
                        },
                        Err(error) => {
                            error!("Failed to send location {error}");
                            Some(gettext("Failed to retrieve current location."))
                        }
                        _ => None,
                    };

                    if let Some(message) = toast_error {
                        toast!(widget, message);
                    }
                }));
            });

            klass.install_property_action("message-toolbar.markdown", "markdown-enabled");

            klass.install_action(
                "message-toolbar.clear-related-event",
                None,
                move |widget, _, _| widget.clear_related_event(),
            );
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for MessageToolbar {
        fn properties() -> &'static [glib::ParamSpec] {
            use once_cell::sync::Lazy;
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpecObject::builder::<Room>("room")
                        .explicit_notify()
                        .build(),
                    glib::ParamSpecBoolean::builder("can-send-messages")
                        .read_only()
                        .build(),
                    glib::ParamSpecBoolean::builder("markdown-enabled")
                        .explicit_notify()
                        .build(),
                    glib::ParamSpecEnum::builder::<RelatedEventType>("related-event-type")
                        .read_only()
                        .build(),
                    glib::ParamSpecObject::builder::<Event>("related-event")
                        .read_only()
                        .build(),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            let obj = self.obj();

            match pspec.name() {
                "room" => obj.set_room(value.get::<Option<Room>>().unwrap().as_ref()),
                "markdown-enabled" => obj.set_markdown_enabled(value.get().unwrap()),
                _ => unimplemented!(),
            }
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            let obj = self.obj();

            match pspec.name() {
                "room" => obj.room().to_value(),
                "can-send-messages" => obj.can_send_messages().to_value(),
                "markdown-enabled" => obj.markdown_enabled().to_value(),
                "related-event-type" => obj.related_event_type().to_value(),
                "related-event" => obj.related_event().to_value(),
                _ => unimplemented!(),
            }
        }

        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            // Clipboard.
            self.message_entry
                .connect_paste_clipboard(clone!(@weak obj => move |entry| {
                    if !obj.can_send_messages() {
                        return;
                    }

                    let formats = obj.clipboard().formats();

                    // We only handle files and supported images.
                    if formats.contains_type(gio::File::static_type()) || formats.contains_type(gdk::Texture::static_type()) {
                        entry.stop_signal_emission_by_name("paste-clipboard");
                        spawn!(
                            clone!(@weak obj => async move {
                                obj.read_clipboard().await;
                        }));
                    }
                }));
            self.message_entry
                .connect_copy_clipboard(clone!(@weak obj => move |entry| {
                    entry.stop_signal_emission_by_name("copy-clipboard");
                    obj.copy_buffer_selection_to_clipboard();
                }));
            self.message_entry
                .connect_cut_clipboard(clone!(@weak obj => move |entry| {
                    entry.stop_signal_emission_by_name("cut-clipboard");
                    obj.copy_buffer_selection_to_clipboard();
                    entry.buffer().delete_selection(true, true);
                }));

            // Key bindings.
            let key_events = gtk::EventControllerKey::new();
            key_events
                .connect_key_pressed(clone!(@weak obj => @default-return glib::Propagation::Proceed, move |_, key, _, modifier| {
                if modifier.is_empty() && (key == gdk::Key::Return || key == gdk::Key::KP_Enter) {
                    obj.send_text_message();
                    glib::Propagation::Stop
                } else if modifier.is_empty() && key == gdk::Key::Escape && obj.related_event_type() != RelatedEventType::None {
                    obj.clear_related_event();
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            }));
            self.message_entry.add_controller(key_events);

            let buffer = self
                .message_entry
                .buffer()
                .downcast::<sourceview::Buffer>()
                .unwrap();

            crate::utils::sourceview::setup_style_scheme(&buffer);

            // Actions on changes in message entry.
            buffer.connect_text_notify(clone!(@weak obj => move |buffer| {
               let (start_iter, end_iter) = buffer.bounds();
               let is_empty = start_iter == end_iter;
               obj.action_set_enabled("message-toolbar.send-text-message", !is_empty);
               obj.send_typing_notification(!is_empty);
            }));

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

            obj.set_sensitive(obj.can_send_messages());
        }

        fn dispose(&self) {
            self.completion.unparent();
        }
    }

    impl WidgetImpl for MessageToolbar {}
    impl BoxImpl for MessageToolbar {}
}

glib::wrapper! {
    /// A toolbar with different actions to send messages.
    pub struct MessageToolbar(ObjectSubclass<imp::MessageToolbar>)
        @extends gtk::Widget, gtk::Box, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl MessageToolbar {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// The room to send messages in.
    pub fn room(&self) -> Option<Room> {
        self.imp().room.upgrade()
    }

    /// Set the room currently displayed.
    pub fn set_room(&self, room: Option<&Room>) {
        let old_room = self.room();
        if old_room.as_ref() == room {
            return;
        }

        let imp = self.imp();

        if let Some(room) = old_room {
            if let Some(handler) = imp.power_levels_handler.take() {
                room.power_levels().disconnect(handler);
            }
        }

        self.clear_related_event();

        imp.room.set(room);

        self.update_completion(room);
        self.set_up_can_send_messages(room);
        imp.message_entry.grab_focus();

        self.notify("room");
    }

    /// The `Member` for our own user in the current room.
    pub fn own_member(&self) -> Option<Member> {
        self.imp().own_member.upgrade()
    }

    /// Whether outgoing messages should be interpreted as markdown.
    pub fn markdown_enabled(&self) -> bool {
        self.imp().md_enabled.get()
    }

    /// Set whether outgoing messages should be interpreted as markdown.
    pub fn set_markdown_enabled(&self, enabled: bool) {
        let imp = self.imp();

        imp.md_enabled.set(enabled);

        self.notify("markdown-enabled");
    }

    /// The type of related event of the composer.
    pub fn related_event_type(&self) -> RelatedEventType {
        self.imp().related_event_type.get()
    }

    /// Set the type of related event of the composer.
    fn set_related_event_type(&self, related_type: RelatedEventType) {
        if self.related_event_type() == related_type {
            return;
        }

        self.imp().related_event_type.set(related_type);
        self.notify("related-event-type");
    }

    /// The related event of the composer.
    pub fn related_event(&self) -> Option<Event> {
        self.imp().related_event.borrow().clone()
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
        self.notify("related-event");
    }

    pub fn clear_related_event(&self) {
        if self.related_event_type() == RelatedEventType::Edit {
            // Clean up the entry.
            self.imp().message_entry.buffer().set_text("");
        };

        self.set_related_event(None);
        self.set_related_event_type(RelatedEventType::default());
    }

    pub fn set_reply_to(&self, event: Event) {
        let imp = self.imp();
        imp.related_event_header
            .set_widgets(vec![Pill::for_user(event.sender().upcast_ref())]);
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
        // We don't support editing non-text messages.
        let Some((text, formatted)) = event.message().and_then(|msg| match msg {
            MessageType::Emote(emote) => Some((format!("/me {}", emote.body), emote.formatted)),
            MessageType::Text(text) => Some((text.body, text.formatted)),
            _ => None,
        }) else {
            return;
        };

        let mentions = if let Some(html) =
            formatted.and_then(|f| (f.format == MessageFormat::Html).then_some(f.body))
        {
            let (_, mentions) = extract_mentions(&html, &event.room());
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
                    text[pos..].find(&s).map(|index| {
                        let start = pos + index;
                        let end = start + s.len();
                        pos = end;
                        (pill, (start, end))
                    })
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        let imp = self.imp();
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
            // the TextView.
            buffer.set_text("");

            let mut pos = 0;
            let mut iter = buffer.iter_at_offset(0);

            for (pill, (start, end)) in mentions {
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

    fn send_text_message(&self) {
        if !self.can_send_messages() {
            return;
        }
        let Some(room) = self.room() else {
            return;
        };

        let imp = self.imp();
        let buffer = imp.message_entry.buffer();
        let (start_iter, end_iter) = buffer.bounds();
        let body_len = buffer.text(&start_iter, &end_iter, true).len();

        let is_markdown = imp.md_enabled.get();
        let mut has_mentions = false;
        let mut plain_body = String::with_capacity(body_len);
        // formatted_body is Markdown if is_markdown is true, and HTML if false.
        let mut formatted_body = String::with_capacity(body_len);

        for chunk in self.split_buffer_mentions(start_iter, end_iter) {
            match chunk {
                MentionChunk::Text(text) => {
                    plain_body.push_str(&text);
                    formatted_body.push_str(&text);
                }
                MentionChunk::Mention { name, uri } => {
                    has_mentions = true;
                    plain_body.push_str(&name);
                    formatted_body.push_str(&if is_markdown {
                        format!("[{name}]({uri})")
                    } else {
                        format!("<a href=\"{uri}\">{name}</a>")
                    });
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
        } else if has_mentions {
            // Already formatted with HTML
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
        } else {
            let mut content = if let Some(html_body) = html_body {
                RoomMessageEventContent::text_html(plain_body, html_body)
            } else {
                RoomMessageEventContent::text_plain(plain_body)
            };

            if self.related_event_type() == RelatedEventType::Reply {
                let related_event = self
                    .related_event()
                    .unwrap()
                    .raw()
                    .unwrap()
                    .deserialize()
                    .unwrap();
                if let AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::RoomMessage(
                    SyncMessageLikeEvent::Original(related_message_event),
                )) = related_event
                {
                    let full_related_message_event = related_message_event
                        .into_full_event(self.room().unwrap().room_id().to_owned());
                    content = content.make_reply_to(
                        &full_related_message_event,
                        ForwardThread::Yes,
                        AddMentions::No,
                    )
                }
            }

            content
        };

        // Handle edit.
        if self.related_event_type() == RelatedEventType::Edit {
            let related_event = self.related_event().unwrap();
            let related_message = related_event
                .raw()
                .unwrap()
                .deserialize_as::<OriginalSyncRoomMessageEvent>()
                .unwrap();

            // Try to get the replied to message of the original event if it's available
            // locally.
            let replied_to_message = related_event
                .reply_to_id()
                .and_then(|id| room.timeline().event_by_key(&EventKey::EventId(id)))
                .and_then(|e| e.raw())
                .and_then(|r| r.deserialize_as::<OriginalSyncRoomMessageEvent>().ok())
                .map(|e| e.into_full_event(room.room_id().to_owned()));

            content = content.make_replacement(&related_message, replied_to_message.as_ref());
        }

        room.send_room_message_event(content);
        buffer.set_text("");
        self.clear_related_event();
    }

    fn open_emoji(&self) {
        if !self.can_send_messages() {
            return;
        }
        self.imp().message_entry.emit_insert_emoji();
    }

    async fn send_location(&self) -> ashpd::Result<()> {
        if !self.can_send_messages() {
            return Ok(());
        }
        let Some(room) = self.room() else {
            return Ok(());
        };

        let handle = spawn_tokio!(async move {
            let proxy = LocationProxy::new().await?;
            let identifier = WindowIdentifier::default();

            let session = proxy
                .create_session(Some(0), Some(0), Some(Accuracy::Exact))
                .await?;

            // We want to be listening for new locations whenever the session is up
            // otherwise we might lose the first response and will have to wait for a future
            // update by geoclue
            // FIXME: We should update the location on the map according to updates received
            // by the proxy.
            let mut stream = proxy.receive_location_updated().await?;
            let (_, location) = futures_util::try_join!(
                proxy.start(&session, &identifier).into_future(),
                stream.next().map(|l| l.ok_or(ashpd::Error::NoResponse))
            )?;

            ashpd::Result::Ok(location)
        });

        let location = handle.await.unwrap()?;
        let geo_uri = GeoUri::builder()
            .latitude(location.latitude())
            .longitude(location.longitude())
            .build()
            .expect("Got invalid coordinates from ashpd");

        let window = self.root().and_downcast::<gtk::Window>().unwrap();
        let dialog = AttachmentDialog::for_location(&window, &gettext("Your Location"), &geo_uri);
        if dialog.run_future().await != gtk::ResponseType::Ok {
            return Ok(());
        }

        let geo_uri_string = geo_uri.to_string();
        let iso8601_datetime =
            glib::DateTime::from_unix_local(location.timestamp().as_secs() as i64)
                .expect("Valid location timestamp");
        let location_body = gettext_f(
            // Translators: Do NOT translate the content between '{' and '}', this is a variable
            // name.
            "User Location {geo_uri} at {iso8601_datetime}",
            &[
                ("geo_uri", &geo_uri_string),
                (
                    "iso8601_datetime",
                    iso8601_datetime.format_iso8601().unwrap().as_str(),
                ),
            ],
        );
        room.send_room_message_event(AnyMessageLikeEventContent::RoomMessage(
            RoomMessageEventContent::new(MessageType::Location(LocationMessageEventContent::new(
                location_body,
                geo_uri_string,
            ))),
        ));

        Ok(())
    }

    async fn send_image(&self, image: gdk::Texture) {
        if !self.can_send_messages() {
            return;
        }

        let window = self.root().and_downcast::<gtk::Window>().unwrap();
        let filename = filename_for_mime(Some(mime::IMAGE_PNG.as_ref()), None);
        let dialog = AttachmentDialog::for_image(&window, &filename, &image);

        if dialog.run_future().await != gtk::ResponseType::Ok {
            return;
        }

        let Some(room) = self.room() else {
            return;
        };

        let bytes = image.save_to_png_bytes();
        let info = AttachmentInfo::Image(BaseImageInfo {
            width: Some((image.width() as u32).into()),
            height: Some((image.height() as u32).into()),
            size: Some((bytes.len() as u32).into()),
            blurhash: None,
        });

        room.send_attachment(bytes.to_vec(), mime::IMAGE_PNG, &filename, info);
    }

    pub async fn select_file(&self) {
        if !self.can_send_messages() {
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

    pub async fn send_file(&self, file: gio::File) {
        match load_file(&file).await {
            Ok((bytes, file_info)) => {
                let window = self.root().and_downcast::<gtk::Window>().unwrap();
                let dialog = AttachmentDialog::for_file(&window, &file_info.filename, &file);

                if dialog.run_future().await != gtk::ResponseType::Ok {
                    return;
                }

                let Some(room) = self.room() else {
                    error!("Cannot send file without a room");
                    return;
                };

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

                room.send_attachment(bytes, file_info.mime, &file_info.filename, info);
            }
            Err(error) => {
                warn!("Could not read file: {error}");
                toast!(self, gettext("Error reading file"));
            }
        }
    }

    async fn read_clipboard(&self) {
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

    pub fn handle_paste_action(&self) {
        if !self.can_send_messages() {
            return;
        }

        spawn!(glib::clone!(@weak self as obj => async move {
            obj.read_clipboard().await;
        }));
    }

    // Update the completion for the current room.
    fn update_completion(&self, room: Option<&Room>) {
        let completion = &self.imp().completion;

        completion.set_user_id(room.map(|r| r.session().user_id().to_string()));
        // `RoomHistory` should have a strong reference to the list so we can use
        // `get_or_create_members()`.
        completion.set_members(room.map(|r| r.get_or_create_members()));
    }

    // Copy the selection in the message entry to the clipboard while replacing
    // mentions.
    fn copy_buffer_selection_to_clipboard(&self) {
        if let Some((start, end)) = self.imp().message_entry.buffer().selection_bounds() {
            let content: String = self
                .split_buffer_mentions(start, end)
                .map(|chunk| match chunk {
                    MentionChunk::Text(str) => str,
                    MentionChunk::Mention { name, .. } => name,
                })
                .collect();
            self.clipboard().set_text(&content);
        }
    }

    fn send_typing_notification(&self, typing: bool) {
        if let Some(room) = self.room() {
            room.send_typing_notification(typing);
        }
    }

    /// Whether our own user can send messages in the current room.
    pub fn can_send_messages(&self) -> bool {
        self.imp().can_send_messages.get()
    }

    /// Update whether our own user can send messages in the current room.
    fn update_can_send_messages(&self) {
        let can_send = self.compute_can_send_messages();

        if self.can_send_messages() == can_send {
            return;
        }

        self.imp().can_send_messages.set(can_send);
        self.set_sensitive(can_send);
        self.notify("can-send-messages");
    }

    fn set_up_can_send_messages(&self, room: Option<&Room>) {
        if let Some(room) = room {
            let own_user_id = room.session().user_id().to_owned();
            let imp = self.imp();

            let own_member = room
                .get_or_create_members()
                .get_or_create(own_user_id.clone());

            // We don't need to keep the handler around, the member should be dropped when
            // switching rooms.
            own_member.connect_notify_local(
                Some("membership"),
                clone!(@weak self as obj => move |_, _| {
                    obj.update_can_send_messages();
                }),
            );
            imp.own_member.set(Some(&own_member));

            let power_levels_handler = room.power_levels().connect_notify_local(
                Some("power-levels"),
                clone!(@weak self as obj => move |_, _| {
                    obj.update_can_send_messages();
                }),
            );
            imp.power_levels_handler.replace(Some(power_levels_handler));
        }

        self.update_can_send_messages();
    }

    fn compute_can_send_messages(&self) -> bool {
        let Some(room) = self.room() else {
            return false;
        };
        let Some(member) = self.own_member() else {
            return false;
        };

        if member.membership() != Membership::Join {
            return false;
        }

        room.power_levels().member_is_allowed_to(
            &member.user_id(),
            PowerLevelAction::SendMessage(MessageLikeEventType::RoomMessage),
        )
    }
}

enum MentionChunk {
    Text(String),
    Mention { name: String, uri: String },
}

struct SplitMentions {
    iter: gtk::TextIter,
    end: gtk::TextIter,
}

impl Iterator for SplitMentions {
    type Item = MentionChunk;

    fn next(&mut self) -> Option<Self::Item> {
        if self.iter == self.end {
            // We reached the end.
            return None;
        }

        if let Some(pill) = self
            .iter
            .child_anchor()
            .map(|anchor| anchor.widgets())
            .as_ref()
            .and_then(|widgets| widgets.first())
            .and_then(|widget| widget.downcast_ref::<Pill>())
        {
            // This chunk is a mention.
            let (name, uri) = if let Some(user) = pill.user() {
                (
                    user.display_name(),
                    UserExt::user_id(&user).matrix_to_uri().to_string(),
                )
            } else if let Some(room) = pill.room() {
                (
                    room.display_name(),
                    room.room_id().matrix_to_uri().to_string(),
                )
            } else {
                unreachable!()
            };

            self.iter.forward_cursor_position();

            return Some(MentionChunk::Mention { name, uri });
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
