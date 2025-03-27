use gettextrs::gettext;
use gtk::{gdk, gio, glib, prelude::*, subclass::prelude::*};
use matrix_sdk::{sync::Notification, Room as MatrixRoom};
use ruma::{api::client::device::get_device, OwnedRoomId, RoomId};
use tracing::{debug, warn};

mod notifications_settings;

pub use self::notifications_settings::{
    NotificationsGlobalSetting, NotificationsRoomSetting, NotificationsSettings,
};
use super::{IdentityVerification, Session, VerificationKey};
use crate::{
    gettext_f,
    intent::{SessionIntent, SessionIntentType},
    prelude::*,
    spawn_tokio,
    utils::matrix::{
        get_event_body, AnySyncOrStrippedTimelineEvent, MatrixEventIdUri, MatrixIdUri,
        MatrixRoomIdUri,
    },
    Application, Window,
};

mod imp {
    use std::{
        cell::RefCell,
        collections::{HashMap, HashSet},
    };

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::Notifications)]
    pub struct Notifications {
        /// The current session.
        #[property(get, set = Self::set_session, explicit_notify, nullable)]
        pub session: glib::WeakRef<Session>,
        /// The push notifications that were presented.
        ///
        /// A map of room ID to list of notification IDs.
        pub push: RefCell<HashMap<OwnedRoomId, HashSet<String>>>,
        /// The identity verification notifications that were presented.
        ///
        /// A map of verification key to notification ID.
        pub identity_verifications: RefCell<HashMap<VerificationKey, String>>,
        /// The notifications settings for this session.
        #[property(get)]
        pub settings: NotificationsSettings,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Notifications {
        const NAME: &'static str = "Notifications";
        type Type = super::Notifications;
    }

    #[glib::derived_properties]
    impl ObjectImpl for Notifications {}

    impl Notifications {
        /// Set the current session.
        fn set_session(&self, session: Option<&Session>) {
            if self.session.upgrade().as_ref() == session {
                return;
            }

            self.session.set(session);
            self.obj().notify_session();

            self.settings.set_session(session);
        }
    }
}

glib::wrapper! {
    /// The notifications of a `Session`.
    pub struct Notifications(ObjectSubclass<imp::Notifications>);
}

impl Notifications {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Whether notifications are enabled for the current session.
    pub fn enabled(&self) -> bool {
        let settings = self.settings();
        settings.account_enabled() && settings.session_enabled()
    }

    /// Helper method to create notification
    fn send_notification(
        id: &str,
        title: &str,
        body: &str,
        default_action: (&str, glib::Variant),
        icon: Option<&gdk::Texture>,
    ) {
        let notification = gio::Notification::new(title);
        notification.set_category(Some("im.received"));
        notification.set_priority(gio::NotificationPriority::High);
        notification.set_body(Some(body));

        let (action, target_value) = default_action;
        notification.set_default_action_and_target_value(action, Some(&target_value));

        if let Some(notification_icon) = icon {
            notification.set_icon(notification_icon);
        }

        Application::default().send_notification(Some(id), &notification);
    }

    /// Ask the system to show the given push notification, if applicable.
    ///
    /// The notification will not be shown if the application is active and the
    /// room of the event is displayed.
    pub async fn show_push(&self, matrix_notification: Notification, matrix_room: MatrixRoom) {
        // Do not show notifications if they are disabled.
        if !self.enabled() {
            return;
        }

        let Some(session) = self.session() else {
            return;
        };

        let app = Application::default();
        let window = app.active_window().and_downcast::<Window>();
        let session_id = session.session_id();
        let room_id = matrix_room.room_id();

        // Do not show notifications for the current room in the current session if the
        // window is active.
        if window.is_some_and(|w| {
            w.is_active()
                && w.current_session_id().as_deref() == Some(session_id)
                && w.session_view()
                    .selected_room()
                    .is_some_and(|r| r.room_id() == room_id)
        }) {
            return;
        }

        let Some(room) = session.room_list().get(room_id) else {
            warn!("Could not display notification for missing room {room_id}",);
            return;
        };

        let event = match AnySyncOrStrippedTimelineEvent::from_raw(&matrix_notification.event) {
            Ok(event) => event,
            Err(error) => {
                warn!(
                    "Could not display notification for unrecognized event in room {room_id}: {error}",
                );
                return;
            }
        };

        let is_direct = room.direct_member().is_some();
        let sender_id = event.sender();
        let owned_sender_id = sender_id.to_owned();
        let handle =
            spawn_tokio!(async move { matrix_room.get_member_no_sync(&owned_sender_id).await });

        let sender = match handle.await.unwrap() {
            Ok(member) => member,
            Err(error) => {
                warn!("Could not get member for notification: {error}");
                None
            }
        };

        let sender_name = sender.as_ref().map_or_else(
            || sender_id.localpart().to_owned(),
            |member| {
                let name = member.name();

                if member.name_ambiguous() {
                    format!("{name} ({})", member.user_id())
                } else {
                    name.to_owned()
                }
            },
        );

        let Some(body) = get_event_body(&event, &sender_name, session.user_id(), !is_direct) else {
            debug!("Received notification for event of unexpected type {event:?}",);
            return;
        };

        let room_id = room.room_id().to_owned();
        let event_id = event.event_id();

        let room_uri = MatrixRoomIdUri {
            id: room_id.clone().into(),
            via: vec![],
        };
        let matrix_uri = if let Some(event_id) = event_id {
            MatrixIdUri::Event(MatrixEventIdUri {
                event_id: event_id.to_owned(),
                room_uri,
            })
        } else {
            MatrixIdUri::Room(room_uri)
        };

        let id = if event_id.is_some() {
            format!("{session_id}//{matrix_uri}")
        } else {
            let random_id = glib::uuid_string_random();
            format!("{session_id}//{matrix_uri}//{random_id}")
        };
        let payload =
            SessionIntent::ShowMatrixId(matrix_uri).to_variant_with_session_id(session_id);
        let icon = room.avatar_data().as_notification_icon().await;

        Self::send_notification(
            &id,
            &room.display_name(),
            &body,
            (
                SessionIntentType::ShowMatrixId.app_action_name(),
                payload.to_variant(),
            ),
            icon.as_ref(),
        );

        self.imp()
            .push
            .borrow_mut()
            .entry(room_id)
            .or_default()
            .insert(id);
    }

    /// Show a notification for the given in-room identity verification.
    pub async fn show_in_room_identity_verification(&self, verification: &IdentityVerification) {
        // Do not show notifications if they are disabled.
        if !self.enabled() {
            return;
        }

        let Some(session) = self.session() else {
            return;
        };
        let Some(room) = verification.room() else {
            return;
        };

        let room_id = room.room_id().to_owned();
        let session_id = session.session_id();
        let flow_id = verification.flow_id();

        // In-room verifications should only happen for other users.
        let user = verification.user();
        let user_id = user.user_id();

        let title = gettext("Verification Request");
        let body = gettext_f(
            // Translators: Do NOT translate the content between '{' and '}', this is a
            // variable name.
            "{user} sent a verification request",
            &[("user", &user.display_name())],
        );

        let payload = SessionIntent::ShowIdentityVerification(verification.key())
            .to_variant_with_session_id(session_id);

        let icon = user.avatar_data().as_notification_icon().await;

        let id = format!("{session_id}//{room_id}//{user_id}//{flow_id}");
        Self::send_notification(
            &id,
            &title,
            &body,
            (
                SessionIntentType::ShowIdentityVerification.app_action_name(),
                payload.to_variant(),
            ),
            icon.as_ref(),
        );

        self.imp()
            .identity_verifications
            .borrow_mut()
            .insert(verification.key(), id);
    }

    /// Show a notification for the given to-device identity verification.
    pub async fn show_to_device_identity_verification(&self, verification: &IdentityVerification) {
        // Do not show notifications if they are disabled.
        if !self.enabled() {
            return;
        }

        let Some(session) = self.session() else {
            return;
        };
        // To-device verifications should only happen for other sessions.
        let Some(other_device_id) = verification.other_device_id() else {
            return;
        };

        let session_id = session.session_id();
        let flow_id = verification.flow_id();

        let client = session.client();
        let request = get_device::v3::Request::new(other_device_id.clone());
        let handle = spawn_tokio!(async move { client.send(request).await });

        let display_name = match handle.await.unwrap() {
            Ok(res) => res.device.display_name,
            Err(error) => {
                warn!("Could not get device for notification: {error}");
                None
            }
        };
        let display_name = display_name
            .as_deref()
            .unwrap_or_else(|| other_device_id.as_str());

        let title = gettext("Login Request From Another Session");
        let body = gettext_f(
            // Translators: Do NOT translate the content between '{' and '}', this is a
            // variable name.
            "Verify your new session “{name}”",
            &[("name", display_name)],
        );

        let payload = SessionIntent::ShowIdentityVerification(verification.key())
            .to_variant_with_session_id(session_id);

        let id = format!("{session_id}//{other_device_id}//{flow_id}");

        Self::send_notification(
            &id,
            &title,
            &body,
            (
                SessionIntentType::ShowIdentityVerification.app_action_name(),
                payload.to_variant(),
            ),
            None,
        );

        self.imp()
            .identity_verifications
            .borrow_mut()
            .insert(verification.key(), id);
    }

    /// Ask the system to remove the known notifications for the room with the
    /// given ID.
    ///
    /// Only the notifications that were shown since the application's startup
    /// are known, older ones might still be present.
    pub fn withdraw_all_for_room(&self, room_id: &RoomId) {
        if let Some(notifications) = self.imp().push.borrow_mut().remove(room_id) {
            let app = Application::default();

            for id in notifications {
                app.withdraw_notification(&id);
            }
        }
    }

    /// Ask the system to remove the known notification for the identity
    /// verification with the given key.
    pub fn withdraw_identity_verification(&self, key: &VerificationKey) {
        if let Some(id) = self.imp().identity_verifications.borrow_mut().remove(key) {
            let app = Application::default();
            app.withdraw_notification(&id);
        }
    }

    /// Ask the system to remove all the known notifications for this session.
    ///
    /// Only the notifications that were shown since the application's startup
    /// are known, older ones might still be present.
    pub fn clear(&self) {
        let app = Application::default();

        for id in self.imp().push.take().values().flatten() {
            app.withdraw_notification(id);
        }
        for id in self.imp().identity_verifications.take().values() {
            app.withdraw_notification(id);
        }
    }
}

impl Default for Notifications {
    fn default() -> Self {
        Self::new()
    }
}
