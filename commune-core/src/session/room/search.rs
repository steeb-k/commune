//! The search of the messages of a room.
//!
//! The headless counterpart of the application's `RoomSearch` `GObject`
//! (`src/session/room/search.rs`): the same two backends, chosen the same
//! way, with the same paging. Messages of an encrypted room can only be
//! searched in the local search index, since the server cannot read their
//! content; other rooms are searched on the server, which knows the whole
//! history of the room.
//!
//! What stayed in the application is the abort handle and the
//! `gio::ListStore` — the task that runs a page is the bridge's to cancel.
//! What moved is the generation counter, because a response that arrives
//! after the term changed has to be dropped wherever the request runs.

use std::sync::{Arc, Mutex};

use eyeball::SharedObservable;
use ruma::{
    MilliSecondsSinceUnixEpoch, OwnedEventId, OwnedUserId, UInt,
    api::client::{
        filter::RoomEventFilter,
        search::search_events::{
            self,
            v3::{Categories, Criteria, EventContext, OrderBy, SearchKeys},
        },
    },
    assign,
    events::{
        AnySyncTimelineEvent, MessageLikeEventType, room::message::OriginalSyncRoomMessageEvent,
    },
    serde::Raw,
};
use tracing::{debug, warn};

use super::Room;
use crate::{
    UserFacingError, matrix::original_message_event_from_raw, spawn_tokio, utils::LoadingState,
};

/// The maximum number of results in a page, as the application pages them.
pub const RESULTS_PAGE_SIZE: usize = 20;
/// The maximum number of results requested from the local search index.
///
/// The index returns results ordered by relevance, so all of them must be
/// fetched before they can be presented ordered by recency.
const LOCAL_MAX_RESULTS: usize = 500;

/// Characters with a special meaning for the query parser of the local search
/// index.
///
/// If they are not removed, the parser returns an error for a query that a user
/// could reasonably type, like `who's there?`.
const QUERY_SPECIAL_CHARS: [char; 15] = [
    '"', '\'', '+', '-', '!', '(', ')', '{', '}', '[', ']', '^', '~', '*', ':',
];

/// Sanitize the given search term for the query parser of the local search
/// index.
///
/// The parser has a syntax of its own, and returns an error rather than no
/// results when a query does not respect it. Since we present it as a plain
/// text search field, every term is escaped and quoted.
fn sanitize_local_query(search_term: &str) -> String {
    search_term
        .split_whitespace()
        .map(|token| token.replace(QUERY_SPECIAL_CHARS, "").replace('\\', ""))
        .filter(|token| !token.is_empty())
        .map(|token| format!("\"{token}\""))
        .collect::<Vec<_>>()
        .join(" ")
}

/// What can go wrong while searching a room.
///
/// A failure of the local index is not here on purpose: the application
/// treats it as "nothing to show", because the query parser rejects a
/// syntax that cannot always be prevented and that is not an error the
/// user can act on.
#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    /// The homeserver could not search the room.
    ///
    /// Boxed because `matrix_sdk::HttpError` is large enough that carrying
    /// it by value makes every `Result` in this module expensive.
    #[error(transparent)]
    Server(#[from] Box<matrix_sdk::HttpError>),
    /// The local search index could not take the room's loaded messages.
    ///
    /// The error type of the index belongs to a crate that is not a direct
    /// dependency, so it cannot be named here — the application has the
    /// same limitation and carries the rendering too.
    #[error("{0}")]
    Index(String),
}

impl UserFacingError for SearchError {
    fn to_user_facing(&self) -> String {
        match self {
            // The embedder has its own rendering of an SDK error — the GTK
            // application's is translated — so this is only the fallback.
            Self::Server(error) => error.to_string(),
            Self::Index(error) => error.clone(),
        }
    }
}

/// A message matching a search in a room.
#[derive(Debug, Clone)]
pub struct SearchResult {
    event: OriginalSyncRoomMessageEvent,
}

impl SearchResult {
    /// The Matrix event of this result.
    #[must_use]
    pub fn event(&self) -> &OriginalSyncRoomMessageEvent {
        &self.event
    }

    /// The ID of the event of this result.
    #[must_use]
    pub fn event_id(&self) -> OwnedEventId {
        self.event.event_id.clone()
    }

    /// The ID of the sender of this message.
    #[must_use]
    pub fn sender_id(&self) -> OwnedUserId {
        self.event.sender.clone()
    }

    /// The timestamp of this message.
    #[must_use]
    pub fn timestamp(&self) -> MilliSecondsSinceUnixEpoch {
        self.event.origin_server_ts
    }

    /// The textual content of this message.
    #[must_use]
    pub fn body(&self) -> String {
        self.event.content.msgtype.body().to_owned()
    }
}

/// The search of the messages of a room.
///
/// Cheap to clone; every clone shares the same state, so a task can run a
/// page while the owner keeps a handle to restart the search.
#[derive(Debug, Clone)]
pub struct RoomSearch {
    inner: Arc<RoomSearchInner>,
}

#[derive(Debug)]
struct RoomSearchInner {
    /// The room to search in.
    room: Room,
    /// The maximum number of results in a page.
    page_size: usize,
    /// The loading state of the list.
    loading_state: SharedObservable<LoadingState>,
    /// The mutable search state, never held across an await.
    state: Mutex<SearchState>,
}

#[derive(Debug, Default)]
struct SearchState {
    /// The term that is currently searched.
    search_term: String,
    /// The results of the current search, in the order they were presented.
    results: Vec<SearchResult>,
    /// Whether all the results of the current search were loaded.
    has_reached_end: bool,
    /// The next batch to continue the search on the server, if any.
    next_batch: Option<String>,
    /// The results of the local search index that were not presented yet.
    ///
    /// The local index returns all its results at once, so they are
    /// presented page by page from here.
    pending: Vec<OriginalSyncRoomMessageEvent>,
    /// The number of the current search.
    ///
    /// It is used to ignore the response of a search that is not the
    /// current one anymore.
    generation: u64,
}

impl RoomSearch {
    /// Construct a new `RoomSearch` for the given room, with pages of the
    /// application's size.
    #[must_use]
    pub fn new(room: &Room) -> Self {
        Self::with_page_size(room, RESULTS_PAGE_SIZE)
    }

    /// Construct a new `RoomSearch` for the given room, with pages of the
    /// given size.
    ///
    /// # Panics
    ///
    /// Panics if `page_size` is zero, because a page that can never hold
    /// a result would never reach the end.
    #[must_use]
    pub fn with_page_size(room: &Room, page_size: usize) -> Self {
        assert!(page_size > 0, "a search page holds at least one result");

        Self {
            inner: Arc::new(RoomSearchInner {
                room: room.clone(),
                page_size,
                loading_state: SharedObservable::new(LoadingState::Initial),
                state: Mutex::new(SearchState::default()),
            }),
        }
    }

    /// The room to search in.
    #[must_use]
    pub fn room(&self) -> &Room {
        &self.inner.room
    }

    /// The term that is currently searched.
    #[must_use]
    pub fn search_term(&self) -> String {
        self.state().search_term.clone()
    }

    /// The results of the current search, in the order they were loaded.
    #[must_use]
    pub fn results(&self) -> Vec<SearchResult> {
        self.state().results.clone()
    }

    /// Whether the list of results is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.state().results.is_empty()
    }

    /// The loading state of the list.
    #[must_use]
    pub fn loading_state(&self) -> LoadingState {
        self.inner.loading_state.get()
    }

    /// Subscribe to the loading state of the list.
    pub fn subscribe_loading_state(&self) -> eyeball::Subscriber<LoadingState> {
        self.inner.loading_state.subscribe()
    }

    /// Whether all the results of the current search were loaded.
    #[must_use]
    pub fn has_reached_end(&self) -> bool {
        self.state().has_reached_end
    }

    /// Whether more results can be loaded with the current search.
    #[must_use]
    pub fn can_load_more(&self) -> bool {
        let state = self.state();
        self.loading_state() != LoadingState::Loading
            && !state.has_reached_end
            && !state.search_term.is_empty()
    }

    /// Set the term to search.
    ///
    /// This restarts the search from scratch: the results are cleared, and
    /// the response of a page still in flight is dropped when it arrives.
    /// The first page is loaded with [`Self::load_more()`].
    pub fn set_search_term(&self, search_term: &str) {
        let search_term = search_term.trim();

        let mut state = self.state();
        if state.search_term == search_term {
            return;
        }

        // Any ongoing search is not the current one anymore.
        state.generation = state.generation.wrapping_add(1);
        search_term.clone_into(&mut state.search_term);
        state.results.clear();
        state.next_batch = None;
        state.pending.clear();
        state.has_reached_end = false;
        let is_empty = state.search_term.is_empty();
        drop(state);

        if is_empty {
            self.inner.loading_state.set_if_not_eq(LoadingState::Ready);
        }
    }

    /// Load more results.
    ///
    /// Returns the results that were added, which is nothing when the term
    /// is empty, when the end was reached, or when the term changed while
    /// the page was loading.
    pub async fn load_more(&self) -> Result<Vec<SearchResult>, SearchError> {
        let (search_term, generation) = {
            let state = self.state();
            (state.search_term.clone(), state.generation)
        };

        if search_term.is_empty() {
            return Ok(Vec::new());
        }

        self.inner
            .loading_state
            .set_if_not_eq(LoadingState::Loading);

        if self.inner.room.is_encrypted() {
            self.load_local(search_term, generation).await
        } else {
            self.load_from_server(search_term, generation).await
        }
    }

    /// Load more results from the server.
    ///
    /// The server cannot search the content of encrypted rooms, but it can
    /// search the whole history of other rooms.
    async fn load_from_server(
        &self,
        search_term: String,
        generation: u64,
    ) -> Result<Vec<SearchResult>, SearchError> {
        let room = &self.inner.room;
        let room_id = room.room_id().to_owned();
        let client = room.matrix_room().client();
        let next_batch = self.state().next_batch.clone();
        let page_size = self.inner.page_size;

        let handle = spawn_tokio!(async move {
            let filter = assign!(RoomEventFilter::default(), {
                rooms: Some(vec![room_id]),
                types: Some(vec![MessageLikeEventType::RoomMessage.to_string()]),
                limit: UInt::try_from(page_size).ok(),
            });
            let criteria = assign!(Criteria::new(search_term), {
                keys: Some(vec![SearchKeys::ContentBody]),
                filter,
                // Show the most recent messages first: in a conversation, the
                // latest match is usually the interesting one.
                order_by: Some(OrderBy::Recent),
                // The context of a result is not presented, the user opens the
                // message in the timeline instead.
                event_context: assign!(EventContext::new(), {
                    before_limit: UInt::default(),
                    after_limit: UInt::default(),
                }),
            });
            let categories = assign!(Categories::new(), { room_events: Some(criteria) });
            let request = assign!(search_events::v3::Request::new(categories), {
                next_batch,
            });

            client.send(request).await
        });

        let result = handle.await.expect("task was not aborted");

        if self.state().generation != generation {
            // This is not the current search anymore, ignore the response.
            return Ok(Vec::new());
        }

        let response = match result {
            Ok(response) => response,
            Err(error) => {
                self.inner.loading_state.set_if_not_eq(LoadingState::Error);
                return Err(Box::new(error).into());
            }
        };

        let room_events = response.search_categories.room_events;

        let results = room_events
            .results
            .iter()
            .filter_map(|result| result.result.as_ref())
            // The server returns the event with its room ID, which we do not need
            // since we search in a single room.
            .map(Raw::cast_ref_unchecked::<AnySyncTimelineEvent>)
            .filter_map(original_message_event_from_raw)
            .map(|event| SearchResult { event })
            .collect::<Vec<_>>();

        {
            let mut state = self.state();
            state.has_reached_end = room_events.next_batch.is_none();
            state.next_batch = room_events.next_batch;
            state.results.extend(results.iter().cloned());
        }
        self.inner.loading_state.set_if_not_eq(LoadingState::Ready);

        Ok(results)
    }

    /// Load more results from the local search index.
    ///
    /// This is the only way to search an encrypted room, since the server
    /// cannot read its content. It can only find messages that this device
    /// has received and indexed.
    async fn load_local(
        &self,
        search_term: String,
        generation: u64,
    ) -> Result<Vec<SearchResult>, SearchError> {
        if !self.state().pending.is_empty() {
            // The results were already fetched, just present the next page.
            return Ok(self.present_pending_page());
        }

        let query = sanitize_local_query(&search_term);

        if query.is_empty() {
            self.state().has_reached_end = true;
            self.inner.loading_state.set_if_not_eq(LoadingState::Ready);
            return Ok(Vec::new());
        }

        let matrix_room = self.inner.room.matrix_room().clone();
        let handle = spawn_tokio!(async move {
            // The error type of the index belongs to a crate that is not a direct
            // dependency, so it cannot be named here.
            let results = matrix_room
                .search(&query, LOCAL_MAX_RESULTS, None)
                .await
                .map_err(|error| error.to_string())?;

            if results.len() == LOCAL_MAX_RESULTS {
                debug!(
                    "Reached the maximum number of local search results, some messages \
                     matching the search are not presented"
                );
            }

            // The index only stores event IDs, so the events themselves must be
            // loaded from the local store.
            let mut events = Vec::with_capacity(results.len());
            for (_score, event_id) in results {
                match matrix_room.load_or_fetch_event(&event_id, None).await {
                    Ok(event) => events.push(event),
                    Err(error) => {
                        warn!("Could not load search result {event_id}: {error}");
                    }
                }
            }

            Ok::<_, String>(events)
        });

        let result = handle.await.expect("task was not aborted");

        if self.state().generation != generation {
            // This is not the current search anymore, ignore the response.
            return Ok(Vec::new());
        }

        let events = match result {
            Ok(events) => events,
            Err(error) => {
                // The query parser rejects a syntax that we cannot always prevent,
                // and that is not an error the user can act on: there is simply
                // nothing to show.
                debug!("Could not search messages locally: {error}");
                self.state().has_reached_end = true;
                self.inner.loading_state.set_if_not_eq(LoadingState::Ready);
                return Ok(Vec::new());
            }
        };

        // The index sorts results by relevance, but a conversation is read by
        // recency, so present the most recent matches first.
        let mut message_events = events
            .iter()
            .filter_map(|event| original_message_event_from_raw(event.raw()))
            .collect::<Vec<_>>();
        message_events.sort_by_key(|event| std::cmp::Reverse(event.origin_server_ts));

        self.state().pending = message_events;
        Ok(self.present_pending_page())
    }

    /// Present the next page of results that were already fetched.
    fn present_pending_page(&self) -> Vec<SearchResult> {
        let results = {
            let mut state = self.state();
            let page_len = state.pending.len().min(self.inner.page_size);
            let results = state
                .pending
                .drain(..page_len)
                .map(|event| SearchResult { event })
                .collect::<Vec<_>>();
            state.has_reached_end = state.pending.is_empty();
            state.results.extend(results.iter().cloned());
            results
        };

        self.inner.loading_state.set_if_not_eq(LoadingState::Ready);
        results
    }

    /// Add the messages that are loaded in the room to its local search
    /// index.
    ///
    /// The index is only fed as the event cache stores an event, so an
    /// event that was already stored when the index was created was never
    /// handed to it, and no search can find it. That is every message a
    /// device received before it had an index at all. This hands the events
    /// the room has loaded over after the fact; loading more of the history
    /// and doing it again covers more of it.
    ///
    /// The search is restarted afterwards, so that the messages that were
    /// just added are presented: the caller loads the first page again.
    pub async fn reindex(&self) -> Result<(), SearchError> {
        if self.loading_state() == LoadingState::Loading {
            return Ok(());
        }

        let matrix_room = self.inner.room.matrix_room().clone();

        self.inner
            .loading_state
            .set_if_not_eq(LoadingState::Loading);

        let handle = spawn_tokio!(async move {
            let client = matrix_room.client();
            let room_id = matrix_room.room_id();

            let (room_cache, _drop_handles) = client
                .event_cache()
                .room(room_id)
                .await
                .map_err(|error| error.to_string())?;
            let events = room_cache
                .events()
                .await
                .map_err(|error| error.to_string())?;
            let redaction_rules = matrix_room
                .clone_info()
                .room_version_rules_or_default()
                .redaction;

            // The error type of the index belongs to a crate that is not a direct
            // dependency, so it cannot be named here.
            client
                .search_index()
                .lock()
                .await
                .bulk_handle_timeline_event(
                    events.into_iter(),
                    &room_cache,
                    room_id,
                    &redaction_rules,
                )
                .await
                .map_err(|error| error.to_string())
        });

        if let Err(error) = handle.await.expect("task was not aborted") {
            // Leaving the state as it is would show a spinner that never stops.
            self.inner.loading_state.set_if_not_eq(LoadingState::Error);
            return Err(SearchError::Index(error));
        }

        // Search again, so that the messages that were just added are presented.
        self.restart();
        Ok(())
    }

    /// Search the current term again, from scratch.
    fn restart(&self) {
        let mut state = self.state();
        state.generation = state.generation.wrapping_add(1);
        state.results.clear();
        state.next_batch = None;
        state.pending.clear();
        state.has_reached_end = false;
        let is_empty = state.search_term.is_empty();
        drop(state);

        if is_empty {
            self.inner.loading_state.set_if_not_eq(LoadingState::Ready);
        }
    }

    /// The mutable search state.
    fn state(&self) -> std::sync::MutexGuard<'_, SearchState> {
        self.inner.state.lock().expect("mutex is not poisoned")
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize_local_query;

    #[test]
    fn sanitize_query_quotes_every_token() {
        assert_eq!(sanitize_local_query("hello world"), r#""hello" "world""#);
    }

    #[test]
    fn sanitize_query_removes_special_chars() {
        // These would all make the query parser of the index return an error.
        assert_eq!(sanitize_local_query(r#"a"b there?"#), r#""ab" "there?""#);
        assert_eq!(sanitize_local_query("a:b"), r#""ab""#);
        assert_eq!(sanitize_local_query("(foo)"), r#""foo""#);
        assert_eq!(sanitize_local_query("-bar^2"), r#""bar2""#);
    }

    #[test]
    fn sanitize_query_drops_empty_tokens() {
        assert_eq!(sanitize_local_query("  foo   "), r#""foo""#);
        // A term that is only special chars leaves nothing to search.
        assert_eq!(sanitize_local_query("^^^"), "");
        assert_eq!(sanitize_local_query(""), "");
    }

    /// The characters that the query parser of the index treats as syntax.
    ///
    /// This repeats the list that [`sanitize_local_query()`] strips, on
    /// purpose: checking against the constant that the implementation uses
    /// would make this test pass whenever a character is dropped from it,
    /// which is the mistake it exists to catch.
    const PARSER_SYNTAX: [char; 16] = [
        '"', '\'', '+', '-', '!', '(', ')', '{', '}', '[', ']', '^', '~', '*', ':', '\\',
    ];

    /// Search terms that a person could reasonably type, and that the query
    /// parser of the index rejects with an error when they reach it as they
    /// are.
    const AWKWARD_SEARCH_TERMS: &[&str] = &[
        r#"foo""#,
        "a:b",
        "(",
        ")",
        "^^^",
        "***",
        "?",
        "~",
        "!",
        "+",
        "-",
        "[",
        "]",
        "{",
        "}",
        "\\",
        r#""unclosed"#,
        "who's there?",
        "-bar^2",
        "a AND b",
        "title:foo",
        "1 + 1 = 2",
        "wait... what?!",
        "https://example.org/a?b=c",
        "#room:example.org",
        "@user:example.org",
        "C:\\path",
        "50%",
        "",
        "   ",
        "\t\n",
    ];

    #[test]
    fn sanitize_query_never_emits_syntax_the_parser_rejects() {
        for term in AWKWARD_SEARCH_TERMS {
            let query = sanitize_local_query(term);

            if query.is_empty() {
                // Nothing left to search, which is handled as no results.
                continue;
            }

            // The output must be a sequence of quoted tokens, which is the
            // simplest syntax the parser accepts.
            for (index, part) in query.split(' ').enumerate() {
                assert!(
                    part.len() >= 2 && part.starts_with('"') && part.ends_with('"'),
                    "token {index} of {query:?} (from {term:?}) is not quoted",
                );

                let inner = &part[1..part.len() - 1];
                assert!(
                    !inner.is_empty(),
                    "token {index} of {query:?} (from {term:?}) is empty",
                );
                assert!(
                    !inner.contains(PARSER_SYNTAX),
                    "token {index} of {query:?} (from {term:?}) still has syntax \
                     that the parser would try to interpret",
                );
            }
        }
    }
}
