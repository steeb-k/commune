use std::collections::HashMap;

use adw::{prelude::*, subclass::prelude::*};
use futures_util::{StreamExt, future, lock::Mutex, pin_mut};
use gettextrs::{gettext, pgettext};
use gtk::{gdk, gio, glib, glib::clone};
use matrix_sdk::{
    Client,
    attachment::{AttachmentInfo, BaseFileInfo, Thumbnail},
    room::edit::EditedContent,
};
use matrix_sdk_ui::timeline::{
    AttachmentConfig, AttachmentSource, TimelineEventItemId, TimelineItemContent,
};
use ruma::{
    OwnedEventId, OwnedRoomId,
    events::{
        AnyMessageLikeEventContent, Mentions,
        room::{
            ImageInfo,
            message::{LocationMessageEventContent, MessageType, RoomMessageEventContent},
            tombstone::RoomTombstoneEventContent,
        },
        sticker::{StickerEventContent, StickerMediaSource},
    },
};
use tracing::{debug, error, warn};

mod attachment_dialog;
mod completion;
mod composer_parser;
mod composer_state;
mod sticker_picker;
mod voice_recorder;

pub(crate) use self::composer_state::{ComposerState, MessageEventSource, RelationInfo};
use self::{
    attachment_dialog::{AttachmentDialog, AttachmentResponse},
    completion::CompletionPopover,
    composer_parser::ComposerParser,
    sticker_picker::StickerPicker,
    voice_recorder::{VoiceRecorder, VoiceRecorderError},
};
use super::message_row::MessageContent;
use crate::{
    Application, Window,
    components::{AvatarImageSafetySetting, CustomEntry, LabelWithWidgets, LoadingButton},
    gettext_f,
    prelude::*,
    session::{Event, Member, PackImage, Room, RoomListRoomInfo, Timeline},
    spawn, spawn_tokio, toast,
    utils::{
        File, Location, LocationError, TemplateCallbacks, TokioDrop, http,
        klipy::{self, SelectedGif},
        local_path,
        media::{
            FileInfo, audio::load_audio_info, filename_for_mime, image::ImageInfoLoader,
            video::load_video_info,
        },
        save_data_to_tmp_file,
    },
};

/// A map of composer state per-session, then per-room and thread.
///
/// A thread keeps a composer state of its own — the state is where the
/// half-typed draft and the reply selection live, and a draft typed for the
/// room must not be one thread-open away from being sent into a thread.
type ComposerStatesMap =
    HashMap<Option<String>, HashMap<Option<(OwnedRoomId, Option<OwnedEventId>)>, ComposerState>>;

/// The available stack pages of the [`MessageToolbar`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MessageToolbarPage {
    /// The composer and other buttons to send messages.
    Composer,
    /// A voice message is being recorded.
    VoiceRecording,
    /// The user is not allowed to send messages in the room.
    NoPermission,
    /// The room was tombstoned.
    Tombstoned,
}

impl MessageToolbarPage {
    /// The name of this page.
    const fn name(self) -> &'static str {
        match self {
            Self::Composer => "composer",
            Self::VoiceRecording => "voice-recording",
            Self::NoPermission => "no-permission",
            Self::Tombstoned => "tombstoned",
        }
    }

    /// Get the page matching the given name.
    ///
    /// Panics if the name does not match any variant.
    fn from_name(name: &str) -> Self {
        match name {
            "composer" => Self::Composer,
            "voice-recording" => Self::VoiceRecording,
            "no-permission" => Self::NoPermission,
            "tombstoned" => Self::Tombstoned,
            _ => panic!("Unknown MessageToolbarPage: {name}"),
        }
    }
}

/// How sending one file of a batch ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SendOutcome {
    /// Sent; the next file is previewed.
    Sent,
    /// Sent, and the rest go without a preview.
    SentAll,
    /// Could not be read; the next file is still previewed.
    Failed,
    /// The person cancelled; the rest are dropped.
    Cancelled,
}

mod imp {
    use std::{
        cell::{Cell, RefCell},
        marker::PhantomData,
    };

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session_view/room_history/message_toolbar/mod.ui")]
    #[properties(wrapper_type = super::MessageToolbar)]
    pub struct MessageToolbar {
        #[template_child]
        main_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub(super) message_entry: TemplateChild<sourceview::View>,
        #[template_child]
        attach_button: TemplateChild<gtk::Button>,
        #[template_child]
        emoji_button: TemplateChild<gtk::Button>,
        #[template_child]
        more_button: TemplateChild<gtk::MenuButton>,
        #[template_child]
        action_row: TemplateChild<gtk::Box>,
        #[template_child]
        toolbar_row: TemplateChild<gtk::Box>,
        #[template_child]
        voice_button: TemplateChild<gtk::Button>,
        #[template_child]
        recording_elapsed_label: TemplateChild<gtk::Label>,
        #[template_child]
        sticker_button: TemplateChild<gtk::MenuButton>,
        #[template_child]
        sticker_picker: TemplateChild<StickerPicker>,
        #[template_child]
        send_button: TemplateChild<gtk::Button>,
        #[template_child]
        related_event_header: TemplateChild<LabelWithWidgets>,
        #[template_child]
        related_event_content: TemplateChild<MessageContent>,
        #[template_child]
        tombstoned_label: TemplateChild<gtk::Label>,
        #[template_child]
        tombstoned_button: TemplateChild<LoadingButton>,
        /// The timeline used to send messages.
        #[property(get, set = Self::set_timeline, explicit_notify, nullable)]
        timeline: glib::WeakRef<Timeline>,
        successor_room_list_info: RoomListRoomInfo,
        room_handlers: RefCell<Vec<glib::SignalHandlerId>>,
        send_message_permission_handler: RefCell<Option<glib::SignalHandlerId>>,
        send_sticker_permission_handler: RefCell<Option<glib::SignalHandlerId>>,
        /// Whether non-text messages can be sent in the current state.
        can_send_non_text_messages: Cell<bool>,
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
        /// The recorder of a voice message, while one is being recorded.
        voice_recorder: RefCell<Option<VoiceRecorder>>,
        /// The binding of the elapsed time of the recording to its label.
        voice_elapsed_binding: RefCell<Option<glib::Binding>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MessageToolbar {
        const NAME: &'static str = "MessageToolbar";
        type Type = super::MessageToolbar;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            CustomEntry::ensure_type();
            StickerPicker::ensure_type();

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

            // Six widgets do not fit beside a typing area on a phone — the
            // send button was the one pushed off the screen — so the
            // auxiliary buttons move to a row of their own above the entry,
            // and the row that types keeps only the entry and Send. Done here
            // rather than with a breakpoint because on Android narrow is not
            // a window state, it is the shape of the device.
            #[cfg(target_os = "android")]
            {
                for button in [
                    self.attach_button.upcast_ref::<gtk::Widget>(),
                    self.emoji_button.upcast_ref(),
                    self.sticker_button.upcast_ref(),
                    self.more_button.upcast_ref(),
                ] {
                    self.toolbar_row.remove(button);
                    self.action_row.append(button);
                }
                // The overflow menu reads best at the far end of its row.
                self.more_button.set_hexpand(true);
                self.more_button.set_halign(gtk::Align::End);
                self.action_row.set_visible(true);
            }

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

            // Stickers.
            self.sticker_picker.connect_sticker_selected(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_, image| {
                    spawn!(clone!(
                        #[weak]
                        imp,
                        async move {
                            imp.send_sticker(&image).await;
                        }
                    ));
                }
            ));

            // GIFs.
            self.sticker_picker.connect_gif_selected(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_, gif| {
                    spawn!(clone!(
                        #[weak]
                        imp,
                        async move {
                            imp.send_gif(gif).await;
                        }
                    ));
                }
            ));

            // Listen to changes in the room list for the successor.
            self.successor_room_list_info
                .connect_is_joining_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_tombstoned_page();
                    }
                ));
            self.successor_room_list_info
                .connect_local_room_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_tombstoned_page();
                    }
                ));
        }

        fn dispose(&self) {
            self.completion.unparent();
            self.disconnect_signals();
        }
    }

    impl WidgetImpl for MessageToolbar {
        fn grab_focus(&self) -> bool {
            let Some(visible_page) = self
                .main_stack
                .visible_child_name()
                .map(|name| MessageToolbarPage::from_name(&name))
            else {
                return false;
            };

            match visible_page {
                MessageToolbarPage::Composer => self.message_entry.grab_focus(),
                MessageToolbarPage::VoiceRecording | MessageToolbarPage::NoPermission => false,
                MessageToolbarPage::Tombstoned => {
                    if self.tombstoned_button.is_visible() {
                        self.tombstoned_button.grab_focus()
                    } else {
                        false
                    }
                }
            }
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

            // A recording does not follow across rooms or threads.
            if let Some(recorder) = self.voice_recorder.borrow().clone() {
                recorder.cancel();
            }
            self.reset_voice_recording();

            self.disconnect_signals();

            if let Some(timeline) = timeline {
                let room = timeline.room();

                let is_tombstoned_handler = room.connect_is_tombstoned_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_visible_page();
                    }
                ));
                let successor_id_handler = room.connect_successor_id_string_notify(clone!(
                    #[weak(rename_to= imp)]
                    self,
                    move |_| {
                        imp.update_successor_identifier();
                        imp.update_tombstoned_page();
                    }
                ));
                let successor_handler = room.connect_successor_notify(clone!(
                    #[weak(rename_to= imp)]
                    self,
                    move |_| {
                        imp.update_tombstoned_page();
                    }
                ));
                self.room_handlers.replace(vec![
                    is_tombstoned_handler,
                    successor_id_handler,
                    successor_handler,
                ]);

                let send_message_permission_handler = timeline
                    .room()
                    .permissions()
                    .connect_can_send_message_notify(clone!(
                        #[weak(rename_to = imp)]
                        self,
                        move |_| {
                            imp.update_visible_page();
                        }
                    ));
                self.send_message_permission_handler
                    .replace(Some(send_message_permission_handler));

                let send_sticker_permission_handler = timeline
                    .room()
                    .permissions()
                    .connect_can_send_sticker_notify(clone!(
                        #[weak(rename_to = imp)]
                        self,
                        move |_| {
                            imp.update_sticker_button();
                        }
                    ));
                self.send_sticker_permission_handler
                    .replace(Some(send_sticker_permission_handler));

                if let Some(session) = room.session() {
                    self.successor_room_list_info
                        .set_room_list(session.room_list());
                }
            }

            self.completion.set_room(timeline.map(Timeline::room));
            self.sticker_picker
                .set_room(timeline.map(Timeline::room).as_ref());
            self.timeline.set(timeline);

            self.update_successor_identifier();
            self.update_tombstoned_page();
            self.update_visible_page();

            obj.notify_timeline();
            self.update_current_composer_state(old_timeline);
        }

        /// The stack page that should be presented given the current state.
        fn visible_page(&self) -> MessageToolbarPage {
            let Some(room) = self.timeline.upgrade().map(|timeline| timeline.room()) else {
                return MessageToolbarPage::NoPermission;
            };

            if room.is_tombstoned() {
                MessageToolbarPage::Tombstoned
            } else if room.permissions().can_send_message() {
                if self.voice_recorder.borrow().is_some() {
                    MessageToolbarPage::VoiceRecording
                } else {
                    MessageToolbarPage::Composer
                }
            } else {
                MessageToolbarPage::NoPermission
            }
        }

        /// Whether the user can compose a message.
        ///
        /// It depends on whether our own user has the permission to send a
        /// message in the current room.
        pub(super) fn can_compose_message(&self) -> bool {
            self.visible_page() == MessageToolbarPage::Composer
        }

        /// Update the visible stack page.
        fn update_visible_page(&self) {
            let page = self.visible_page();

            if page != MessageToolbarPage::VoiceRecording && self.voice_recorder.borrow().is_some()
            {
                // The state changed under the recording — the permission went,
                // or the room was tombstoned. Nothing sensible can be done
                // with the recording, so it is dropped.
                if let Some(recorder) = self.voice_recorder.borrow().clone() {
                    recorder.cancel();
                }
                self.reset_voice_recording();
            }

            self.main_stack.set_visible_child_name(page.name());
        }

        /// Update the identifier to watch for the successor of the current
        /// room.
        fn update_successor_identifier(&self) {
            let successor_id = self
                .timeline
                .upgrade()
                .and_then(|timeline| timeline.room().successor_id());
            self.successor_room_list_info
                .set_identifiers(successor_id.into_iter().map(Into::into).collect());
        }

        /// Update the tombstoned stack page.
        fn update_tombstoned_page(&self) {
            let Some(room) = self.timeline.upgrade().map(|timeline| timeline.room()) else {
                return;
            };

            // A "real" successor must have the current room as a predecessor. We still want
            // to show the "View" button if it is only the room that matches the successor
            // ID.
            let has_successor_room =
                room.successor().is_some() || self.successor_room_list_info.local_room().is_some();
            let has_successor_id = room.successor_id().is_some();
            let has_successor = has_successor_room || has_successor_id;

            // Update description.
            let description = if has_successor {
                gettext("The conversation continues in a new room")
            } else {
                gettext("The conversation has ended")
            };
            self.tombstoned_label.set_label(&description);

            // Update button.
            if has_successor {
                let label = if has_successor_room {
                    // Translators: This is a verb, as in 'View Room'.
                    gettext("View")
                } else {
                    gettext("Join")
                };
                self.tombstoned_button.set_content_label(label);

                let is_joining_successor = self.successor_room_list_info.is_joining();
                self.tombstoned_button.set_is_loading(is_joining_successor);
            }
            self.tombstoned_button.set_visible(has_successor);
        }

        /// Whether the buffer of the composer is empty.
        ///
        /// Returns `true` if the buffer is empty or contains only whitespace.
        fn is_buffer_empty(&self) -> bool {
            let mut iter = self.message_entry.buffer().start_iter();

            if iter.is_end() {
                return true;
            }

            loop {
                if !iter.char().is_whitespace() {
                    // The buffer is not empty.
                    return false;
                }

                if !iter.forward_cursor_position() {
                    // We are at the end and we did not encounter a non-whitespace character, the
                    // buffer is empty.
                    return true;
                }
            }
        }

        /// The current composer state.
        fn current_composer_state(&self) -> ComposerState {
            let timeline = self.timeline.upgrade();
            self.composer_state(timeline)
        }

        /// The composer state for the given timeline.
        ///
        /// A room and a thread within it have composer states of their own.
        ///
        /// If the composer state does not exist, it is created.
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
                .entry(room.map(|room| {
                    (
                        room.room_id().to_owned(),
                        timeline.as_ref().and_then(Timeline::thread_root),
                    )
                }))
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
                move |_| {
                    let is_empty = imp.is_buffer_empty();
                    imp.send_button.set_sensitive(!is_empty);
                    imp.send_typing_notification(!is_empty);
                }
            ));

            let is_empty = self.is_buffer_empty();
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
                Some(RelationInfo::Reply(event)) => {
                    self.update_for_reply(&event);
                    self.enable_sending_non_text_messages(false);
                }
                Some(RelationInfo::Edit(_)) => {
                    self.update_for_edit();
                    self.enable_sending_non_text_messages(false);
                }
                None => {
                    self.enable_sending_non_text_messages(true);
                }
            }
        }

        /// Update the displayed related event for the given replied-to event.
        fn update_for_reply(&self, message_event: &MessageEventSource) {
            let Some(msgtype) = message_event.msgtype() else {
                // The event was probably redacted, we cannot reply to it anymore.
                self.clear_related_event();
                return;
            };
            let Some(timeline) = self.timeline.upgrade() else {
                return;
            };

            let room = timeline.room();
            let sender = room
                .get_or_create_members()
                .get_or_create(message_event.sender());

            let label = gettext_f(
                // Translators: Do NOT translate the content between '{' and '}',
                // this is a variable name. In this string, 'Reply' is a noun.
                "Reply to {user}",
                &[("user", LabelWithWidgets::PLACEHOLDER)],
            );
            // We do not need to watch safety settings for mentions, rooms will be watched
            // automatically.
            let pill = sender.to_pill(AvatarImageSafetySetting::None, None);

            self.related_event_header
                .set_label_and_widgets(label, vec![pill]);

            self.related_event_content
                .update_for_related_event(&msgtype, message_event, &sender);
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

        /// Toggle UI for sending non-text messages.
        fn enable_sending_non_text_messages(&self, enable: bool) {
            self.attach_button.set_sensitive(enable);
            self.voice_button.set_sensitive(enable);
            self.can_send_non_text_messages.set(enable);
            self.update_sticker_button();
            self.obj().action_set_enabled(
                "message-toolbar.send-location",
                enable && Location::new().is_available(),
            );
        }

        /// Start recording a voice message.
        #[template_callback]
        fn start_voice_recording(&self) {
            if !self.can_compose_message() || !self.can_send_non_text_messages.get() {
                return;
            }
            if self.voice_recorder.borrow().is_some() {
                return;
            }

            let recorder = VoiceRecorder::new();

            recorder.connect_failed(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_, no_microphone| {
                    // On some platforms a missing microphone only surfaces
                    // here, after the pipeline accepted starting.
                    let message = if no_microphone {
                        gettext("No microphone could be opened")
                    } else {
                        gettext("Could not record a voice message")
                    };
                    toast!(imp.obj(), message);
                    imp.reset_voice_recording();
                    imp.update_visible_page();
                }
            ));

            match recorder.start() {
                Ok(()) => {
                    let binding = recorder
                        .bind_property("elapsed", &*self.recording_elapsed_label, "label")
                        .sync_create()
                        .build();
                    self.voice_elapsed_binding.replace(Some(binding));
                    self.voice_recorder.replace(Some(recorder));
                    self.update_visible_page();
                }
                Err(VoiceRecorderError::NoMicrophone) => {
                    toast!(self.obj(), gettext("No microphone could be opened"));
                }
                Err(VoiceRecorderError::Other) => {
                    toast!(self.obj(), gettext("Could not record a voice message"));
                }
            }
        }

        /// Stop recording a voice message and throw the recording away.
        #[template_callback]
        fn cancel_voice_recording(&self) {
            if let Some(recorder) = self.voice_recorder.borrow().clone() {
                recorder.cancel();
            }

            self.reset_voice_recording();
            self.update_visible_page();
        }

        /// Forget the current voice recording.
        ///
        /// Does not touch the recorder itself: the caller decides whether the
        /// recording is cancelled or already stopped.
        fn reset_voice_recording(&self) {
            if let Some(binding) = self.voice_elapsed_binding.take() {
                binding.unbind();
            }
            self.voice_recorder.take();
        }

        /// Stop recording a voice message and send it.
        #[template_callback]
        async fn send_voice_message(&self) {
            let Some(_send_guard) = self.send_guard.try_lock() else {
                return;
            };
            let Some(recorder) = self.voice_recorder.borrow().clone() else {
                return;
            };

            let path = recorder.stop().await;
            self.reset_voice_recording();
            self.update_visible_page();

            let Ok(path) = path else {
                toast!(self.obj(), gettext("Could not record a voice message"));
                return;
            };

            // The same code that measures an audio file picked from disk
            // computes the duration and the waveform of the recording.
            let file = gio::File::for_path(&path);
            let mut base_info = load_audio_info(&file).await;

            let bytes = std::fs::read(&path);
            if let Err(error) = std::fs::remove_file(&path) {
                warn!("Could not remove the voice recording file: {error}");
            }
            let Ok(bytes) = bytes else {
                error!("Could not read the voice recording");
                toast!(self.obj(), gettext("Could not send voice message"));
                return;
            };

            base_info.size = bytes.len().try_into().ok();

            let Ok(mime) = "audio/ogg".parse::<mime::Mime>() else {
                return;
            };

            let source = AttachmentSource::Data {
                bytes,
                // Translators: This is the body of a voice message, which is
                // what other clients show as its file name.
                filename: format!("{}.ogg", gettext("Voice message")),
            };

            self.send_attachment(source, mime, AttachmentInfo::Voice(base_info), None)
                .await;
        }

        /// Update whether a sticker can be sent in the current state.
        pub(super) fn update_sticker_button(&self) {
            let has_permission = self
                .timeline
                .upgrade()
                .is_some_and(|timeline| timeline.room().permissions().can_send_sticker());

            self.sticker_button
                .set_sensitive(self.can_send_non_text_messages.get() && has_permission);
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

            // We do not need to watch safety settings for users.
            let pill = member.to_pill(AvatarImageSafetySetting::None, None);
            self.current_composer_state().add_widget(pill, &mut insert);

            self.message_entry.grab_focus();
        }

        /// Set the event to reply to.
        pub(super) fn set_reply_to(&self, event: Event) {
            if !self.can_compose_message() {
                return;
            }

            if event.event_id().is_none() {
                warn!("Cannot send reply for event that is not sent yet");
                return;
            }
            let Some(message_event) = MessageEventSource::from_event(event) else {
                warn!("Unsupported event type for reply");
                return;
            };

            self.current_composer_state()
                .set_related_to(Some(RelationInfo::Reply(message_event.into())));

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
            let TimelineItemContent::MsgLike(msg_like) = item.content() else {
                warn!("Unsupported event type for edit");
                return;
            };
            let Some(message) = msg_like.as_message() else {
                warn!("Unsupported event type for edit");
                return;
            };

            self.current_composer_state()
                .set_edit_source(event_id.to_owned(), &message);

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
            // Do not capture key press if there is a mask other than CapsLock.
            if modifier != gdk::ModifierType::NO_MODIFIER_MASK
                && modifier != gdk::ModifierType::LOCK_MASK
            {
                return glib::Propagation::Proceed;
            }

            // Send message on enter.
            if matches!(
                key,
                gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::ISO_Enter,
            ) {
                spawn!(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    async move {
                        imp.send_text_message().await;
                    }
                ));
                return glib::Propagation::Stop;
            }

            // Clear related event on escape.
            if key == gdk::Key::Escape && self.current_composer_state().has_relation() {
                self.clear_related_event();
                return glib::Propagation::Stop;
            }

            // Edit the last message on key up, if the composer is empty and the completion
            // popover is not open.
            if matches!(key, gdk::Key::Up | gdk::Key::KP_Up)
                && !self.completion.is_visible()
                && self.is_buffer_empty()
            {
                if self
                    .obj()
                    .activate_action("room-history.edit-latest-message", None)
                    .is_err()
                {
                    error!("Could not activate `room-history.edit-latest-message` action");
                }
                return glib::Propagation::Stop;
            }

            glib::Propagation::Proceed
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
                Some(RelationInfo::Reply(message_event)) => {
                    let event_id = message_event.event_id();

                    let handle =
                        spawn_tokio!(
                            async move { matrix_timeline.send_reply(content, event_id).await }
                        );

                    if let Err(error) = handle.await.expect("task was not aborted") {
                        error!("Could not send reply: {error}");
                        toast!(self.obj(), gettext("Could not send reply"));
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
                        toast!(self.obj(), gettext("Could not send edit"));
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
                        toast!(self.obj(), gettext("Could not send message"));
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
                        let _ = TokioDrop::new(location_stream);

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
                toast!(self.obj(), gettext("Could not send location"));
            }
        }

        /// Send the given image of a pack as a sticker.
        async fn send_sticker(&self, image: &PackImage) {
            let Some(timeline) = self.timeline.upgrade() else {
                return;
            };

            let content = image.sticker_content();

            let matrix_timeline = timeline.matrix_timeline();
            let handle = spawn_tokio!(async move {
                matrix_timeline
                    .send(AnyMessageLikeEventContent::Sticker(content))
                    .await
            });

            if let Err(error) = handle.await.unwrap() {
                error!("Could not send sticker: {error}");
                toast!(self.obj(), gettext("Could not send sticker"));
            }
        }

        /// Send the given GIF as a sticker.
        ///
        /// The GIF is downloaded from the service and uploaded to the
        /// homeserver, rather than linked: a link would leak the IP address of
        /// everyone in the room to the service, would rot when the service
        /// drops the file, and could not be end-to-end encrypted.
        async fn send_gif(&self, gif: SelectedGif) {
            let Some(timeline) = self.timeline.upgrade() else {
                return;
            };
            let room = timeline.room();
            let Some(session) = room.session() else {
                return;
            };
            let client = session.client();
            let is_encrypted = room.is_encrypted();

            // Through the wrapper: the fields are moved out of the core's
            // struct, and `Deref` only lends them.
            let commune_core::klipy::SelectedGif {
                url,
                width,
                height,
                size,
                slug,
                title,
            } = gif.into_inner();

            let handle = spawn_tokio!(async move {
                let data = http::fetch(&url, MAX_GIF_SIZE).await?;
                let source = upload_gif(&client, is_encrypted, data).await?;
                Ok::<_, SendGifError>(source)
            });

            let source = match handle.await.expect("task should not be aborted") {
                Ok(source) => source,
                Err(error) => {
                    error!("Could not send GIF: {error}");
                    toast!(self.obj(), gettext("Could not send GIF"));
                    return;
                }
            };

            let mut info = ImageInfo::new();
            info.width = Some(width.into());
            info.height = Some(height.into());
            info.size = size.try_into().ok();
            info.mimetype = Some(mime::IMAGE_GIF.to_string());
            // Without this the receiving client asks its homeserver for a
            // thumbnail, which is a still frame.
            info.is_animated = Some(true);

            let content = StickerEventContent::with_source(title, info, source);

            let matrix_timeline = timeline.matrix_timeline();
            let handle = spawn_tokio!(async move {
                matrix_timeline
                    .send(AnyMessageLikeEventContent::Sticker(content))
                    .await
            });

            if let Err(error) = handle.await.unwrap() {
                error!("Could not send GIF: {error}");
                toast!(self.obj(), gettext("Could not send GIF"));
                return;
            }

            // Tell the service that the GIF was shared. This is how it counts, and
            // it is why the API is free to use.
            spawn_tokio!(async move { klipy::report_share(&slug).await });
        }

        /// Show a toast for the given location error;
        fn location_error_toast(&self, error: LocationError) {
            let msg = match error {
                LocationError::Cancelled => gettext("The location request has been cancelled"),
                LocationError::Disabled => gettext("The location services are disabled"),
                LocationError::Other => gettext("Could not retrieve current location"),
            };

            toast!(self.obj(), msg);
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

            // Ask the homeserver's upload limit before sending, rather than
            // uploading the whole file to be told no at the end. The answer
            // is cached after the first ask; when it cannot be had, the
            // upload proceeds and the server stays the judge.
            let size = match &source {
                AttachmentSource::Data { bytes, .. } => u64::try_from(bytes.len()).ok(),
                AttachmentSource::File(path) => {
                    std::fs::metadata(path).ok().map(|metadata| metadata.len())
                }
            };
            if let Some(size) = size
                && let Some(session) = timeline.room().session()
            {
                let client = session.client();
                let handle =
                    spawn_tokio!(async move { client.load_or_fetch_max_upload_size().await });
                if let Ok(max_upload_size) = handle.await.expect("task was not aborted")
                    && size > u64::from(max_upload_size)
                {
                    toast!(
                        self.obj(),
                        gettext(
                            // Translators: Do NOT translate the content between '{' and '}', this
                            // is a variable name.
                            "This file is too large, the homeserver takes up to {max}",
                        ),
                        max = glib::format_size(u64::from(max_upload_size)).as_str(),
                    );
                    return;
                }
            }

            let config = AttachmentConfig {
                info: Some(info),
                thumbnail,
                ..Default::default()
            };

            let matrix_timeline = timeline.matrix_timeline();

            let handle = spawn_tokio!(async move {
                matrix_timeline
                    .send_attachment(source, mime, config)
                    .use_send_queue()
                    .await
            });

            if let Err(error) = handle.await.unwrap() {
                error!("Could not send file: {error}");
                toast!(self.obj(), gettext("Could not send file"));
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
                .title(gettext("Select Files"))
                .modal(true)
                .accept_label(gettext("Select"))
                .build();

            // Several at once: a message per file, in the order they were
            // picked, as every client sends a batch of photos.
            match dialog
                .open_multiple_future(obj.root().and_downcast_ref::<gtk::Window>())
                .await
            {
                Ok(files) => {
                    let files = (0..files.n_items())
                        .filter_map(|position| files.item(position).and_downcast::<gio::File>())
                        .collect::<Vec<_>>();
                    self.send_files_inner(files).await;
                }
                Err(error) => {
                    if error.matches(gtk::DialogError::Dismissed) {
                        debug!("File dialog dismissed by user");
                    } else {
                        error!("Could not open file: {error:?}");
                        toast!(obj, gettext("Could not open file"));
                    }
                }
            }
        }

        /// Send the given file.
        pub(super) async fn send_file(&self, file: gio::File) {
            self.send_files(vec![file]).await;
        }

        /// Send the given files, one message each, in order.
        pub(super) async fn send_files(&self, files: Vec<gio::File>) {
            let Some(_send_guard) = self.send_guard.try_lock() else {
                return;
            };
            if !self.can_compose_message() {
                return;
            }

            self.send_files_inner(files).await;
        }

        /// Send the given files, previewing each in turn until the person
        /// sends them all in one go, and stopping at the first cancel.
        ///
        /// Each upload is awaited before the next is previewed, so the
        /// messages land in the order the files were picked.
        async fn send_files_inner(&self, files: Vec<gio::File>) {
            let total = files.len();
            let mut ask = true;

            for (position, file) in files.into_iter().enumerate() {
                let remaining = total - position - 1;
                match self.send_file_inner(file, remaining, ask).await {
                    SendOutcome::Cancelled => break,
                    SendOutcome::SentAll => ask = false,
                    SendOutcome::Sent | SendOutcome::Failed => {}
                }
            }
        }

        /// Send the given file, after previewing it when `ask` is set.
        ///
        /// `remaining` is how many files wait behind this one; the preview
        /// offers to send them all with it.
        async fn send_file_inner(
            &self,
            file: gio::File,
            remaining: usize,
            ask: bool,
        ) -> SendOutcome {
            let obj = self.obj();

            #[cfg(target_os = "macos")]
            let file = crate::utils::repair_pasteboard_file(file);

            let file_info = match FileInfo::try_from_file(&file).await {
                Ok(file_info) => file_info,
                Err(error) => {
                    warn!("Could not read file info: {error}");
                    toast!(obj, gettext("Error reading file"));
                    return SendOutcome::Failed;
                }
            };

            // Everything below this wants the file as a path or as a `file://`
            // URI: the preview and the thumbnailer hand a URI to `GStreamer`,
            // and the upload hands a path to the send queue. A file the Android
            // picker returns has neither in a usable form. Its URI is
            // `content://...`, which `GStreamer` has no handler for, and its
            // path is a document id that points at nothing, which is what
            // `local_path` is checking for. Copying it once here is what makes
            // the rest work, and on every other platform the path is real and
            // nothing is copied.
            //
            // `source_file` is held until this function returns, because a
            // temporary one deletes itself when the last reference to it goes.
            let (source_file, source) = if let Some(path) = local_path(&file) {
                (File::from(file), AttachmentSource::from(path))
            } else {
                let data = match file.load_contents_future().await {
                    Ok((data, _)) => data.to_vec(),
                    Err(error) => {
                        warn!("Could not read file: {error}");
                        toast!(obj, gettext("Error reading file"));
                        return SendOutcome::Failed;
                    }
                };

                // The upload is given the bytes rather than the copy's
                // path, because `matrix-sdk` names an attachment after the
                // file it read, and the copy is named `.tmp` followed by
                // six random characters. The cost is holding the attachment
                // twice while the dialog is open, and the alternative is
                // sending every picked file under a name nobody chose.
                let source = AttachmentSource::Data {
                    bytes: data.clone(),
                    filename: file_info.filename.clone(),
                };

                match save_data_to_tmp_file(data).await {
                    Ok(file) => (file, source),
                    Err(error) => {
                        warn!("Could not copy the file to a temporary file: {error}");
                        toast!(obj, gettext("Error reading file"));
                        return SendOutcome::Failed;
                    }
                }
            };
            let file = source_file.as_gfile();

            let mut send_all = false;
            if ask {
                let dialog = AttachmentDialog::new(&file_info.filename);
                dialog.set_file(file.clone(), file_info.mime.type_().as_str().into());
                dialog.set_remaining(remaining);

                match dialog.queue_response_future(&*obj).await {
                    AttachmentResponse::Cancel => return SendOutcome::Cancelled,
                    AttachmentResponse::SendAll => send_all = true,
                    AttachmentResponse::Send => {}
                }
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

            self.send_attachment(source, file_info.mime, info, thumbnail)
                .await;

            if send_all {
                SendOutcome::SentAll
            } else {
                SendOutcome::Sent
            }
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
            if let Some(related_to) = self.current_composer_state().related_to()
                && self
                    .obj()
                    .activate_action(
                        "room-history.scroll-to-event",
                        Some(&TimelineEventItemId::EventId(related_to.event_id()).to_variant()),
                    )
                    .is_err()
            {
                error!("Could not activate `room-history.scroll-to-event` action");
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

        /// Join or view the successor of the room, if possible.
        #[template_callback]
        async fn join_or_view_successor(&self) {
            let Some(room) = self.timeline.upgrade().map(|timeline| timeline.room()) else {
                return;
            };
            let Some(session) = room.session() else {
                return;
            };

            if !room.is_tombstoned() {
                return;
            }
            let obj = self.obj();

            if let Some(successor) = room
                .successor()
                .or_else(|| self.successor_room_list_info.local_room())
            {
                let Some(window) = obj.root().and_downcast::<Window>() else {
                    return;
                };

                window.session_view().select_room(successor);
            } else if let Some(successor_id) = room.successor_id() {
                // Route the successor room ID via the server of the sender of the tombstone
                // event, which is likely to know the room.
                let matrix_room = room.matrix_room().clone();
                let tombstone_event = spawn_tokio!(async move {
                    matrix_room
                        .get_state_event_static::<RoomTombstoneEventContent>()
                        .await
                })
                .await
                .expect("task was not aborted")
                .ok()
                .flatten();

                let via = tombstone_event
                    .and_then(|raw_event| raw_event.deserialize().ok())
                    .map(|event| event.sender().server_name().to_owned())
                    .into_iter()
                    .collect();

                if let Err(error) = session
                    .room_list()
                    .join_by_id_or_alias(successor_id.into(), via)
                    .await
                {
                    toast!(obj, error);
                }
            }
        }

        /// Disconnect the signal handlers of this toolbar.
        fn disconnect_signals(&self) {
            if let Some(timeline) = self.timeline.upgrade() {
                let room = timeline.room();

                for handler in self.room_handlers.take() {
                    room.disconnect(handler);
                }

                if let Some(handler) = self.send_message_permission_handler.take() {
                    room.permissions().disconnect(handler);
                }

                if let Some(handler) = self.send_sticker_permission_handler.take() {
                    room.permissions().disconnect(handler);
                }
            }
        }
    }
}

glib::wrapper! {
    /// A toolbar with different actions to send messages.
    pub struct MessageToolbar(ObjectSubclass<imp::MessageToolbar>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
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
    pub(crate) fn set_reply_to(&self, event: Event) {
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

    /// Send the given files, one message each, in order.
    pub(crate) async fn send_files(&self, files: Vec<gio::File>) {
        self.imp().send_files(files).await;
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

/// The maximum size of a GIF that we are willing to download to send, 16 MB.
///
/// [`klipy`] already picks a variant well under this; this is the backstop for
/// a service that does not describe its own files accurately.
const MAX_GIF_SIZE: u64 = 16 * 1024 * 1024;

/// An error encountered while putting a GIF on the homeserver.
#[derive(Debug, thiserror::Error)]
enum SendGifError {
    /// The GIF could not be downloaded from the service.
    #[error(transparent)]
    Download(#[from] crate::utils::http::HttpError),
    /// The GIF could not be uploaded to the homeserver.
    #[error(transparent)]
    Upload(#[from] matrix_sdk::Error),
}

/// Upload the given GIF to the homeserver and return the source to refer to it.
///
/// The GIF is encrypted first if it is going to an encrypted room, so a GIF is
/// no less private than any other image sent there.
async fn upload_gif(
    client: &Client,
    is_encrypted: bool,
    data: Vec<u8>,
) -> Result<StickerMediaSource, SendGifError> {
    if is_encrypted {
        let mut cursor = std::io::Cursor::new(data);
        let file = client.upload_encrypted_file(&mut cursor).await?;

        Ok(StickerMediaSource::Encrypted(Box::new(file)))
    } else {
        let response = client.media().upload(&mime::IMAGE_GIF, data, None).await?;

        Ok(StickerMediaSource::Plain(response.content_uri))
    }
}
