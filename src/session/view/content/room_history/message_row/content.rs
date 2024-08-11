use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{gdk, glib, glib::clone};
use matrix_sdk_ui::timeline::{
    Message, RepliedToInfo, ReplyContent, TimelineDetails, TimelineItemContent,
};
use ruma::events::room::message::MessageType;
use tracing::{error, warn};

use super::{
    audio::MessageAudio, caption::MessageCaption, file::MessageFile, location::MessageLocation,
    reply::MessageReply, text::MessageText, visual_media::MessageVisualMedia,
};
use crate::{
    prelude::*,
    session::model::{content_can_show_header, Event, Member, Room, Session},
    spawn,
    utils::matrix::MediaMessage,
};

#[derive(Debug, Default, Hash, Eq, PartialEq, Clone, Copy, glib::Enum)]
#[repr(i32)]
#[enum_type(name = "ContentFormat")]
pub enum ContentFormat {
    /// The content should appear at its natural size.
    #[default]
    Natural = 0,

    /// The content should appear in a smaller format without interactions, if
    /// possible.
    ///
    /// This has no effect on text replies.
    ///
    /// The related events of replies are not displayed.
    Compact = 1,

    /// Like `Compact`, but the content should be ellipsized if possible to show
    /// only a single line.
    Ellipsized = 2,
}

mod imp {
    use std::cell::Cell;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::MessageContent)]
    pub struct MessageContent {
        /// The displayed format of the message.
        #[property(get, set = Self::set_format, explicit_notify, builder(ContentFormat::default()))]
        pub format: Cell<ContentFormat>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MessageContent {
        const NAME: &'static str = "ContentMessageContent";
        type Type = super::MessageContent;
        type ParentType = adw::Bin;
    }

    #[glib::derived_properties]
    impl ObjectImpl for MessageContent {}

    impl WidgetImpl for MessageContent {}
    impl BinImpl for MessageContent {}

    impl MessageContent {
        /// Set the displayed format of the message.
        fn set_format(&self, format: ContentFormat) {
            if self.format.get() == format {
                return;
            }

            self.format.set(format);
            self.obj().notify_format();
        }
    }
}

glib::wrapper! {
    /// The content of a message in the timeline.
    pub struct MessageContent(ObjectSubclass<imp::MessageContent>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl MessageContent {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Access the widget with the visual media content of the event, if any.
    ///
    /// This allows to access the descendant content while discarding the
    /// content of a related message, like a replied-to event, or the caption of
    /// the event.
    pub fn visual_media_widget(&self) -> Option<MessageVisualMedia> {
        let mut child = self.child()?;

        // If it is a reply, the media is in the main content.
        if let Some(reply) = child.downcast_ref::<MessageReply>() {
            child = reply.content().child()?;
        }

        // If it is a caption, the media is the child of the caption.
        if let Some(caption) = child.downcast_ref::<MessageCaption>() {
            child = caption.child()?;
        }

        child.downcast::<MessageVisualMedia>().ok()
    }

    /// Update this widget to present the given `Event`.
    pub fn update_for_event(&self, event: &Event) {
        let detect_at_room = event.can_contain_at_room() && event.sender().can_notify_room();

        let format = self.format();
        if format == ContentFormat::Natural {
            if let Some(related_content) = event.reply_to_event_content() {
                match related_content {
                    TimelineDetails::Unavailable => {
                        spawn!(
                            glib::Priority::HIGH,
                            clone!(
                                #[weak]
                                event,
                                async move {
                                    if let Err(error) = event.fetch_missing_details().await {
                                        error!("Could not fetch event details: {error}");
                                    }
                                }
                            )
                        );
                    }
                    TimelineDetails::Error(error) => {
                        error!(
                            "Could not fetch replied to event '{}': {error}",
                            event.reply_to_id().unwrap()
                        );
                    }
                    TimelineDetails::Ready(replied_to_event) => {
                        // We should have a strong reference to the list in the RoomHistory so we
                        // can use `get_or_create_members()`.
                        let replied_to_sender = event
                            .room()
                            .get_or_create_members()
                            .get_or_create(replied_to_event.sender().to_owned());
                        let replied_to_content = replied_to_event.content();
                        let replied_to_detect_at_room = replied_to_content.can_contain_at_room()
                            && replied_to_sender.can_notify_room();

                        let reply = MessageReply::new();
                        reply.set_show_related_content_header(content_can_show_header(
                            replied_to_content,
                        ));
                        reply.set_related_content_sender(replied_to_sender.upcast_ref());
                        build_content(
                            reply.related_content(),
                            replied_to_content.clone(),
                            ContentFormat::Compact,
                            replied_to_sender,
                            replied_to_detect_at_room,
                        );
                        build_content(
                            reply.content(),
                            event.content(),
                            ContentFormat::Natural,
                            event.sender(),
                            detect_at_room,
                        );
                        self.set_child(Some(&reply));

                        return;
                    }
                    TimelineDetails::Pending => {}
                }
            }
        }

        build_content(
            self,
            event.content(),
            format,
            event.sender(),
            detect_at_room,
        );
    }

    /// Update this widget to present the given related event.
    pub fn update_for_related_event(&self, info: RepliedToInfo, sender: Member) {
        let ReplyContent::Message(message) = info.content() else {
            return;
        };

        let detect_at_room = message.can_contain_at_room() && sender.can_notify_room();

        build_message_content(self, message, self.format(), sender, detect_at_room);
    }

    /// Get the texture displayed by this widget, if any.
    pub fn texture(&self) -> Option<gdk::Texture> {
        self.visual_media_widget()?.texture()
    }
}

/// Build the content widget of `event` as a child of `parent`.
fn build_content(
    parent: &impl IsA<adw::Bin>,
    content: TimelineItemContent,
    format: ContentFormat,
    sender: Member,
    detect_at_room: bool,
) {
    let room = sender.room();

    match content {
        TimelineItemContent::Message(message) => {
            build_message_content(parent, &message, format, sender, detect_at_room)
        }
        TimelineItemContent::Sticker(sticker) => {
            build_media_message_content(
                parent,
                sticker.content().clone().into(),
                format,
                &room,
                detect_at_room,
            );
        }
        TimelineItemContent::UnableToDecrypt(_) => {
            let child = if let Some(child) = parent.child().and_downcast::<MessageText>() {
                child
            } else {
                let child = MessageText::new();
                parent.set_child(Some(&child));
                child
            };
            child.with_plain_text(gettext("Could not decrypt this message, decryption will be retried once the keys are available."), format);
        }
        TimelineItemContent::RedactedMessage => {
            let child = if let Some(child) = parent.child().and_downcast::<MessageText>() {
                child
            } else {
                let child = MessageText::new();
                parent.set_child(Some(&child));
                child
            };
            child.with_plain_text(gettext("This message was removed."), format);
        }
        content => {
            warn!("Unsupported event content: {content:?}");
            let child = if let Some(child) = parent.child().and_downcast::<MessageText>() {
                child
            } else {
                let child = MessageText::new();
                parent.set_child(Some(&child));
                child
            };
            child.with_plain_text(gettext("Unsupported event"), format);
        }
    }
}

/// Build the content widget of the given message as a child of `parent`.
fn build_message_content(
    parent: &impl IsA<adw::Bin>,
    message: &Message,
    format: ContentFormat,
    sender: Member,
    detect_at_room: bool,
) {
    let room = sender.room();

    if let Some(media_message) = MediaMessage::from_message(message.msgtype()) {
        build_media_message_content(parent, media_message, format, &room, detect_at_room);
        return;
    }

    match message.msgtype() {
        MessageType::Emote(message) => {
            let child = if let Some(child) = parent.child().and_downcast::<MessageText>() {
                child
            } else {
                let child = MessageText::new();
                parent.set_child(Some(&child));
                child
            };
            child.with_emote(
                message.formatted.clone(),
                message.body.clone(),
                sender,
                &room,
                format,
                detect_at_room,
            );
        }
        MessageType::Location(message) => {
            let child = if let Some(child) = parent.child().and_downcast::<MessageLocation>() {
                child
            } else {
                let child = MessageLocation::new();
                parent.set_child(Some(&child));
                child
            };
            child.set_geo_uri(&message.geo_uri, format);
        }
        MessageType::Notice(message) => {
            let child = if let Some(child) = parent.child().and_downcast::<MessageText>() {
                child
            } else {
                let child = MessageText::new();
                parent.set_child(Some(&child));
                child
            };
            child.with_markup(
                message.formatted.clone(),
                message.body.clone(),
                &room,
                format,
                detect_at_room,
            );
        }
        MessageType::ServerNotice(message) => {
            let child = if let Some(child) = parent.child().and_downcast::<MessageText>() {
                child
            } else {
                let child = MessageText::new();
                parent.set_child(Some(&child));
                child
            };
            child.with_plain_text(message.body.clone(), format);
        }
        MessageType::Text(message) => {
            let child = if let Some(child) = parent.child().and_downcast::<MessageText>() {
                child
            } else {
                let child = MessageText::new();
                parent.set_child(Some(&child));
                child
            };
            child.with_markup(
                message.formatted.clone(),
                message.body.clone(),
                &room,
                format,
                detect_at_room,
            );
        }
        msgtype => {
            warn!("Event not supported: {msgtype:?}");
            let child = if let Some(child) = parent.child().and_downcast::<MessageText>() {
                child
            } else {
                let child = MessageText::new();
                parent.set_child(Some(&child));
                child
            };
            child.with_plain_text(gettext("Unsupported event"), format);
        }
    }
}

/// Build the content widget of the given media message as a child of `parent`.
fn build_media_message_content(
    parent: &impl IsA<adw::Bin>,
    media_message: MediaMessage,
    format: ContentFormat,
    room: &Room,
    detect_at_room: bool,
) {
    let Some(session) = room.session() else {
        return;
    };

    if let Some((caption, formatted_caption)) = media_message.caption() {
        let caption_widget =
            if let Some(caption_widget) = parent.child().and_downcast::<MessageCaption>() {
                caption_widget
            } else {
                let caption_widget = MessageCaption::new();
                parent.set_child(Some(&caption_widget));
                caption_widget
            };

        caption_widget.set_caption(caption, formatted_caption, room, format, detect_at_room);

        let new_widget =
            build_media_content(caption_widget.child(), media_message, format, &session);
        caption_widget.set_child(Some(new_widget));
    } else {
        let new_widget = build_media_content(parent.child(), media_message, format, &session);
        parent.set_child(Some(&new_widget));
    }
}

/// Build the content widget of the given media content.
///
/// If the given old widget is of the proper type, it is reused.
fn build_media_content(
    old_widget: Option<gtk::Widget>,
    media_message: MediaMessage,
    format: ContentFormat,
    session: &Session,
) -> gtk::Widget {
    match media_message {
        MediaMessage::Audio(audio) => {
            let widget = old_widget
                .and_downcast::<MessageAudio>()
                .unwrap_or_default();

            widget.audio(audio.into(), session, format);

            widget.upcast()
        }
        MediaMessage::File(file) => {
            let widget = old_widget.and_downcast::<MessageFile>().unwrap_or_default();

            let media_message = MediaMessage::from(file);
            widget.set_filename(Some(media_message.filename()));
            widget.set_format(format);

            widget.upcast()
        }
        MediaMessage::Image(image) => {
            let widget = old_widget
                .and_downcast::<MessageVisualMedia>()
                .unwrap_or_default();

            widget.set_media_message(image.into(), session, format);

            widget.upcast()
        }
        MediaMessage::Video(video) => {
            let widget = old_widget
                .and_downcast::<MessageVisualMedia>()
                .unwrap_or_default();

            widget.set_media_message(video.into(), session, format);

            widget.upcast()
        }
        MediaMessage::Sticker(sticker) => {
            let widget = old_widget
                .and_downcast::<MessageVisualMedia>()
                .unwrap_or_default();

            widget.set_media_message(sticker.into(), session, format);

            widget.upcast()
        }
    }
}
