use gtk::{gio, glib, prelude::*, subclass::prelude::*};
use matrix_sdk::{sync::Notification, Room as MatrixRoom};
use ruma::{EventId, OwnedRoomId, RoomId};
use tracing::{debug, error, warn};

mod notifications_settings;

pub use self::notifications_settings::{
    NotificationsGlobalSetting, NotificationsRoomSetting, NotificationsSettings,
};
use super::{Room, Session};
use crate::{
    intent,
    prelude::*,
    spawn_tokio,
    utils::matrix::{get_event_body, AnySyncOrStrippedTimelineEvent},
    Application, Window,
};

mod imp {
    use std::{cell::RefCell, collections::HashMap};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::Notifications)]
    pub struct Notifications {
        /// The current session.
        #[property(get, set = Self::set_session, explicit_notify, nullable)]
        pub session: glib::WeakRef<Session>,
        /// A map of room ID to list of notification IDs.
        pub list: RefCell<HashMap<OwnedRoomId, Vec<String>>>,
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

    /// Ask the system to show the given notification, if applicable.
    ///
    /// The notification won't be shown if the application is active and this
    /// session is displayed.
    pub async fn show(&self, matrix_notification: Notification, matrix_room: MatrixRoom) {
        let Some(session) = self.session() else {
            return;
        };

        // Don't show notifications if they are disabled.
        if !session.settings().notifications_enabled() {
            return;
        }

        let app = Application::default();
        let window = app.active_window().and_downcast::<Window>();
        let session_id = session.session_id();
        let room_id = matrix_room.room_id();

        // Don't show notifications for the current room in the current session if the
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
                error!("Could not get member for notification: {error}");
                None
            }
        };

        let sender_name = sender
            .as_ref()
            .map(|member| {
                let name = member.name();

                if member.name_ambiguous() {
                    format!("{name} ({})", member.user_id())
                } else {
                    name.to_owned()
                }
            })
            .unwrap_or_else(|| sender_id.localpart().to_owned());

        let body = match get_event_body(&event, &sender_name, session.user_id(), !is_direct) {
            Some(body) => body,
            None => {
                debug!("Received notification for event of unexpected type {event:?}",);
                return;
            }
        };

        let room_id = room.room_id().to_owned();
        let event_id = event.event_id();

        let notification = gio::Notification::new(&room.display_name());
        notification.set_priority(gio::NotificationPriority::High);

        let payload = intent::ShowRoomPayload {
            session_id: session_id.to_owned(),
            room_id: room_id.clone(),
        };

        notification
            .set_default_action_and_target_value("app.show-room", Some(&payload.to_variant()));
        notification.set_body(Some(&body));

        if let Some(icon) = room.avatar_data().as_notification_icon() {
            notification.set_icon(&icon);
        }

        let id = notification_id(session_id, &room_id, event_id);
        app.send_notification(Some(&id), &notification);

        self.imp()
            .list
            .borrow_mut()
            .entry(room_id)
            .or_default()
            .push(id);
    }

    /// Ask the system to remove the known notifications for the given room.
    ///
    /// Only the notifications that were shown since the application's startup
    /// are known, older ones might still be present.
    pub fn withdraw_all_for_room(&self, room: &Room) {
        let room_id = room.room_id();
        if let Some(notifications) = self.imp().list.borrow_mut().remove(room_id) {
            let app = Application::default();

            for id in notifications {
                app.withdraw_notification(&id);
            }
        }
    }

    /// Ask the system to remove all the known notifications for this session.
    ///
    /// Only the notifications that were shown since the application's startup
    /// are known, older ones might still be present.
    pub fn clear(&self) {
        let app = Application::default();

        for id in self.imp().list.take().values().flatten() {
            app.withdraw_notification(id);
        }
    }
}

impl Default for Notifications {
    fn default() -> Self {
        Self::new()
    }
}

fn notification_id(session_id: &str, room_id: &RoomId, event_id: Option<&EventId>) -> String {
    if let Some(event_id) = event_id {
        format!("{session_id}//{room_id}//{event_id}")
    } else {
        let random_id = glib::uuid_string_random();
        format!("{session_id}//{room_id}//{random_id}")
    }
}
