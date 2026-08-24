use matrix_sdk::{
    Room as MatrixRoom,
    deserialized_responses::SyncOrStrippedState,
    ruma::events::{
        StateEventType,
        space::{child::SpaceChildEventContent, parent::SpaceParentEventContent},
    },
};
use ruma::RoomId;
use serde_json::json;
use tracing::{error, warn};

use super::{Room, RoomCategory};
use crate::spawn_tokio;

/// Put the given room inside the given space.
///
/// This writes both halves of the relationship the specification defines, and
/// they are not equally important:
///
/// * `m.space.child` in the **space** is the one that counts. The `/hierarchy`
///   endpoint is built from it, and a space that carries no child event for a
///   room does not contain that room, whatever the room says.
/// * `m.space.parent` in the **room** is how the room claims which space it
///   belongs to. It needs power in the room rather than in the space — a
///   different permission, often a different person — so failing to write it is
///   logged and otherwise ignored. The room is in the space either way.
///
/// `canonical` is deliberately left `false` on the parent event: it means "this
/// is the room's main space", and nothing here knows whether the room already
/// has one.
pub(crate) async fn add_room_to_space(room: &Room, space: &Room) -> Result<(), ()> {
    let matrix_room = room.matrix_room().clone();
    let matrix_space = space.matrix_room().clone();
    let room_id = room.room_id().to_owned();
    let space_id = space.room_id().to_owned();

    let handle = spawn_tokio!(async move {
        // The `via` of each event names servers that can be used to reach the
        // room it points at, so the two are not interchangeable. `route()`
        // already leaves out servers the room's ACL excludes.
        let room_via = matrix_room.route().await.unwrap_or_default();
        let space_via = matrix_space.route().await.unwrap_or_default();

        matrix_space
            .send_state_event_for_key(&room_id, SpaceChildEventContent::new(room_via))
            .await?;

        if let Err(error) = matrix_room
            .send_state_event_for_key(&space_id, SpaceParentEventContent::new(space_via))
            .await
        {
            warn!("Could not claim the parent space from within the room: {error}");
        }

        Ok::<_, matrix_sdk::Error>(())
    });

    if let Err(error) = handle.await.expect("task was not aborted") {
        error!("Could not add the room to the space: {error}");
        return Err(());
    }

    Ok(())
}

/// Take the given room back out of the given space.
///
/// The specification has no "delete a state event": a relationship is undone
/// by writing the event again with nothing in it, since a child with no `via`
/// is not a child. Redacting would work too and leaves a hole in the space's
/// timeline instead; this is what other clients do.
///
/// As with adding, the child is the half that matters and the parent is best
/// effort — a room whose administrators are somebody else keeps its stale
/// `m.space.parent`, which nothing will believe, because the space no longer
/// names it as a child.
pub(crate) async fn remove_room_from_space(room: &Room, space: &Room) -> Result<(), ()> {
    let matrix_room = room.matrix_room().clone();
    let matrix_space = space.matrix_room().clone();
    let room_id = room.room_id().to_owned();
    let space_id = space.room_id().to_owned();

    let handle = spawn_tokio!(async move {
        matrix_space
            .send_state_event_raw("m.space.child", room_id.as_str(), json!({}))
            .await?;

        if let Err(error) = matrix_room
            .send_state_event_raw("m.space.parent", space_id.as_str(), json!({}))
            .await
        {
            warn!("Could not drop the parent space claim from within the room: {error}");
        }

        Ok::<_, matrix_sdk::Error>(())
    });

    if let Err(error) = handle.await.expect("task was not aborted") {
        error!("Could not remove the room from the space: {error}");
        return Err(());
    }

    Ok(())
}

/// The spaces that hold the given room, among the ones this account has
/// joined.
///
/// Only joined spaces can be listed: the state of a space nobody here is in is
/// not ours to read, so a room can be inside a space this never mentions.
///
/// The specification is careful about which direction to believe. A space
/// naming a room as its child settles it. A room naming a space as its parent
/// does not, on its own — anybody can claim to belong to anything — and counts
/// only when whoever wrote the claim could have written the child event in
/// that space. Both are checked here, in that order.
pub(crate) async fn parent_spaces(room: &Room) -> Vec<Room> {
    let Some(session) = room.session() else {
        return Vec::new();
    };

    let spaces = session
        .room_list()
        .snapshot()
        .into_iter()
        .filter(|room| room.category() == RoomCategory::Space)
        .collect::<Vec<_>>();

    let mut parents = Vec::new();

    for space in spaces {
        let matrix_room = room.matrix_room().clone();
        let matrix_space = space.matrix_room().clone();
        let room_id = room.room_id().to_owned();
        let space_id = space.room_id().to_owned();

        let handle = spawn_tokio!(async move {
            is_child_of(&matrix_space, &room_id).await
                || claims_parent(&matrix_room, &matrix_space, &space_id).await
        });

        if handle.await.expect("task was not aborted") {
            parents.push(space);
        }
    }

    parents
}

/// Whether the given space names the given room as one of its children.
async fn is_child_of(space: &MatrixRoom, room_id: &RoomId) -> bool {
    let Ok(Some(raw)) = space
        .get_state_event_static_for_key::<SpaceChildEventContent, _>(room_id)
        .await
    else {
        return false;
    };

    let Ok(event) = raw.deserialize() else {
        return false;
    };

    // A child with no servers to reach it through is not a child. That is how
    // the relationship is undone, and how `is_valid` reads it.
    match event {
        SyncOrStrippedState::Sync(event) => event
            .as_original()
            .is_some_and(|event| !event.content.via.is_empty()),
        // A stripped event carries the possibly-redacted content, where `via`
        // is optional — which is exactly the shape of a removal.
        SyncOrStrippedState::Stripped(event) => event
            .content
            .via
            .as_ref()
            .is_some_and(|via| !via.is_empty()),
    }
}

/// Whether the given room claims the given space as a parent, in a way worth
/// believing.
async fn claims_parent(room: &MatrixRoom, space: &MatrixRoom, space_id: &RoomId) -> bool {
    let Ok(Some(raw)) = room
        .get_state_event_static_for_key::<SpaceParentEventContent, _>(space_id)
        .await
    else {
        return false;
    };

    let Ok(event) = raw.deserialize() else {
        return false;
    };

    let (sender, via) = match event {
        SyncOrStrippedState::Sync(event) => {
            let Some(event) = event.as_original() else {
                return false;
            };

            (event.sender.clone(), event.content.via.clone())
        }
        SyncOrStrippedState::Stripped(event) => (
            event.sender.clone(),
            event.content.via.clone().unwrap_or_default(),
        ),
    };

    if via.is_empty() {
        return false;
    }

    // The claim is only worth believing from somebody who could have made it
    // true from the other side. Otherwise any room could announce itself as
    // part of any space.
    space
        .power_levels_or_default()
        .await
        .user_can_send_state(&sender, StateEventType::SpaceChild)
}
