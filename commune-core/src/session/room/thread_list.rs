//! The list of threads of a room, headless.
//!
//! The value half of the application's `ThreadList`
//! (`src/session/room/thread_list.rs`): the SDK's thread list service,
//! built on first use, its pages loaded one at a time, and its items —
//! the SDK's own, passed through with the diffs that follow them, as the
//! timeline's are. What stayed in the application is the `gio::ListStore`
//! of rows and the sentences a row shows for a message it cannot draw;
//! which sentence is [`ContentPreview`]'s to say.

use std::sync::Arc;

use eyeball::{SharedObservable, Subscriber};
use eyeball_im::{Vector, VectorDiff};
use futures_util::Stream;
use matrix_sdk_ui::timeline::{
    MsgLikeKind, TimelineItemContent,
    thread_list_service::{ThreadListItem, ThreadListPaginationState, ThreadListService},
};
use tracing::error;

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
    /// in place when a new thread event arrives from sync; both reach the
    /// subscriber here. The items and diffs are the SDK's own.
    pub async fn subscribe_items(
        &self,
    ) -> (
        Vector<ThreadListItem>,
        impl Stream<Item = Vec<VectorDiff<ThreadListItem>>> + use<>,
    ) {
        let service = self.service().await;

        spawn_tokio!(async move { service.subscribe_to_items_updates() })
            .await
            .expect("task was not aborted")
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
