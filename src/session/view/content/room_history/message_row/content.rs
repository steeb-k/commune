use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{gdk, glib, glib::clone};
use matrix_sdk_ui::timeline::{
    Message, RepliedToInfo, ReplyContent, TimelineDetails, TimelineItemContent,
};
use ruma::{events::room::message::MessageType, OwnedEventId, OwnedTransactionId};
use tracing::{error, warn};

use super::{
    audio::MessageAudio, caption::MessageCaption, file::MessageFile, location::MessageLocation,
    reply::MessageReply, text::MessageText, visual_media::MessageVisualMedia,
};
use crate::{
    prelude::*,
    session::model::{Event, Member, Room, Session},
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
        format: Cell<ContentFormat>,
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
    pub(crate) fn visual_media_widget(&self) -> Option<MessageVisualMedia> {
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
    pub(crate) fn update_for_event(&self, event: &Event) {
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
                        reply.set_show_related_content_header(replied_to_content.can_show_header());
                        reply.set_related_content_sender(replied_to_sender.upcast_ref());
                        build_content(
                            reply.related_content(),
                            replied_to_content.clone(),
                            ContentFormat::Compact,
                            &replied_to_sender,
                            replied_to_detect_at_room,
                            None,
                            event.reply_to_id(),
                        );
                        build_content(
                            reply.content(),
                            event.content(),
                            ContentFormat::Natural,
                            &event.sender(),
                            detect_at_room,
                            event.transaction_id(),
                            event.event_id(),
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
            &event.sender(),
            detect_at_room,
            event.transaction_id(),
            event.event_id(),
        );
    }

    /// Update this widget to present the given related event.
    pub(crate) fn update_for_related_event(&self, info: &RepliedToInfo, sender: &Member) {
        let ReplyContent::Message(message) = info.content() else {
            return;
        };

        let detect_at_room = message.can_contain_at_room() && sender.can_notify_room();

        build_message_content(
            self,
            message,
            self.format(),
            sender,
            detect_at_room,
            None,
            Some(info.event_id().to_owned()),
        );
    }

    /// Get the texture displayed by this widget, if any.
    pub(crate) fn texture(&self) -> Option<gdk::Texture> {
        self.visual_media_widget()?.texture()
    }
}

/// Build the content widget of `event` as a child of `parent`.
fn build_content(
    parent: &impl IsA<adw::Bin>,
    content: TimelineItemContent,
    format: ContentFormat,
    sender: &Member,
    detect_at_room: bool,
    transaction_id: Option<OwnedTransactionId>,
    event_id: Option<OwnedEventId>,
) {
    let room = sender.room();

    match content {
        TimelineItemContent::Message(message) => {
            build_message_content(
                parent,
                &message,
                format,
                sender,
                detect_at_room,
                transaction_id,
                event_id,
            );
        }
        TimelineItemContent::Sticker(sticker) => {
            build_media_message_content(
                parent,
                sticker.content().clone().into(),
                format,
                &room,
                detect_at_room,
                MessageCacheKey {
                    transaction_id,
                    event_id,
                    is_edited: false,
                },
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
    sender: &Member,
    detect_at_room: bool,
    transaction_id: Option<OwnedTransactionId>,
    event_id: Option<OwnedEventId>,
) {
    let room = sender.room();

    if let Some(media_message) = MediaMessage::from_message(message.msgtype()) {
        build_media_message_content(
            parent,
            media_message,
            format,
            &room,
            detect_at_room,
            MessageCacheKey {
                transaction_id,
                event_id,
                is_edited: message.is_edited(),
            },
        );
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
    cache_key: MessageCacheKey,
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

        caption_widget.set_caption(
            caption.to_owned(),
            formatted_caption.cloned(),
            room,
            format,
            detect_at_room,
        );

        let new_widget = build_media_content(
            caption_widget.child(),
            media_message,
            format,
            &session,
            cache_key,
        );
        caption_widget.set_child(Some(new_widget));
    } else {
        let new_widget =
            build_media_content(parent.child(), media_message, format, &session, cache_key);
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
    cache_key: MessageCacheKey,
) -> gtk::Widget {
    match media_message {
        MediaMessage::Audio(audio) => {
            let widget = old_widget
                .and_downcast::<MessageAudio>()
                .unwrap_or_default();

            widget.audio(audio.into(), session, format, cache_key);

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

            widget.set_media_message(image.into(), session, format, cache_key);

            widget.upcast()
        }
        MediaMessage::Video(video) => {
            let widget = old_widget
                .and_downcast::<MessageVisualMedia>()
                .unwrap_or_default();

            widget.set_media_message(video.into(), session, format, cache_key);

            widget.upcast()
        }
        MediaMessage::Sticker(sticker) => {
            let widget = old_widget
                .and_downcast::<MessageVisualMedia>()
                .unwrap_or_default();

            widget.set_media_message(sticker.into(), session, format, cache_key);

            widget.upcast()
        }
    }
}

/// The data used as a cache key for messages.
///
/// This is used when there is no reliable way to detect if the content of a
/// message changed. For example, the URI of a media file might change between a
/// local echo and a remote echo, but we do not need to reload the media in this
/// case, and we have no other way to know that both URIs point to the same
/// file.
#[derive(Debug, Clone, Default)]
pub(crate) struct MessageCacheKey {
    /// The transaction ID of the event.
    ///
    /// Local echo should keep its transaction ID after the message is sent, so
    /// we do not need to reload the message if it did not change.
    transaction_id: Option<OwnedTransactionId>,
    /// The global ID of the event.
    ///
    /// Local echo that was sent and remote echo should have the same event ID,
    /// so we do not need to reload the message if it did not change.
    event_id: Option<OwnedEventId>,
    /// Whether the message is edited.
    ///
    /// The message must be reloaded when it was edited.
    is_edited: bool,
}

impl MessageCacheKey {
    /// Whether the given new `MessageCacheKey` should trigger a reload of the
    /// mmessage compared to this one.
    pub(super) fn should_reload(&self, new: &MessageCacheKey) -> bool {
        if new.is_edited {
            return true;
        }

        let transaction_id_invalidated = self.transaction_id.is_none()
            || new.transaction_id.is_none()
            || self.transaction_id != new.transaction_id;
        let event_id_invalidated =
            self.event_id.is_none() || new.event_id.is_none() || self.event_id != new.event_id;

        transaction_id_invalidated && event_id_invalidated
    }
}
