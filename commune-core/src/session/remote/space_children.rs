//! The hierarchy of a space, as far as its homeserver will describe it.
//!
//! The headless counterpart of the application's `SpaceChildren` and
//! `SpaceChild` (`src/session/remote/space_children.rs`). The whole tree
//! is asked for at once — `/hierarchy` walks it depth-first and returns
//! every room with the `m.space.child` events of the spaces among them —
//! so opening a subspace costs no request and never waits. The listing
//! stops after a fixed number of batches and says so.
//!
//! What stayed in the application is the `gio::ListStore` of rows, the
//! abort handle that cancels a walk when the space changes under it, and
//! `GtkTreeListModel`'s rule that a row's children are asked for once.

use std::collections::HashMap;

use ruma::{
    OwnedRoomId, OwnedServerName, RoomId,
    api::client::space::get_hierarchy,
    assign,
    events::space::child::{HierarchySpaceChildEvent, SpaceChildOrd},
    room::RoomSummary,
    serde::Raw,
};
use tracing::{debug, error, warn};

use super::RemoteRoom;
use crate::{UserFacingError, matrix::MatrixRoomIdUri, session::Session, spawn_tokio};

/// The maximum number of rooms to ask for at a time.
const BATCH_SIZE: u32 = 20;

/// The maximum number of batches to walk through for one space.
///
/// The endpoint paginates and a space can hold thousands of rooms, so
/// something has to stop. When this is reached the list says so rather than
/// pretending to be complete.
const MAX_BATCHES: usize = 10;

/// What can go wrong while walking a space's hierarchy.
#[derive(Debug, thiserror::Error)]
pub enum SpaceChildrenError {
    /// The homeserver could not describe the hierarchy.
    ///
    /// Boxed because `matrix_sdk::HttpError` is large enough that carrying
    /// it by value makes every `Result` in this module expensive.
    #[error(transparent)]
    Server(#[from] Box<matrix_sdk::HttpError>),
}

impl UserFacingError for SpaceChildrenError {
    fn to_user_facing(&self) -> String {
        match self {
            // The embedder has its own rendering of an SDK error — the GTK
            // application's is translated — so this is only the fallback.
            Self::Server(error) => error.to_string(),
        }
    }
}

/// One room inside a space, as the space describes it.
///
/// The `via` servers and the suggestion belong to the `m.space.child` event
/// rather than to the room, so a room reachable from two spaces can be
/// described differently by each.
#[derive(Debug, Clone)]
struct SpaceEdge {
    /// The room the space points at.
    room_id: OwnedRoomId,
    /// The servers to reach it through.
    via: Vec<OwnedServerName>,
    /// Whether the space recommends it.
    suggested: bool,
}

/// The hierarchy of a space, walked as far as the homeserver would
/// describe it.
#[derive(Debug, Clone)]
pub struct SpaceChildren {
    /// The ID of the space these rooms are inside.
    room_id: OwnedRoomId,
    /// What every room in the hierarchy is.
    summaries: HashMap<OwnedRoomId, RoomSummary>,
    /// What every space in the hierarchy holds, in the order the
    /// specification asks for.
    edges: HashMap<OwnedRoomId, Vec<SpaceEdge>>,
    /// Whether the listing stopped before the end of the space.
    is_truncated: bool,
}

impl SpaceChildren {
    /// Walk the whole hierarchy of the given space.
    pub async fn load(session: &Session, room_id: OwnedRoomId) -> Result<Self, SpaceChildrenError> {
        let mut hierarchy = Self {
            room_id,
            summaries: HashMap::new(),
            edges: HashMap::new(),
            is_truncated: false,
        };
        let mut next_batch = None;
        let mut truncated = true;

        for _ in 0..MAX_BATCHES {
            let response = hierarchy.load_batch(session, next_batch.take()).await?;
            next_batch.clone_from(&response.next_batch);
            hierarchy.remember(response);

            if next_batch.is_none() {
                truncated = false;
                break;
            }
        }

        if truncated {
            warn!(
                "Stopped walking the hierarchy of space `{}` after {MAX_BATCHES} batches",
                hierarchy.room_id
            );
            hierarchy.is_truncated = true;
        }

        if hierarchy.children().is_empty() {
            debug!(
                "Nothing to list in the hierarchy of space `{}`",
                hierarchy.room_id
            );
        }

        Ok(hierarchy)
    }

    /// Request one batch of the hierarchy.
    async fn load_batch(
        &self,
        session: &Session,
        from: Option<String>,
    ) -> Result<get_hierarchy::v1::Response, SpaceChildrenError> {
        let request = assign!(get_hierarchy::v1::Request::new(self.room_id.clone()), {
            from,
            limit: Some(BATCH_SIZE.into()),
            // No `max_depth`: the whole tree is asked for at once, so
            // expanding a subspace costs nothing and never waits. The
            // batch cap above is what bounds it.
        });

        let client = session.client();
        let handle = spawn_tokio!(async move { client.send(request).await });

        handle
            .await
            .expect("task was not aborted")
            .map_err(|hierarchy_error| {
                error!(
                    "Could not walk the hierarchy of space `{}`: {hierarchy_error}",
                    self.room_id
                );
                Box::new(hierarchy_error).into()
            })
    }

    /// Remember what the given response says about the hierarchy.
    fn remember(&mut self, response: get_hierarchy::v1::Response) {
        for chunk in response.rooms {
            let room_id = chunk.summary.room_id.clone();

            if !chunk.children_state.is_empty() {
                self.edges
                    .insert(room_id.clone(), space_edges(chunk.children_state));
            }

            self.summaries.insert(room_id, chunk.summary);
        }
    }

    /// The ID of the space these rooms are inside.
    #[must_use]
    pub fn room_id(&self) -> &RoomId {
        &self.room_id
    }

    /// Whether the listing stopped before the end of the space.
    #[must_use]
    pub fn is_truncated(&self) -> bool {
        self.is_truncated
    }

    /// The rooms directly inside the space.
    #[must_use]
    pub fn children(&self) -> Vec<SpaceChild> {
        self.children_of(&self.room_id, &[])
    }

    /// The rooms directly inside the given space, given the spaces already
    /// walked through to reach it.
    #[must_use]
    pub fn children_of(&self, room_id: &RoomId, ancestors: &[OwnedRoomId]) -> Vec<SpaceChild> {
        let Some(edges) = self.edges.get(room_id) else {
            return Vec::new();
        };

        edges
            .iter()
            .filter_map(|edge| {
                // A room the server could not reach has no summary, and
                // there is nothing to draw for it.
                let summary = self.summaries.get(&edge.room_id)?.clone();

                let id = summary
                    .canonical_alias
                    .clone()
                    .map_or_else(|| summary.room_id.clone().into(), Into::into);
                let mut room = RemoteRoom::with_data(
                    MatrixRoomIdUri {
                        id,
                        via: edge.via.clone(),
                    },
                    summary,
                );
                room.is_suggested = edge.suggested;

                Some(SpaceChild {
                    room,
                    ancestors: ancestors.to_owned(),
                })
            })
            .collect()
    }

    /// Whether the given space has anything in it that could be shown.
    #[must_use]
    pub fn holds_rooms(&self, room_id: &RoomId) -> bool {
        self.edges
            .get(room_id)
            .is_some_and(|edges| !edges.is_empty())
    }
}

/// The rooms named by the given `m.space.child` events, in the order the
/// specification defines.
///
/// The order is `order`, then the time the event was sent, then the room
/// ID. The server sorts the rooms it returns the same way, but the events
/// are a set, so this has to sort them itself.
fn space_edges(children_state: Vec<Raw<HierarchySpaceChildEvent>>) -> Vec<SpaceEdge> {
    let mut events = children_state
        .into_iter()
        .filter_map(|raw_event| match raw_event.deserialize() {
            Ok(event) => Some(event),
            Err(error) => {
                warn!("Could not deserialize `m.space.child` event: {error}");
                None
            }
        })
        // A child with no servers to reach it through is not a child. That
        // is how the relationship is undone.
        .filter(|event| !event.content.via.is_empty())
        .collect::<Vec<_>>();

    events.sort_by(SpaceChildOrd::cmp_space_child);

    events
        .into_iter()
        .map(|event| SpaceEdge {
            room_id: event.state_key,
            via: event.content.via,
            suggested: event.content.suggested,
        })
        .collect()
}

/// One room inside a space, and the way down to it.
///
/// The way down is what stops a hierarchy that points back at itself from
/// being opened forever: a space that is already above this row is not
/// offered again.
#[derive(Debug, Clone)]
pub struct SpaceChild {
    /// The room this is.
    pub room: RemoteRoom,
    /// The spaces walked through to reach this room, the outermost first.
    ancestors: Vec<OwnedRoomId>,
}

impl SpaceChild {
    /// The spaces walked through to reach this room, the outermost first.
    #[must_use]
    pub fn ancestors(&self) -> &[OwnedRoomId] {
        &self.ancestors
    }

    /// The rooms inside this one, if it is a space that holds any.
    ///
    /// Returns `None` for a room that is not a space, for a space the walk
    /// found nothing in, and for a space that is already one of the ones
    /// walked through to get here.
    #[must_use]
    pub fn children(&self, hierarchy: &SpaceChildren) -> Option<Vec<SpaceChild>> {
        if !self.room.is_space {
            return None;
        }

        let room_id = &self.room.room_id;

        if self.ancestors.contains(room_id) {
            // A space inside itself, however many steps around. Opening it
            // again would go round the same loop.
            return None;
        }

        if !hierarchy.holds_rooms(room_id) {
            return None;
        }

        let mut child_ancestors = self.ancestors.clone();
        child_ancestors.push(room_id.clone());

        let children = hierarchy.children_of(room_id, &child_ancestors);

        if children.is_empty() {
            return None;
        }

        Some(children)
    }
}
