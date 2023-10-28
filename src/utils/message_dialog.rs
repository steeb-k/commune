//! Common message dialogs.

use adw::prelude::*;
use gettextrs::gettext;

use crate::session::model::{Room, RoomType};

/// Show a dialog to confirm leaving a room.
///
/// This supports both leaving a joined room and rejecting an invite.
pub async fn confirm_leave_room(room: &Room, transient_for: &gtk::Window) -> bool {
    let (heading, body, response) = if room.category() == RoomType::Invited {
        // We are rejecting an invite.
        let heading = gettext("Decline Invite?");
        let body = if room.is_join_rule_public() {
            gettext("Do you really want to decline this invite? You can join this room on your own later.")
        } else {
            gettext(
                "Do you really want to decline this invite? You won’t be able to join this room without it.",
            )
        };
        let response = gettext("Decline");

        (heading, body, response)
    } else {
        // We are leaving a room that was joined.
        let heading = gettext("Leave Room?");
        let body = if room.is_join_rule_public() {
            gettext("Do you really want to leave this room? You can come back later.")
        } else {
            gettext(
                "Do you really want to leave this room? You won’t be able to come back without an invitation.",
            )
        };
        let response = gettext("Leave");

        (heading, body, response)
    };

    // Ask for confirmation.
    let confirm_dialog = adw::MessageDialog::builder()
        .transient_for(transient_for)
        .default_response("cancel")
        .heading(heading)
        .body(body)
        .build();
    confirm_dialog.add_responses(&[("cancel", &gettext("Cancel")), ("leave", &response)]);
    confirm_dialog.set_response_appearance("leave", adw::ResponseAppearance::Destructive);

    confirm_dialog.choose_future().await == "leave"
}
