use gtk::glib;

/// Where a call has got to.
#[derive(Debug, Default, Hash, Eq, PartialEq, Clone, Copy, glib::Enum)]
#[enum_type(name = "CallState")]
pub enum CallState {
    /// Somebody is calling us and we have not answered.
    #[default]
    Ringing,
    /// We are calling somebody and they have not answered.
    Dialing,
    /// The offer and the answer have been exchanged, and the two ends are
    /// trying to find a path to each other.
    Connecting,
    /// Media is flowing.
    Connected,
    /// The call is over.
    Ended,
}

impl CallState {
    /// Whether the call is over.
    pub(crate) fn is_ended(self) -> bool {
        self == Self::Ended
    }
}

/// What became of a call, as far as the room can tell.
///
/// Not [`CallEndReason`], which is about a call this client was in and is
/// phrased for the person who was in it. This is about any call the room has
/// seen — including one that rang on another device — and it is what the row
/// in the timeline says afterwards.
#[derive(Debug, Default, Hash, Eq, PartialEq, Clone, Copy, glib::Enum)]
#[enum_type(name = "CallOutcome")]
pub enum CallOutcome {
    /// An invite was seen and nothing has happened to it yet.
    #[default]
    Ringing,
    /// Somebody answered it.
    Answered,
    /// Somebody said no to it.
    Declined,
    /// It stopped ringing without being answered.
    Missed,
}

/// Why a call ended.
///
/// This is what the interface says, which is not the same thing as the `reason`
/// of an `m.call.hangup`: several of the spec's reasons are the same sentence
/// to the person reading it, and one of ours — being answered elsewhere — is
/// not a hangup reason at all.
#[derive(Debug, Default, Hash, Eq, PartialEq, Clone, Copy, glib::Enum)]
#[enum_type(name = "CallEndReason")]
pub enum CallEndReason {
    /// One of the two people ended it.
    #[default]
    HungUp,
    /// The other party declined.
    Declined,
    /// Nobody answered before the invite expired.
    NotAnswered,
    /// Another of our own devices took the call.
    AnsweredElsewhere,
    /// The two ends could not find a path to each other.
    NoConnection,
    /// Something went wrong locally, usually a missing camera or microphone.
    MediaFailed,
    /// Something else went wrong.
    Failed,
}

impl From<commune_core::session::CallState> for CallState {
    fn from(value: commune_core::session::CallState) -> Self {
        use commune_core::session::CallState as Core;

        match value {
            Core::Ringing => Self::Ringing,
            Core::Dialing => Self::Dialing,
            Core::Connecting => Self::Connecting,
            Core::Connected => Self::Connected,
            Core::Ended => Self::Ended,
        }
    }
}

impl From<commune_core::session::CallOutcome> for CallOutcome {
    fn from(value: commune_core::session::CallOutcome) -> Self {
        use commune_core::session::CallOutcome as Core;

        match value {
            Core::Ringing => Self::Ringing,
            Core::Answered => Self::Answered,
            Core::Declined => Self::Declined,
            Core::Missed => Self::Missed,
        }
    }
}

impl From<commune_core::session::CallEndReason> for CallEndReason {
    fn from(value: commune_core::session::CallEndReason) -> Self {
        use commune_core::session::CallEndReason as Core;

        match value {
            Core::HungUp => Self::HungUp,
            Core::Declined => Self::Declined,
            Core::NotAnswered => Self::NotAnswered,
            Core::AnsweredElsewhere => Self::AnsweredElsewhere,
            Core::NoConnection => Self::NoConnection,
            Core::MediaFailed => Self::MediaFailed,
            Core::Failed => Self::Failed,
        }
    }
}
