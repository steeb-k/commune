use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};
use ruma::{
    OwnedEventId, OwnedUserId, UInt,
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
use tokio::task::AbortHandle;
use tracing::{debug, error, warn};

use super::{Member, Room};
use crate::{
    spawn, spawn_tokio,
    utils::{
        LoadingState,
        matrix::{original_message_event_from_raw, timestamp_to_date},
    },
};

/// The maximum number of results in a page.
const RESULTS_PAGE_SIZE: usize = 20;
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

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::RoomSearch)]
    pub struct RoomSearch {
        /// The room to search in.
        #[property(get, set = Self::set_room, construct_only)]
        room: OnceCell<Room>,
        /// The list of results for the current search.
        #[property(get = Self::list_owned)]
        list: OnceCell<gio::ListStore>,
        /// The term that is currently searched.
        #[property(get)]
        search_term: RefCell<String>,
        /// The loading state of the list.
        #[property(get, builder(LoadingState::default()))]
        loading_state: Cell<LoadingState>,
        /// Whether all the results of the current search were loaded.
        #[property(get)]
        has_reached_end: Cell<bool>,
        /// The next batch to continue the search on the server, if any.
        next_batch: RefCell<Option<String>>,
        /// The results of the local search index that were not presented yet.
        ///
        /// The local index returns all its results at once, so they are
        /// presented page by page from here.
        pending: RefCell<Vec<OriginalSyncRoomMessageEvent>>,
        /// The members of the room.
        ///
        /// A strong reference is kept so that all results use the same list.
        room_members: RefCell<Option<crate::session::MemberList>>,
        /// The abort handle for the current request.
        abort_handle: RefCell<Option<AbortHandle>>,
        /// The number of the current search.
        ///
        /// It is used to ignore the response of a search that is not the
        /// current one anymore.
        generation: Cell<u64>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RoomSearch {
        const NAME: &'static str = "RoomSearch";
        type Type = super::RoomSearch;
    }

    #[glib::derived_properties]
    impl ObjectImpl for RoomSearch {
        fn dispose(&self) {
            if let Some(handle) = self.abort_handle.take() {
                handle.abort();
            }
        }
    }

    impl RoomSearch {
        /// Set the room to search in.
        fn set_room(&self, room: Room) {
            let room = self.room.get_or_init(|| room);
            self.room_members
                .replace(Some(room.get_or_create_members()));
        }

        /// The room to search in.
        fn room(&self) -> &Room {
            self.room.get().expect("room should be initialized")
        }

        /// The list of results for the current search.
        fn list(&self) -> &gio::ListStore {
            self.list
                .get_or_init(gio::ListStore::new::<super::RoomSearchResult>)
        }

        /// The owned list of results for the current search.
        fn list_owned(&self) -> gio::ListStore {
            self.list().clone()
        }

        /// Whether the list is empty.
        pub(super) fn is_empty(&self) -> bool {
            self.list().n_items() == 0
        }

        /// Set the loading state of the list.
        fn set_loading_state(&self, state: LoadingState) {
            if self.loading_state.get() == state {
                return;
            }

            self.loading_state.set(state);
            self.obj().notify_loading_state();
        }

        /// Set whether all the results of the current search were loaded.
        fn set_has_reached_end(&self, has_reached_end: bool) {
            if self.has_reached_end.get() == has_reached_end {
                return;
            }

            self.has_reached_end.set(has_reached_end);
            self.obj().notify_has_reached_end();
        }

        /// Set the term to search.
        pub(super) fn set_search_term(&self, search_term: &str) {
            let search_term = search_term.trim().to_owned();

            if *self.search_term.borrow() == search_term {
                return;
            }

            self.search_term.replace(search_term.clone());
            self.obj().notify_search_term();

            // Any ongoing search is not the current one anymore.
            self.generation.set(self.generation.get().wrapping_add(1));
            if let Some(handle) = self.abort_handle.take() {
                handle.abort();
            }

            self.list().remove_all();
            self.next_batch.take();
            self.pending.take();
            self.set_has_reached_end(false);

            if search_term.is_empty() {
                self.set_loading_state(LoadingState::Ready);
                return;
            }

            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    imp.load().await;
                }
            ));
        }

        /// Whether we can load more results with the current search.
        pub(super) fn can_load_more(&self) -> bool {
            self.loading_state.get() != LoadingState::Loading
                && !self.has_reached_end.get()
                && !self.search_term.borrow().is_empty()
        }

        /// Load more results.
        pub(super) async fn load(&self) {
            let search_term = self.search_term.borrow().clone();

            if search_term.is_empty() {
                return;
            }

            self.set_loading_state(LoadingState::Loading);

            if self.room().is_encrypted() {
                self.load_local(search_term).await;
            } else {
                self.load_from_server(search_term).await;
            }
        }

        /// Load more results from the server.
        ///
        /// The server cannot search the content of encrypted rooms, but it can
        /// search the whole history of other rooms.
        async fn load_from_server(&self, search_term: String) {
            let generation = self.generation.get();
            let room = self.room();
            let room_id = room.room_id().to_owned();
            let client = room.matrix_room().client();
            let next_batch = self.next_batch.borrow().clone();

            let handle = spawn_tokio!(async move {
                let filter = assign!(RoomEventFilter::default(), {
                    rooms: Some(vec![room_id]),
                    types: Some(vec![MessageLikeEventType::RoomMessage.to_string()]),
                    limit: UInt::try_from(RESULTS_PAGE_SIZE).ok(),
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

            self.abort_handle.replace(Some(handle.abort_handle()));

            let Ok(result) = handle.await else {
                // The request was aborted.
                self.abort_handle.take();
                return;
            };

            self.abort_handle.take();

            if self.generation.get() != generation {
                // This is not the current search anymore, ignore the response.
                return;
            }

            let response = match result {
                Ok(response) => response,
                Err(error) => {
                    error!("Could not search messages: {error}");
                    self.set_loading_state(LoadingState::Error);
                    return;
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
                .filter_map(|raw| self.result_from_raw(raw))
                .collect::<Vec<_>>();

            self.next_batch.replace(room_events.next_batch.clone());
            self.set_has_reached_end(room_events.next_batch.is_none());

            self.list().extend_from_slice(&results);
            self.set_loading_state(LoadingState::Ready);
        }

        /// Load more results from the local search index.
        ///
        /// This is the only way to search an encrypted room, since the server
        /// cannot read its content. It can only find messages that this device
        /// has received and indexed.
        async fn load_local(&self, search_term: String) {
            if !self.pending.borrow().is_empty() {
                // The results were already fetched, just present the next page.
                self.present_pending_page();
                return;
            }

            let generation = self.generation.get();
            let matrix_room = self.room().matrix_room().clone();
            let query = sanitize_local_query(&search_term);

            if query.is_empty() {
                self.set_has_reached_end(true);
                self.set_loading_state(LoadingState::Ready);
                return;
            }

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

            self.abort_handle.replace(Some(handle.abort_handle()));

            let Ok(result) = handle.await else {
                // The request was aborted.
                self.abort_handle.take();
                return;
            };

            self.abort_handle.take();

            if self.generation.get() != generation {
                // This is not the current search anymore, ignore the response.
                return;
            }

            let events = match result {
                Ok(events) => events,
                Err(error) => {
                    // The query parser rejects a syntax that we cannot always prevent,
                    // and that is not an error the user can act on: there is simply
                    // nothing to show.
                    debug!("Could not search messages locally: {error}");
                    self.set_has_reached_end(true);
                    self.set_loading_state(LoadingState::Ready);
                    return;
                }
            };

            // The index sorts results by relevance, but a conversation is read by
            // recency, so present the most recent matches first.
            let mut message_events = events
                .iter()
                .filter_map(|event| original_message_event_from_raw(event.raw()))
                .collect::<Vec<_>>();
            message_events.sort_by_key(|event| std::cmp::Reverse(event.origin_server_ts));

            self.pending.replace(message_events);
            self.present_pending_page();
        }

        /// Present the next page of results that were already fetched.
        fn present_pending_page(&self) {
            let mut pending = self.pending.borrow_mut();
            let page_len = pending.len().min(RESULTS_PAGE_SIZE);
            let page = pending.drain(..page_len).collect::<Vec<_>>();
            let has_reached_end = pending.is_empty();
            drop(pending);

            let results = page
                .into_iter()
                .map(|message_event| self.result_from_message_event(message_event))
                .collect::<Vec<_>>();

            self.list().extend_from_slice(&results);
            self.set_has_reached_end(has_reached_end);
            self.set_loading_state(LoadingState::Ready);
        }

        /// Construct a result for the given raw event, if it is a message.
        fn result_from_raw(
            &self,
            raw: &Raw<AnySyncTimelineEvent>,
        ) -> Option<super::RoomSearchResult> {
            let message_event = original_message_event_from_raw(raw)?;
            Some(self.result_from_message_event(message_event))
        }

        /// Construct a result for the given message event.
        fn result_from_message_event(
            &self,
            message_event: OriginalSyncRoomMessageEvent,
        ) -> super::RoomSearchResult {
            super::RoomSearchResult::new(self.room(), message_event)
        }
    }
}

glib::wrapper! {
    /// The search of the messages of a room.
    ///
    /// Messages of an encrypted room can only be searched in the local search
    /// index, since the server cannot read their content. Other rooms are
    /// searched on the server, which knows the whole history of the room.
    pub struct RoomSearch(ObjectSubclass<imp::RoomSearch>);
}

impl RoomSearch {
    /// Construct a new `RoomSearch` for the given room.
    pub(crate) fn new(room: &Room) -> Self {
        glib::Object::builder().property("room", room).build()
    }

    /// Whether the list of results is empty.
    pub(crate) fn is_empty(&self) -> bool {
        self.imp().is_empty()
    }

    /// Set the term to search.
    ///
    /// This restarts the search from scratch.
    pub(crate) fn set_search_term(&self, search_term: &str) {
        self.imp().set_search_term(search_term);
    }

    /// Whether more results can be loaded with the current search.
    pub(crate) fn can_load_more(&self) -> bool {
        self.imp().can_load_more()
    }

    /// Load more results.
    pub(crate) fn load_more(&self) {
        let imp = self.imp();

        if !imp.can_load_more() {
            return;
        }

        spawn!(clone!(
            #[weak]
            imp,
            async move {
                imp.load().await;
            }
        ));
    }
}

mod result_imp {
    use std::cell::OnceCell;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::RoomSearchResult)]
    pub struct RoomSearchResult {
        /// The room containing this message.
        #[property(get, construct_only)]
        pub(super) room: glib::WeakRef<Room>,
        /// The Matrix event.
        matrix_event: OnceCell<OriginalSyncRoomMessageEvent>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RoomSearchResult {
        const NAME: &'static str = "RoomSearchResult";
        type Type = super::RoomSearchResult;
    }

    #[glib::derived_properties]
    impl ObjectImpl for RoomSearchResult {}

    impl RoomSearchResult {
        /// Set the Matrix event.
        pub(super) fn set_matrix_event(&self, event: OriginalSyncRoomMessageEvent) {
            self.matrix_event
                .set(event)
                .expect("Matrix event should be uninitialized");
        }

        /// The Matrix event.
        pub(super) fn matrix_event(&self) -> &OriginalSyncRoomMessageEvent {
            self.matrix_event
                .get()
                .expect("Matrix event should be initialized")
        }
    }
}

glib::wrapper! {
    /// A message matching a search in a room.
    pub struct RoomSearchResult(ObjectSubclass<result_imp::RoomSearchResult>);
}

impl RoomSearchResult {
    /// Construct a new `RoomSearchResult` for the given message.
    fn new(room: &Room, matrix_event: OriginalSyncRoomMessageEvent) -> Self {
        let obj = glib::Object::builder::<Self>()
            .property("room", room)
            .build();
        obj.imp().set_matrix_event(matrix_event);
        obj
    }

    /// The ID of the event of this result.
    pub(crate) fn event_id(&self) -> OwnedEventId {
        self.imp().matrix_event().event_id.clone()
    }

    /// The ID of the sender of this message.
    pub(crate) fn sender_id(&self) -> OwnedUserId {
        self.imp().matrix_event().sender.clone()
    }

    /// The sender of this message.
    pub(crate) fn sender(&self) -> Option<Member> {
        let room = self.room()?;
        Some(room.get_or_create_members().get_or_create(self.sender_id()))
    }

    /// The timestamp of this message, as a `GDateTime`.
    pub(crate) fn timestamp(&self) -> glib::DateTime {
        timestamp_to_date(self.imp().matrix_event().origin_server_ts)
    }

    /// The textual content of this message.
    pub(crate) fn body(&self) -> String {
        self.imp().matrix_event().content.msgtype.body().to_owned()
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
