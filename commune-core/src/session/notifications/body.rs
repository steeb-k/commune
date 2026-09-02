//! What a notification is about, before it is put into words.
//!
//! The application's `Notifications` read the event a push or a sync
//! notification carries and chose a sentence for it: what kind of message,
//! whether it is an invite to our own user, whether it is a call and with
//! video. Choosing is a computation over the event and stays here; the
//! sentence is the embedder's, since it is the embedder that has the
//! translations. The application's `show_push` and `show_notification`
//! render these; the Kotlin application renders them in its own
//! notification code.

use ruma::{
    UserId,
    events::{
        AnyMessageLikeEventContent, AnyStrippedStateEvent, AnySyncStateEvent, AnySyncTimelineEvent,
        SyncStateEvent,
        room::{member::MembershipState, message::MessageType},
        rtc::notification::CallIntent,
    },
    html::{HtmlSanitizerMode, RemoveReplyFallback},
};

use crate::matrix::AnySyncOrStrippedTimelineEvent;

/// What a notification says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationBody {
    /// A text, notice or server notice, with its body, the reply fallback
    /// removed.
    Text(String),
    /// An emote, with its body; it reads as the sender followed by the
    /// body.
    Emote(String),
    /// An audio file.
    Audio,
    /// A file.
    File,
    /// An image.
    Image,
    /// A location.
    Location,
    /// A video.
    Video,
    /// A sticker.
    Sticker,
    /// An invite to our own user.
    Invite,
    /// An incoming call announced by an RTC notification, which another
    /// client has to answer.
    IncomingCall {
        /// Whether the call has video.
        video: bool,
    },
}

impl NotificationBody {
    /// What the given event says, if a notification can say it.
    ///
    /// `own_user_id` tells an invite to us from an invite to someone else,
    /// which says nothing.
    #[must_use]
    pub fn of(event: &AnySyncOrStrippedTimelineEvent, own_user_id: &UserId) -> Option<Self> {
        if let Some(body) = Self::of_message(event) {
            return Some(body);
        }

        if let Some(body) = Self::of_call(event) {
            return Some(body);
        }

        Self::of_invite(event, own_user_id)
    }

    /// Whether the given event is an invite to a one-to-one call.
    ///
    /// The push rule `.m.rule.call` fires for these, and the calls module
    /// rings, notifies and withdraws for them; a notification made of the
    /// event would be a second one for one call.
    #[must_use]
    pub fn is_call_invite(event: &AnySyncOrStrippedTimelineEvent) -> bool {
        let AnySyncOrStrippedTimelineEvent::Sync(sync_event) = event else {
            return false;
        };
        let AnySyncTimelineEvent::MessageLike(message_event) = &**sync_event else {
            return false;
        };

        matches!(
            message_event.original_content(),
            Some(AnyMessageLikeEventContent::CallInvite(_))
        )
    }

    /// What the given event says, if it is a message-like event of a
    /// supported type.
    fn of_message(event: &AnySyncOrStrippedTimelineEvent) -> Option<Self> {
        let AnySyncOrStrippedTimelineEvent::Sync(sync_event) = event else {
            return None;
        };
        let AnySyncTimelineEvent::MessageLike(message_event) = &**sync_event else {
            return None;
        };

        match message_event.original_content()? {
            AnyMessageLikeEventContent::RoomMessage(mut message) => {
                message.sanitize(HtmlSanitizerMode::Compat, RemoveReplyFallback::Yes);

                let body = match message.msgtype {
                    MessageType::Audio(_) => Self::Audio,
                    MessageType::Emote(content) => Self::Emote(content.body),
                    MessageType::File(_) => Self::File,
                    MessageType::Image(_) => Self::Image,
                    MessageType::Location(_) => Self::Location,
                    MessageType::Notice(content) => Self::Text(content.body),
                    MessageType::ServerNotice(content) => Self::Text(content.body),
                    MessageType::Text(content) => Self::Text(content.body),
                    MessageType::Video(_) => Self::Video,
                    _ => return None,
                };
                Some(body)
            }
            AnyMessageLikeEventContent::Sticker(_) => Some(Self::Sticker),
            _ => None,
        }
    }

    /// What the given event says, if it is an RTC notification of a call.
    fn of_call(event: &AnySyncOrStrippedTimelineEvent) -> Option<Self> {
        let AnySyncOrStrippedTimelineEvent::Sync(sync_event) = event else {
            return None;
        };
        let AnySyncTimelineEvent::MessageLike(message_event) = &**sync_event else {
            return None;
        };

        match message_event.original_content()? {
            AnyMessageLikeEventContent::RtcNotification(content) => Some(Self::IncomingCall {
                video: content.call_intent == Some(CallIntent::Video),
            }),
            _ => None,
        }
    }

    /// What the given event says, if it is an invite to our own user.
    fn of_invite(event: &AnySyncOrStrippedTimelineEvent, own_user_id: &UserId) -> Option<Self> {
        let (membership, state_key) = match event {
            AnySyncOrStrippedTimelineEvent::Sync(sync_event) => {
                let AnySyncTimelineEvent::State(AnySyncStateEvent::RoomMember(member_event)) =
                    &**sync_event
                else {
                    return None;
                };

                match member_event {
                    SyncStateEvent::Original(original_event) => (
                        &original_event.content.membership,
                        &original_event.state_key,
                    ),
                    SyncStateEvent::Redacted(redacted_event) => (
                        &redacted_event.content.membership,
                        &redacted_event.state_key,
                    ),
                }
            }
            AnySyncOrStrippedTimelineEvent::Stripped(stripped_event) => {
                let AnyStrippedStateEvent::RoomMember(member_event) = &**stripped_event else {
                    return None;
                };

                (&member_event.content.membership, &member_event.state_key)
            }
        };

        (*membership == MembershipState::Invite && state_key == own_user_id).then_some(Self::Invite)
    }
}

#[cfg(test)]
mod tests {
    use ruma::{
        events::{AnyStrippedStateEvent, AnySyncTimelineEvent},
        serde::Raw,
        user_id,
    };
    use serde_json::json;

    use super::*;

    fn sync_event(json: serde_json::Value) -> AnySyncOrStrippedTimelineEvent {
        let raw = serde_json::from_value::<Raw<AnySyncTimelineEvent>>(json).unwrap();
        AnySyncOrStrippedTimelineEvent::Sync(Box::new(raw.deserialize().unwrap()))
    }

    fn stripped_event(json: serde_json::Value) -> AnySyncOrStrippedTimelineEvent {
        let raw = serde_json::from_value::<Raw<AnyStrippedStateEvent>>(json).unwrap();
        AnySyncOrStrippedTimelineEvent::Stripped(Box::new(raw.deserialize().unwrap()))
    }

    fn message(msgtype: &str, body: &str) -> AnySyncOrStrippedTimelineEvent {
        let mut content = json!({ "msgtype": msgtype, "body": body });
        if matches!(msgtype, "m.image" | "m.video") {
            content["url"] = json!("mxc://example.org/media");
        }
        sync_event(json!({
            "type": "m.room.message",
            "event_id": "$message",
            "sender": "@alice:example.org",
            "origin_server_ts": 1,
            "content": content,
        }))
    }

    #[test]
    fn a_text_keeps_its_body_and_a_media_message_its_kind() {
        let own = user_id!("@me:example.org");

        assert_eq!(
            NotificationBody::of(&message("m.text", "hello"), own),
            Some(NotificationBody::Text("hello".to_owned()))
        );
        assert_eq!(
            NotificationBody::of(&message("m.emote", "waves"), own),
            Some(NotificationBody::Emote("waves".to_owned()))
        );
        assert_eq!(
            NotificationBody::of(&message("m.image", "cat.png"), own),
            Some(NotificationBody::Image)
        );
        assert_eq!(
            NotificationBody::of(&message("m.video", "cat.mp4"), own),
            Some(NotificationBody::Video)
        );
    }

    #[test]
    fn a_reply_fallback_is_removed_from_a_text() {
        let own = user_id!("@me:example.org");
        let event = sync_event(json!({
            "type": "m.room.message",
            "event_id": "$reply",
            "sender": "@alice:example.org",
            "origin_server_ts": 1,
            "content": {
                "msgtype": "m.text",
                "body": "> <@bob:example.org> earlier\n\nlater",
                "m.relates_to": { "m.in_reply_to": { "event_id": "$earlier" } },
            },
        }));

        assert_eq!(
            NotificationBody::of(&event, own),
            Some(NotificationBody::Text("later".to_owned()))
        );
    }

    #[test]
    fn an_invite_says_so_only_for_our_own_user() {
        let own = user_id!("@me:example.org");
        let invite = |state_key: &str| {
            stripped_event(json!({
                "type": "m.room.member",
                "sender": "@alice:example.org",
                "state_key": state_key,
                "content": { "membership": "invite" },
            }))
        };

        assert_eq!(
            NotificationBody::of(&invite("@me:example.org"), own),
            Some(NotificationBody::Invite)
        );
        assert_eq!(NotificationBody::of(&invite("@bob:example.org"), own), None);
    }

    #[test]
    fn an_rtc_notification_says_whether_the_call_has_video() {
        let own = user_id!("@me:example.org");
        let notification = |intent: Option<&str>| {
            let mut content = json!({
                "m.mentions": { "room": true },
                "notification_type": "ring",
                "lifetime": 30000,
                "sender_ts": 1,
            });
            if let Some(intent) = intent {
                content["m.call.intent"] = json!(intent);
            }
            sync_event(json!({
                "type": "m.rtc.notification",
                "event_id": "$call",
                "sender": "@alice:example.org",
                "origin_server_ts": 1,
                "content": content,
            }))
        };

        assert_eq!(
            NotificationBody::of(&notification(Some("video")), own),
            Some(NotificationBody::IncomingCall { video: true })
        );
        assert_eq!(
            NotificationBody::of(&notification(None), own),
            Some(NotificationBody::IncomingCall { video: false })
        );
    }

    #[test]
    fn a_call_invite_is_the_calls_module_s_and_says_nothing() {
        let own = user_id!("@me:example.org");
        let event = sync_event(json!({
            "type": "m.call.invite",
            "event_id": "$invite",
            "sender": "@alice:example.org",
            "origin_server_ts": 1,
            "content": {
                "call_id": "call",
                "lifetime": 60000,
                "version": "1",
                "offer": { "type": "offer", "sdp": "v=0" },
            },
        }));

        assert!(NotificationBody::is_call_invite(&event));
        assert_eq!(NotificationBody::of(&event, own), None);
    }
}
