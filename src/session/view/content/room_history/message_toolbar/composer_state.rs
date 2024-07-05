use gtk::{
    glib,
    glib::{clone, closure_local},
    prelude::*,
    subclass::prelude::*,
};
use matrix_sdk::{ComposerDraft, ComposerDraftType};
use matrix_sdk_ui::timeline::{EditInfo, RepliedToInfo, TimelineEventItemId};
use ruma::{RoomOrAliasId, UserId};
use sourceview::prelude::*;
use tracing::{error, warn};

use super::{MessageBufferChunk, MessageBufferParser};
use crate::{
    components::{AtRoom, PillSource},
    prelude::*,
    session::model::{EventKey, Member, Room},
    spawn, spawn_tokio,
    utils::matrix::AT_ROOM,
};

// The duration in seconds we wait for before saving a change.
const SAVING_TIMEOUT: u32 = 3;
/// The start tag to represent a mention in a serialized draft.
const MENTION_START_TAG: &str = "<org.gnome.fractal.mention>";
/// The end tag to represent a mention in a serialized draft.
const MENTION_END_TAG: &str = "</org.gnome.fractal.mention>";

mod imp {
    use std::{cell::RefCell, marker::PhantomData};

    use futures_util::lock::Mutex;
    use glib::subclass::Signal;
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::ComposerState)]
    pub struct ComposerState {
        /// The room associated with this state.
        #[property(get, construct_only, nullable)]
        pub room: glib::WeakRef<Room>,
        /// The buffer of this state.
        #[property(get)]
        pub buffer: sourceview::Buffer,
        /// The relation of this state.
        pub related_to: RefCell<Option<RelationInfo>>,
        /// Whether this state has a relation.
        #[property(get = Self::has_relation)]
        pub has_relation: PhantomData<bool>,
        /// The widgets of this state.
        ///
        /// These are the widgets inserted in the composer.
        pub widgets: RefCell<Vec<(gtk::Widget, gtk::TextChildAnchor)>>,
        /// The current view attached to this state.
        pub view: glib::WeakRef<sourceview::View>,
        /// The draft that was saved in the store.
        pub saved_draft: RefCell<Option<ComposerDraft>>,
        /// The signal handler for the current draft saving timeout.
        draft_timeout: RefCell<Option<glib::SourceId>>,
        /// The lock to prevent multiple draft saving operations at the same
        /// time.
        draft_lock: Mutex<()>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ComposerState {
        const NAME: &'static str = "ContentComposerState";
        type Type = super::ComposerState;
    }

    #[glib::derived_properties]
    impl ObjectImpl for ComposerState {
        fn signals() -> &'static [Signal] {
            static SIGNALS: Lazy<Vec<Signal>> =
                Lazy::new(|| vec![Signal::builder("related-to-changed").build()]);
            SIGNALS.as_ref()
        }

        fn constructed(&self) {
            self.parent_constructed();

            crate::utils::sourceview::setup_style_scheme(&self.buffer);

            // Markdown highlighting.
            let md_lang = sourceview::LanguageManager::default().language("markdown");
            self.buffer.set_language(md_lang.as_ref());

            self.buffer.connect_changed(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.update_widgets();
                    imp.trigger_draft_saving();
                }
            ));
        }
    }

    impl ComposerState {
        /// Whether this state has a relation.
        fn has_relation(&self) -> bool {
            self.related_to.borrow().is_some()
        }

        /// Update the list of widgets present in the composer.
        pub(super) fn update_widgets(&self) {
            self.widgets
                .borrow_mut()
                .retain(|(_w, anchor)| !anchor.is_deleted());
        }

        /// Get the draft for the current state.
        ///
        /// Returns `None` if the draft would be empty.
        fn draft(&self) -> Option<ComposerDraft> {
            let obj = self.obj();
            let draft_type = self
                .related_to
                .borrow()
                .as_ref()
                .map(|r| r.as_draft_type())
                .unwrap_or(ComposerDraftType::NewMessage);

            let (start, end) = self.buffer.bounds();
            let body_len = end.offset() as usize;
            let mut plain_text = String::with_capacity(body_len);

            let split_message = MessageBufferParser::new(&obj, start, end);
            for chunk in split_message {
                match chunk {
                    MessageBufferChunk::Text(text) => {
                        plain_text.push_str(&text);
                    }
                    MessageBufferChunk::Mention(source) => {
                        plain_text.push_str(MENTION_START_TAG);

                        if let Some(user) = source.downcast_ref::<Member>() {
                            plain_text.push_str(user.user_id().as_ref());
                        } else if let Some(room) = source.downcast_ref::<Room>() {
                            plain_text.push_str(
                                room.aliases()
                                    .alias()
                                    .as_ref()
                                    .map(AsRef::as_ref)
                                    .unwrap_or_else(|| room.room_id().as_ref()),
                            );
                        } else if source.is::<AtRoom>() {
                            plain_text.push_str(AT_ROOM);
                        } else {
                            unreachable!()
                        };

                        plain_text.push_str(MENTION_END_TAG);
                    }
                }
            }

            if draft_type == ComposerDraftType::NewMessage && plain_text.trim().is_empty() {
                None
            } else {
                Some(ComposerDraft {
                    plain_text,
                    html_text: None,
                    draft_type,
                })
            }
        }

        /// Trigger the timeout for saving the current draft.
        pub(super) fn trigger_draft_saving(&self) {
            if self.draft_timeout.borrow().is_some() {
                return;
            }

            let draft = self.draft();
            if *self.saved_draft.borrow() == draft {
                return;
            }

            let timeout = glib::timeout_add_seconds_local_once(
                SAVING_TIMEOUT,
                clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move || {
                        imp.draft_timeout.take();
                        let obj = imp.obj().clone();

                        spawn!(glib::Priority::DEFAULT_IDLE, async move {
                            obj.imp().save_draft().await;
                        });
                    }
                ),
            );
            self.draft_timeout.replace(Some(timeout));
        }

        /// Save the current draft.
        async fn save_draft(&self) {
            let Some(room) = self.room.upgrade() else {
                return;
            };
            let Some(_lock) = self.draft_lock.try_lock() else {
                // The previous saving operation is still ongoing, try saving again later.
                self.trigger_draft_saving();
                return;
            };

            let draft = self.draft();
            if *self.saved_draft.borrow() == draft {
                // Nothing to do.
                return;
            }

            let matrix_room = room.matrix_room().clone();
            let draft_clone = draft.clone();
            let handle = spawn_tokio!(async move {
                if let Some(draft) = draft_clone {
                    matrix_room.save_composer_draft(draft).await
                } else {
                    matrix_room.clear_composer_draft().await
                }
            });

            match handle.await.unwrap() {
                Ok(()) => {
                    self.saved_draft.replace(draft);
                }
                Err(error) => {
                    error!("Could not save composer draft: {error}");
                }
            }
        }

        /// Add the given widget and anchor to this state.
        pub(super) fn add_widget(
            &self,
            widget: impl IsA<gtk::Widget>,
            anchor: gtk::TextChildAnchor,
        ) {
            let widget = widget.upcast();

            if let Some(view) = self.view.upgrade() {
                view.add_child_at_anchor(&widget, &anchor);
            }

            self.widgets.borrow_mut().push((widget, anchor));
        }

        /// Restore the state from the persisted draft.
        pub(super) async fn restore_draft(&self) {
            let Some(room) = self.room.upgrade() else {
                return;
            };

            let matrix_room = room.matrix_room().clone();
            let handle = spawn_tokio!(async move { matrix_room.load_composer_draft().await });

            match handle.await.unwrap() {
                Ok(Some(draft)) => self.restore_from_draft(draft).await,
                Ok(None) => {}
                Err(error) => {
                    error!("Could not restore draft: {error}");
                }
            }
        }

        /// Restore the state from the given draft.
        async fn restore_from_draft(&self, draft: ComposerDraft) {
            let Some(room) = self.room.upgrade() else {
                return;
            };

            // Restore the relation.
            self.restore_related_to_from_draft(draft.draft_type.clone())
                .await;

            // Make sure we start from an empty state.
            self.buffer.set_text("");
            self.widgets.borrow_mut().clear();

            // Fill the buffer while inserting mentions.
            let text = &draft.plain_text;
            let mut end_iter = self.buffer.end_iter();
            let mut pos = 0;

            while let Some(rel_start) = text[pos..].find(MENTION_START_TAG) {
                let start = pos + rel_start;
                let content_start = start + MENTION_START_TAG.len();

                let Some(rel_content_end) = text[content_start..].find(MENTION_END_TAG) else {
                    // Abort parsing.
                    error!("Could not find end tag for mention in serialized draft");
                    break;
                };
                let content_end = content_start + rel_content_end;

                if start != pos {
                    self.buffer.insert(&mut end_iter, &text[pos..start]);
                }

                match DraftMention::new(&room, &text[content_start..content_end]) {
                    DraftMention::Source(source) => {
                        let anchor = match end_iter.child_anchor() {
                            Some(anchor) => anchor,
                            None => self.buffer.create_child_anchor(&mut end_iter),
                        };
                        self.add_widget(source.to_pill(), anchor);
                    }
                    DraftMention::Text(s) => {
                        self.buffer.insert(&mut end_iter, s);
                    }
                }

                pos = content_end + MENTION_END_TAG.len();
            }

            if pos != text.len() {
                self.buffer.insert(&mut end_iter, &text[pos..]);
            }

            self.saved_draft.replace(Some(draft));
        }

        /// Restore the relation from the given draft content.
        async fn restore_related_to_from_draft(&self, draft_type: ComposerDraftType) {
            let Some(room) = self.room.upgrade() else {
                return;
            };

            let related_to = match draft_type {
                ComposerDraftType::NewMessage => None,
                ComposerDraftType::Reply { event_id } => {
                    let matrix_timeline = room.timeline().matrix_timeline();

                    let handle = spawn_tokio!(async move {
                        matrix_timeline
                            .replied_to_info_from_event_id(&event_id)
                            .await
                    });

                    match handle.await.unwrap() {
                        Ok(info) => Some(RelationInfo::Reply(info)),
                        Err(error) => {
                            warn!("Could not fetch replied-to event content of draft: {error}");
                            None
                        }
                    }
                }
                ComposerDraftType::Edit { event_id } => {
                    let matrix_timeline = room.timeline().matrix_timeline();

                    let handle = spawn_tokio!(async move {
                        matrix_timeline.edit_info_from_event_id(&event_id).await
                    });

                    match handle.await.unwrap() {
                        Ok(info) => Some(RelationInfo::Edit(info)),
                        Err(error) => {
                            warn!("Could not fetch replied-to event content of draft: {error}");
                            None
                        }
                    }
                }
            };

            self.related_to.replace(related_to);

            let obj = self.obj();
            obj.emit_by_name::<()>("related-to-changed", &[]);
            obj.notify_has_relation();
        }
    }
}

glib::wrapper! {
    /// The composer state for a room.
    ///
    /// This allows to save and restore the composer state between room changes.
    /// It keeps track of the related event and restores the state of the composer's `GtkSourceView`.
    pub struct ComposerState(ObjectSubclass<imp::ComposerState>);
}

impl ComposerState {
    /// Create a new empty `ComposerState` for the given room.
    pub fn new(room: Option<&Room>) -> Self {
        let obj = glib::Object::builder::<Self>()
            .property("room", room)
            .build();

        let imp = obj.imp();
        spawn!(clone!(
            #[weak]
            imp,
            async move {
                imp.restore_draft().await;
            }
        ));

        obj
    }

    /// Attach this state to the given view.
    pub fn attach_to_view(&self, view: Option<&sourceview::View>) {
        let imp = self.imp();

        imp.view.set(view);

        if let Some(view) = view {
            view.set_buffer(Some(&imp.buffer));

            imp.update_widgets();

            for (widget, anchor) in &*imp.widgets.borrow() {
                view.add_child_at_anchor(widget, anchor);
            }
        }
    }

    /// Clear this state.
    pub fn clear(&self) {
        self.set_related_to(None);

        let imp = self.imp();
        imp.buffer.set_text("");
        imp.widgets.borrow_mut().clear();
    }

    /// The relation to send with the current message.
    pub fn related_to(&self) -> Option<RelationInfo> {
        self.imp().related_to.borrow().clone()
    }

    /// Set the relation to send with the current message.
    pub fn set_related_to(&self, related_to: Option<RelationInfo>) {
        let imp = self.imp();

        let had_relation = self.has_relation();

        if imp
            .related_to
            .borrow()
            .as_ref()
            .is_some_and(|r| matches!(r, RelationInfo::Edit(_)))
        {
            // The user aborted the edit or the edit is done, clean up the entry.
            imp.buffer.set_text("");
        }

        imp.related_to.replace(related_to);

        if self.has_relation() != had_relation {
            self.notify_has_relation();
        }

        self.emit_by_name::<()>("related-to-changed", &[]);
        self.imp().trigger_draft_saving();
    }

    /// Add the given widget and anchor to this state.
    pub fn add_widget(&self, widget: impl IsA<gtk::Widget>, anchor: gtk::TextChildAnchor) {
        self.imp().add_widget(widget, anchor);
    }

    /// Get the widget at the given anchor, if any.
    pub fn widget_at_anchor(&self, anchor: &gtk::TextChildAnchor) -> Option<gtk::Widget> {
        self.imp()
            .widgets
            .borrow()
            .iter()
            .find(|(_, a)| a == anchor)
            .map(|(w, _)| w.clone())
    }

    /// Connect to the signal emitted when the relation changed.
    pub fn connect_related_to_changed<F: Fn(&Self) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "related-to-changed",
            true,
            closure_local!(move |obj: Self| {
                f(&obj);
            }),
        )
    }
}

/// The possible relations to send with a message.
#[derive(Debug, Clone)]
pub enum RelationInfo {
    /// Send a reply with the given replied to info.
    Reply(RepliedToInfo),

    /// Send an edit with the given edit info.
    Edit(EditInfo),
}

impl RelationInfo {
    /// The unique key of the related event.
    pub fn key(&self) -> EventKey {
        match self {
            RelationInfo::Reply(info) => EventKey::EventId(info.event_id().to_owned()),
            RelationInfo::Edit(info) => match info.id() {
                TimelineEventItemId::TransactionId(txn_id) => {
                    EventKey::TransactionId(txn_id.clone())
                }
                TimelineEventItemId::EventId(event_id) => EventKey::EventId(event_id.clone()),
            },
        }
    }

    /// Get this `RelationInfo` as a draft type.
    pub fn as_draft_type(&self) -> ComposerDraftType {
        match self {
            Self::Reply(info) => ComposerDraftType::Reply {
                event_id: info.event_id().to_owned(),
            },
            Self::Edit(info) => match info.id() {
                // We don't support editing local echos (yet).
                TimelineEventItemId::TransactionId(_) => ComposerDraftType::NewMessage,
                TimelineEventItemId::EventId(event_id) => ComposerDraftType::Reply {
                    event_id: event_id.clone(),
                },
            },
        }
    }
}

/// A mention that was serialized in a draft.
///
/// If we managed to restore the mention, this is a `PillSource`, otherwise it's
/// the text of the mention.
enum DraftMention<'a> {
    /// The source of the mention.
    Source(PillSource),
    /// The text of the mention.
    Text(&'a str),
}

impl<'a> DraftMention<'a> {
    /// Construct a `MentionContent` from the given string in the given room.
    fn new(room: &Room, s: &'a str) -> Self {
        if s == AT_ROOM {
            Self::Source(room.at_room().upcast())
        } else if s.starts_with('@') {
            // This is a user mention.
            match UserId::parse(s) {
                Ok(user_id) => {
                    let member = Member::new(room, user_id);
                    member.update();
                    Self::Source(member.upcast())
                }
                Err(error) => {
                    error!("Could not parse user ID `{s}` from serialized mention: {error}");
                    Self::Text(s)
                }
            }
        } else {
            // It should be a room mention.
            let Some(session) = room.session() else {
                return Self::Text(s);
            };
            let room_list = session.room_list();

            match RoomOrAliasId::parse(s) {
                Ok(identifier) => match room_list.get_by_identifier(&identifier) {
                    Some(room) => Self::Source(room.upcast()),
                    None => {
                        warn!("Could not find room `{s}` from serialized mention");
                        Self::Text(s)
                    }
                },
                Err(error) => {
                    error!(
                        "Could not parse room identifier `{s}` from serialized mention: {error}"
                    );
                    Self::Text(s)
                }
            }
        }
    }
}
