//! Rooms the session is not in, described by the homeserver on request.
//!
//! The headless counterpart of the application's `session/remote/`: a
//! remote room is a value the homeserver hands back — from the summary
//! endpoint, the public directory or a space's hierarchy — rather than a
//! room sync keeps current. Since Phase 4's module 7 the cache is here too:
//! the entries a page follows, the staleness timer that asks again after a
//! day for a room and an hour for a profile, the peek at a room that can be
//! read without joining, and the preview of a URL. What stayed in the
//! application is the `GObject` around each, the avatar image and the
//! linkified topic.

mod cache;
mod room;
mod room_peek;
mod space_children;
mod url_preview;
mod user;

pub use self::{
    cache::{
        PROFILE_VALIDITY_DURATION, ROOM_DATA_VALIDITY_DURATION, RemoteCache, RemoteRoomEntry,
        RemoteRoomState, RemoteUserEntry, RemoteUserState, UrlPreviewEntry, UrlPreviewState,
    },
    room::{RemoteRoom, RemoteRoomError},
    room_peek::{PeekedMessage, RoomPeekError},
    space_children::{SpaceChild, SpaceChildren, SpaceChildrenError},
    url_preview::{UrlPreview, UrlPreviewError, UrlPreviewImage, UrlPreviewSupport, url_host},
    user::{RemoteUserError, RemoteUserProfile},
};
