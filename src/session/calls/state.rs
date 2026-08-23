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

    /// Whether the call has not been answered yet, in either direction.
    pub(crate) fn is_pending(self) -> bool {
        matches!(self, Self::Ringing | Self::Dialing)
    }
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
