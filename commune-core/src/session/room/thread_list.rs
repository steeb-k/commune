//! The list of threads of a room, headless.
//!
//! The value half of the application's `ThreadList`
//! (`src/session/room/thread_list.rs`): the SDK's thread list service,
//! built on first use, its pages loaded one at a time, and its items —
//! the SDK's own, passed through with the diffs that follow them, as the
//! timeline's are. What stayed in the application is the `gio::ListStore`
//! of rows and the sentences a row shows for a message it cannot draw;
//! which sentence is [`ContentPreview`]'s to say.

use std::sync::{Arc, Mutex};

use eyeball::{SharedObservable, Subscriber};
use eyeball_im::{ObservableVector, Vector, VectorDiff};
use futures_util::{Stream, StreamExt};
use matrix_sdk_ui::timeline::{
    MsgLikeKind, Profile, TimelineDetails, TimelineItemContent,
    thread_list_service::{
        ThreadListItem, ThreadListItemEvent, ThreadListPaginationState, ThreadListService,
    },
};
use ruma::OwnedEventId;
use tracing::{debug, error, warn};

use crate::{UserFacingError, spawn_tokio, utils::LoadingState};

/// What a thread row shows of an event, in one line.
///
/// The row has no room for the real widgets, so a message keeps its body
/// and everything else says what it is; the saying is the embedder's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentPreview {
    /// A message or a sticker, with its body.
    Body(String),
    /// A message that was removed.
    Redacted,
    /// A message that could not be decrypted.
    UnableToDecrypt,
    /// Anything else.
    Unsupported,
}

impl ContentPreview {
    /// The preview of the given content.
    #[must_use]
    pub fn of(content: Option<&TimelineItemContent>) -> Self {
        match content {
            Some(TimelineItemContent::MsgLike(msg_like)) => match &msg_like.kind {
                MsgLikeKind::Message(message) => Self::Body(message.msgtype().body().to_owned()),
                MsgLikeKind::Sticker(sticker) => Self::Body(sticker.content().body.clone()),
                MsgLikeKind::Redacted => Self::Redacted,
                MsgLikeKind::UnableToDecrypt(_) => Self::UnableToDecrypt,
                _ => Self::Unsupported,
            },
            _ => Self::Unsupported,
        }
    }
}

/// What can go wrong while listing the threads of a room.
#[derive(Debug, thiserror::Error)]
pub enum ThreadListError {
    /// The homeserver could not list the threads.
    ///
    /// Boxed because the SDK's error is large enough that carrying it by
    /// value makes every `Result` in this module expensive.
    #[error(transparent)]
    Server(#[from] Box<matrix_sdk_ui::timeline::thread_list_service::ThreadListServiceError>),
}

impl UserFacingError for ThreadListError {
    fn to_user_facing(&self) -> String {
        match self {
            // The embedder has its own rendering of an SDK error — the GTK
            // application's is translated — so this is only the fallback.
            Self::Server(error) => error.to_string(),
        }
    }
}

/// The list of threads of a room.
///
/// It is loaded from the `/threads` endpoint page by page, most recent
/// activity first, and the SDK keeps the reply count and latest event of
/// each listed thread current as new thread events arrive from sync.
/// Cheap to clone; every clone shares the same state.
#[derive(Debug, Clone)]
pub struct ThreadList {
    inner: Arc<ThreadListInner>,
}

struct ThreadListInner {
    /// The room API of the SDK.
    matrix_room: matrix_sdk::room::Room,
    /// The underlying SDK service, built on first use.
    service: tokio::sync::OnceCell<Arc<ThreadListService>>,
    /// The list as presented: the SDK's, with the threads that began while
    /// it was open put in front. Built on first subscription.
    mirror: tokio::sync::OnceCell<Arc<Mirror>>,
    /// The loading state of the list.
    loading_state: SharedObservable<LoadingState>,
    /// Whether the whole thread list was loaded.
    has_reached_end: SharedObservable<bool>,
}

// The SDK service does not implement `Debug`, so this cannot be derived.
impl std::fmt::Debug for ThreadListInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThreadListInner")
            .field("loading_state", &self.loading_state)
            .field("has_reached_end", &self.has_reached_end)
            .finish_non_exhaustive()
    }
}

impl ThreadList {
    /// Create the thread list of the given room.
    pub(crate) fn new(matrix_room: matrix_sdk::room::Room) -> Self {
        Self {
            inner: Arc::new(ThreadListInner {
                matrix_room,
                service: tokio::sync::OnceCell::new(),
                mirror: tokio::sync::OnceCell::new(),
                loading_state: SharedObservable::new(LoadingState::Initial),
                has_reached_end: SharedObservable::new(false),
            }),
        }
    }

    /// The loading state of the list.
    #[must_use]
    pub fn loading_state(&self) -> LoadingState {
        self.inner.loading_state.get()
    }

    /// Subscribe to the loading state of the list.
    pub fn subscribe_loading_state(&self) -> Subscriber<LoadingState> {
        self.inner.loading_state.subscribe()
    }

    /// Whether the whole thread list was loaded.
    #[must_use]
    pub fn has_reached_end(&self) -> bool {
        self.inner.has_reached_end.get()
    }

    /// Subscribe to whether the whole thread list was loaded.
    pub fn subscribe_has_reached_end(&self) -> Subscriber<bool> {
        self.inner.has_reached_end.subscribe()
    }

    /// Whether more threads can be loaded.
    #[must_use]
    pub fn can_load_more(&self) -> bool {
        self.loading_state() != LoadingState::Loading && !self.has_reached_end()
    }

    /// The SDK service, built on first use.
    ///
    /// The service spawns its live-update task at construction, which needs
    /// the runtime.
    async fn service(&self) -> Arc<ThreadListService> {
        let inner = &self.inner;

        inner
            .service
            .get_or_init(|| async {
                let matrix_room = inner.matrix_room.clone();
                spawn_tokio!(async move { Arc::new(ThreadListService::new(matrix_room)) })
                    .await
                    .expect("task was not aborted")
            })
            .await
            .clone()
    }

    /// The current items, and the stream of the changes that follow them.
    ///
    /// The service appends pages as they are fetched and rewrites an item
    /// in place when a new thread event arrives from sync. What it does
    /// not do is notice a thread that begins while the list is open: its
    /// listener only touches roots it already lists, so the first reply
    /// to a message the list never fetched went nowhere, and a new thread
    /// appeared only on the next load. The list presented here is the
    /// SDK's with that gap closed — a thread that begins is put in front,
    /// most recent activity first, and its replies keep it current.
    pub async fn subscribe_items(
        &self,
    ) -> (
        Vector<ThreadListItem>,
        impl Stream<Item = Vec<VectorDiff<ThreadListItem>>> + use<>,
    ) {
        let mirror = self.mirror().await;

        let items = mirror.items.lock().expect("mutex is not poisoned");
        let subscriber = items.subscribe();
        (items.clone(), subscriber.into_batched_stream())
    }

    /// The presented list, built on first use from the SDK's, with its two
    /// tasks: one carries the SDK's own changes over, one watches sync for
    /// threads the SDK does not list yet.
    async fn mirror(&self) -> Arc<Mirror> {
        let inner = &self.inner;

        inner
            .mirror
            .get_or_init(|| async {
                let service = self.service().await;
                let matrix_room = inner.matrix_room.clone();

                spawn_tokio!(async move { Mirror::new(&service, matrix_room) })
                    .await
                    .expect("task was not aborted")
            })
            .await
            .clone()
    }

    /// Load the next page of threads.
    ///
    /// Nothing is loaded when a page is already loading or the end was
    /// reached; a failure puts the list in the error state, which the
    /// state observable reports.
    pub async fn load_more(&self) -> Result<(), ThreadListError> {
        if !self.can_load_more() {
            return Ok(());
        }

        self.inner
            .loading_state
            .set_if_not_eq(LoadingState::Loading);

        let service = self.service().await;

        let service_clone = service.clone();
        let handle = spawn_tokio!(async move { service_clone.paginate().await });

        match handle.await.expect("task was not aborted") {
            Ok(()) => {
                let end_reached = matches!(
                    service.pagination_state(),
                    ThreadListPaginationState::Idle { end_reached: true }
                );
                self.inner.has_reached_end.set_if_not_eq(end_reached);
                self.inner.loading_state.set_if_not_eq(LoadingState::Ready);
                Ok(())
            }
            Err(paginate_error) => {
                error!("Could not load the threads of the room: {paginate_error}");
                self.inner.loading_state.set_if_not_eq(LoadingState::Error);
                Err(ThreadListError::Server(Box::new(paginate_error)))
            }
        }
    }
}

/// The list as presented: the SDK's items after the ones inserted here.
///
/// The inserted threads sit in front, so an SDK index maps to a presented
/// index by adding how many were inserted; the SDK never touches the
/// inserted ones, whose replies are followed here instead.
struct Mirror {
    /// The presented items.
    items: Mutex<ObservableVector<ThreadListItem>>,
    /// The roots inserted here, front-most first.
    inserted: Mutex<Vec<OwnedEventId>>,
    /// The tasks feeding the list; aborted with the mirror.
    tasks: Mutex<Vec<tokio::task::AbortHandle>>,
}

impl Drop for Mirror {
    fn drop(&mut self) {
        for task in self.tasks.lock().expect("mutex is not poisoned").drain(..) {
            task.abort();
        }
    }
}

impl Mirror {
    /// Build the mirror over the given service and start its tasks.
    ///
    /// Must be called from the tokio runtime.
    fn new(service: &Arc<ThreadListService>, matrix_room: matrix_sdk::room::Room) -> Arc<Self> {
        let (snapshot, mut sdk_diffs) = service.subscribe_to_items_updates();

        let mut items = ObservableVector::new();
        items.append(snapshot);

        let mirror = Arc::new(Self {
            items: Mutex::new(items),
            inserted: Mutex::new(Vec::new()),
            tasks: Mutex::new(Vec::new()),
        });

        // The SDK's own changes, carried over with the inserted count as
        // the offset.
        let forward = {
            let mirror = Arc::downgrade(&mirror);
            tokio::spawn(async move {
                while let Some(diffs) = sdk_diffs.next().await {
                    let Some(mirror) = mirror.upgrade() else {
                        break;
                    };
                    mirror.apply_sdk_diffs(diffs);
                }
            })
        };

        // The threads that begin while the list is open.
        let watch = {
            let mirror = Arc::downgrade(&mirror);
            let service = service.clone();
            tokio::spawn(async move {
                Self::watch_new_threads(mirror, service, matrix_room).await;
            })
        };

        mirror
            .tasks
            .lock()
            .expect("mutex is not poisoned")
            .extend([forward.abort_handle(), watch.abort_handle()]);

        mirror
    }

    /// Apply the SDK's changes to the presented list, shifted past the
    /// inserted threads.
    fn apply_sdk_diffs(&self, diffs: Vec<VectorDiff<ThreadListItem>>) {
        let offset = self.inserted.lock().expect("mutex is not poisoned").len();
        let mut items = self.items.lock().expect("mutex is not poisoned");

        for diff in diffs {
            match diff {
                VectorDiff::Append { values } => items.append(values),
                VectorDiff::Clear => {
                    items.truncate(offset);
                }
                VectorDiff::PushFront { value } => items.insert(offset, value),
                VectorDiff::PushBack { value } => items.push_back(value),
                VectorDiff::PopFront => {
                    if items.len() > offset {
                        items.remove(offset);
                    }
                }
                VectorDiff::PopBack => {
                    if items.len() > offset {
                        items.pop_back();
                    }
                }
                VectorDiff::Insert { index, value } => {
                    let index = (index + offset).min(items.len());
                    items.insert(index, value);
                }
                VectorDiff::Set { index, value } => {
                    if index + offset < items.len() {
                        items.set(index + offset, value);
                    }
                }
                VectorDiff::Remove { index } => {
                    if index + offset < items.len() {
                        items.remove(index + offset);
                    }
                }
                VectorDiff::Truncate { length } => items.truncate(length + offset),
                VectorDiff::Reset { values } => {
                    items.truncate(offset);
                    items.append(values);
                }
            }
        }
    }

    /// Follow the room's event cache for thread replies, and put the
    /// thread in front when its root is listed nowhere yet.
    async fn watch_new_threads(
        mirror: std::sync::Weak<Self>,
        service: Arc<ThreadListService>,
        matrix_room: matrix_sdk::room::Room,
    ) {
        use matrix_sdk::event_cache::RoomEventCacheUpdate;
        use tokio::sync::broadcast::error::RecvError;

        let (_drop_handles, mut subscriber) = match async {
            let (room_event_cache, drop_handles) = matrix_room.event_cache().await?;
            let (_, subscriber) = room_event_cache.subscribe().await?;
            matrix_sdk::event_cache::Result::Ok((drop_handles, subscriber))
        }
        .await
        {
            Ok(pair) => pair,
            Err(error) => {
                warn!("Could not follow the room for new threads: {error}");
                return;
            }
        };

        loop {
            let update = match subscriber.recv().await {
                Ok(update) => update,
                Err(RecvError::Closed) => break,
                Err(RecvError::Lagged(_)) => continue,
            };
            let RoomEventCacheUpdate::UpdateTimelineEvents(timeline_diffs) = update else {
                continue;
            };

            let mut events = Vec::new();
            for diff in timeline_diffs.diffs {
                match diff {
                    VectorDiff::Append { values } | VectorDiff::Reset { values } => {
                        events.extend(values);
                    }
                    VectorDiff::PushFront { value }
                    | VectorDiff::PushBack { value }
                    | VectorDiff::Insert { value, .. }
                    | VectorDiff::Set { value, .. } => events.push(value),
                    _ => {}
                }
            }

            for event in events {
                let Some(root_id) = thread_root_of(&event) else {
                    continue;
                };
                let Some(mirror) = mirror.upgrade() else {
                    return;
                };

                let inserted_at = mirror
                    .inserted
                    .lock()
                    .expect("mutex is not poisoned")
                    .iter()
                    .position(|root| *root == root_id);

                if let Some(index) = inserted_at {
                    // A thread that began here: the SDK does not follow
                    // it, so its replies are counted here.
                    if let Some(latest_event) = build_event(&matrix_room, event).await {
                        let mut items = mirror.items.lock().expect("mutex is not poisoned");
                        if let Some(current) = items.get(index).cloned() {
                            let mut updated = current;
                            updated.latest_event = Some(latest_event);
                            updated.num_replies = updated.num_replies.saturating_add(1);
                            items.set(index, updated);
                        }
                    }
                    continue;
                }

                if service
                    .items()
                    .iter()
                    .any(|item| item.root_event.event_id == root_id)
                {
                    // The SDK lists it and keeps it current.
                    continue;
                }

                // A thread that just began. Its root is fetched the way a
                // page fetches roots, and the reply is its latest event.
                let root = match matrix_room.event(&root_id, None).await {
                    Ok(root) => root,
                    Err(error) => {
                        debug!("Could not fetch the root of a new thread: {error}");
                        continue;
                    }
                };
                let Some(root_event) = build_event(&matrix_room, root).await else {
                    continue;
                };
                let latest_event = build_event(&matrix_room, event).await;

                let item = ThreadListItem {
                    root_event,
                    latest_event,
                    num_replies: 1,
                };

                let mut inserted = mirror.inserted.lock().expect("mutex is not poisoned");
                if inserted.contains(&root_id) {
                    continue;
                }
                inserted.insert(0, root_id);
                mirror
                    .items
                    .lock()
                    .expect("mutex is not poisoned")
                    .push_front(item);
            }
        }
    }
}

/// The root of the thread the given event replies in, if it is a thread
/// reply.
fn thread_root_of(
    event: &matrix_sdk::deserialized_responses::TimelineEvent,
) -> Option<OwnedEventId> {
    let content = event
        .raw()
        .get_field::<serde_json::Value>("content")
        .ok()
        .flatten()?;
    let relation = content.get("m.relates_to")?;
    if relation.get("rel_type")?.as_str()? != "m.thread" {
        return None;
    }
    let root = relation.get("event_id")?.as_str()?;
    OwnedEventId::try_from(root).ok()
}

/// A thread list event for the given event, the way the SDK builds its
/// own: the sender's profile from the room, the content as the timeline
/// reads it.
async fn build_event(
    room: &matrix_sdk::room::Room,
    event: matrix_sdk::deserialized_responses::TimelineEvent,
) -> Option<ThreadListItemEvent> {
    let event_id = event.event_id()?.to_owned();
    let timestamp = event.timestamp()?;
    let sender = event.sender()?;
    let is_own = room.own_user_id() == sender;
    let sender_profile = TimelineDetails::from_initial_value(Profile::load(room, &sender).await);
    let content = TimelineItemContent::from_event(room, event).await;

    Some(ThreadListItemEvent {
        event_id,
        timestamp,
        sender,
        is_own,
        sender_profile,
        content,
    })
}
