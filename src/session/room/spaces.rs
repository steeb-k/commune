use matrix_sdk::ruma::events::space::{
    child::SpaceChildEventContent, parent::SpaceParentEventContent,
};
use tracing::{error, warn};

use super::Room;
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
