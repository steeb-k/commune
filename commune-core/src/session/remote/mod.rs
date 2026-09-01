//! Rooms the session is not in, described by the homeserver on request.
//!
//! The headless counterpart of the application's `session/remote/`: a
//! remote room is a value the homeserver hands back — from the summary
//! endpoint, the public directory or a space's hierarchy — rather than a
//! room sync keeps current. What stayed in the application is the
//! `GObject` around it, the avatar image, the linkified topic and the
//! staleness timer that reloads a preview after a day.

mod room;
mod space_children;

pub use self::{
    room::{RemoteRoom, RemoteRoomError},
    space_children::{SpaceChild, SpaceChildren, SpaceChildrenError},
};
