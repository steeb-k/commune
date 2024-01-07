use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{gdk, glib, glib::clone};
use matrix_sdk_ui::timeline::{TimelineDetails, TimelineItemContent};
use ruma::events::room::message::MessageType;
use tracing::{error, warn};

use super::{
    audio::MessageAudio, file::MessageFile, location::MessageLocation, media::MessageMedia,
    reply::MessageReply, text::MessageText,
};
use crate::{
    session::model::{content_can_show_header, Event, Member, Room},
    spawn,
    utils::media::filename_for_mime,
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

    /// Access the widget with the own content of the event.
    ///
    /// This allows to access the descendant content while discarding the
    /// content of a related message, like a replied-to event.
    pub fn content_widget(&self) -> Option<gtk::Widget> {
        let child = self.child()?;

        if let Some(reply) = child.downcast_ref::<MessageReply>() {
            reply.content().child()
        } else {
            Some(child)
        }
    }

    pub fn update_for_event(&self, event: &Event) {
        let room = event.room();
        let format = self.format();
        if format == ContentFormat::Natural {
            if let Some(related_content) = event.reply_to_event_content() {
                match related_content {
                    TimelineDetails::Unavailable => {
                        spawn!(
                            glib::Priority::HIGH,
                            clone!(@weak event => async move {
                                if let Err(error) = event.fetch_missing_details().await {
                                    error!("Failed to fetch event details: {error}");
                                }
                            })
                        );
                    }
                    TimelineDetails::Error(error) => {
                        error!(
                            "Failed to fetch replied to event '{}': {error}",
                            event.reply_to_id().unwrap()
                        );
                    }
                    TimelineDetails::Ready(replied_to_event) => {
                        // We should have a strong reference to the list in the RoomHistory so we
                        // can use `get_or_create_members()`.
                        let replied_to_sender = room
                            .get_or_create_members()
                            .get_or_create(replied_to_event.sender().to_owned());
                        let replied_to_content = replied_to_event.content();

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
                            &room,
                        );
                        build_content(
                            reply.content(),
                            event.content(),
                            ContentFormat::Natural,
                            event.sender(),
                            &room,
                        );
                        self.set_child(Some(&reply));

                        return;
                    }
                    TimelineDetails::Pending => {}
                }
            }
        }

        build_content(self, event.content(), format, event.sender(), &room);
    }

    /// Get the texture displayed by this widget, if any.
    pub fn texture(&self) -> Option<gdk::Texture> {
        self.content_widget()?
            .downcast_ref::<MessageMedia>()?
            .texture()
    }
}

/// Build the content widget of `event` as a child of `parent`.
fn build_content(
    parent: &impl IsA<adw::Bin>,
    content: TimelineItemContent,
    format: ContentFormat,
    sender: Member,
    room: &Room,
) {
    let Some(session) = room.session() else {
        return;
    };

    let parent = parent.upcast_ref();
    match content {
        TimelineItemContent::Message(message) => {
            match message.msgtype() {
                MessageType::Audio(message) => {
                    let child = if let Some(child) = parent.child().and_downcast::<MessageAudio>() {
                        child
                    } else {
                        let child = MessageAudio::new();
                        parent.set_child(Some(&child));
                        child
                    };
                    child.audio(message.clone(), &session, format);
                }
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
                        room,
                        format,
                    );
                }
                MessageType::File(message) => {
                    let info = message.info.as_ref();
                    let filename = message
                        .filename
                        .clone()
                        .filter(|name| !name.is_empty())
                        .or_else(|| Some(message.body.clone()))
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| {
                            filename_for_mime(info.and_then(|info| info.mimetype.as_deref()), None)
                        });

                    let child = if let Some(child) = parent.child().and_downcast::<MessageFile>() {
                        child
                    } else {
                        let child = MessageFile::new();
                        parent.set_child(Some(&child));
                        child
                    };
                    child.set_filename(Some(filename));
                    child.set_format(format);
                }
                MessageType::Image(message) => {
                    let child = if let Some(child) = parent.child().and_downcast::<MessageMedia>() {
                        child
                    } else {
                        let child = MessageMedia::new();
                        parent.set_child(Some(&child));
                        child
                    };
                    child.image(message.clone(), &session, format);
                }
                MessageType::Location(message) => {
                    let child =
                        if let Some(child) = parent.child().and_downcast::<MessageLocation>() {
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
                        room,
                        format,
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
                    child.with_text(message.body.clone(), format);
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
                        room,
                        format,
                    );
                }
                MessageType::Video(message) => {
                    let child = if let Some(child) = parent.child().and_downcast::<MessageMedia>() {
                        child
                    } else {
                        let child = MessageMedia::new();
                        parent.set_child(Some(&child));
                        child
                    };
                    child.video(message.clone(), &session, format);
                }
                MessageType::VerificationRequest(_) => {
                    // TODO: show more information about the verification
                    let child = if let Some(child) = parent.child().and_downcast::<MessageText>() {
                        child
                    } else {
                        let child = MessageText::new();
                        parent.set_child(Some(&child));
                        child
                    };
                    child.with_text(gettext("Identity verification was started"), format);
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
                    child.with_text(gettext("Unsupported event"), format);
                }
            }
        }
        TimelineItemContent::Sticker(sticker) => {
            let child = if let Some(child) = parent.child().and_downcast::<MessageMedia>() {
                child
            } else {
                let child = MessageMedia::new();
                parent.set_child(Some(&child));
                child
            };
            child.sticker(sticker.content().clone(), &session, format);
        }
        TimelineItemContent::UnableToDecrypt(_) => {
            let child = if let Some(child) = parent.child().and_downcast::<MessageText>() {
                child
            } else {
                let child = MessageText::new();
                parent.set_child(Some(&child));
                child
            };
            child.with_text(gettext("Unable to decrypt this message, decryption will be retried once the keys are available."), format);
        }
        TimelineItemContent::RedactedMessage => {
            let child = if let Some(child) = parent.child().and_downcast::<MessageText>() {
                child
            } else {
                let child = MessageText::new();
                parent.set_child(Some(&child));
                child
            };
            child.with_text(gettext("This message was removed."), format);
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
            child.with_text(gettext("Unsupported event"), format);
        }
    }
}
