//! Common message dialogs.

use adw::prelude::*;
use gettextrs::gettext;
use ruma::events::room::power_levels::PowerLevelAction;

use crate::{
    i18n::gettext_f,
    prelude::*,
    session::model::{Member, Membership, Room, RoomType},
};

/// Show a dialog to confirm leaving a room.
///
/// This supports both leaving a joined room and rejecting an invite.
pub async fn confirm_leave_room(room: &Room, transient_for: &gtk::Window) -> bool {
    let (heading, body, response) = if room.category() == RoomType::Invited {
        // We are rejecting an invite.
        let heading = gettext("Decline Invite?");
        let body = if room.can_join() {
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
        let body = if room.can_join() {
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

/// Show a dialog to confirm the given "destructive" action on the given room
/// member.
///
/// Returns a tuple `(confirm, reason)`.
///
/// Panics if the action is something else than `Kick` or `Ban`.
pub async fn confirm_room_member_destructive_action(
    room: &Room,
    member: &Member,
    action: PowerLevelAction,
    transient_for: &gtk::Window,
) -> (bool, Option<String>) {
    let (heading, body, response) = match action {
        PowerLevelAction::Ban => {
            let heading = gettext_f("Ban {user}?", &[("user", &member.display_name())]);
            let body = gettext_f(
                "Are you sure you want to ban {user_id}? They will not be able to join the room again until someone unbans them.",
                &[("user_id", member.user_id().as_str())]
            );
            let response = gettext("Ban");
            (heading, body, response)
        }
        PowerLevelAction::Kick => {
            let can_rejoin = room.anyone_can_join();

            match member.membership() {
                Membership::Invite => {
                    let heading = gettext_f(
                        "Revoke Invite for {user}?",
                        &[("user", &member.display_name())],
                    );
                    let body = if can_rejoin {
                        gettext_f(
                        "Are you sure you want to revoke the invite for {user_id}? They will still be able to join the room on their own.",
                        &[("user_id", member.user_id().as_str())]
                    )
                    } else {
                        gettext_f(
                        "Are you sure you want to revoke the invite for {user_id}? They will not be able to join the room again until someone reinvites them.",
                        &[("user_id", member.user_id().as_str())]
                    )
                    };
                    let response = gettext("Revoke Invite");
                    (heading, body, response)
                }
                Membership::Knock => {
                    let heading = gettext_f(
                        "Deny Access to {user}?",
                        &[("user", &member.display_name())],
                    );
                    let body = gettext_f(
                        "Are you sure you want to deny access to {user_id}?",
                        &[("user_id", member.user_id().as_str())],
                    );
                    let response = gettext("Deny Access");
                    (heading, body, response)
                }
                _ => {
                    let heading = gettext_f("Kick {user}?", &[("user", &member.display_name())]);
                    let body = if can_rejoin {
                        gettext_f(
                        "Are you sure you want to kick {user_id}? They will still be able to join the room again on their own.",
                        &[("user_id", member.user_id().as_str())]
                    )
                    } else {
                        gettext_f(
                        "Are you sure you want to kick {user_id}? They will not be able to join the room again until someone invites them.",
                        &[("user_id", member.user_id().as_str())]
                    )
                    };
                    let response = gettext("Kick");
                    (heading, body, response)
                }
            }
        }
        _ => unimplemented!(),
    };

    // Add an entry for the optional reason.
    let reason_entry = adw::EntryRow::builder()
        .title(gettext("Reason (optional)"))
        .build();
    let list_box = gtk::ListBox::builder()
        .css_classes(["boxed-list"])
        .margin_top(6)
        .build();
    list_box.append(&reason_entry);

    // Ask for confirmation.
    let confirm_dialog = adw::MessageDialog::builder()
        .transient_for(transient_for)
        .default_response("cancel")
        .heading(heading)
        .body(body)
        .extra_child(&list_box)
        .build();
    confirm_dialog.add_responses(&[("cancel", &gettext("Cancel")), ("confirm", &response)]);
    confirm_dialog.set_response_appearance("confirm", adw::ResponseAppearance::Destructive);

    let confirmed = confirm_dialog.choose_future().await == "confirm";

    // Only get the reason when the user confirmed, and filter out if the reason is
    // empty.
    let reason = confirmed
        .then(|| reason_entry.text().trim().to_owned())
        .filter(|s| !s.is_empty());

    (confirmed, reason)
}
