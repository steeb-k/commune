//! Common message dialogs.

use adw::prelude::*;
use gettextrs::gettext;

use crate::{
    i18n::gettext_f,
    ngettext_f,
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

/// The room member destructive actions that need to be confirmed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomMemberDestructiveAction {
    /// Ban the member.
    Ban,
    /// Kick the member.
    Kick,
    /// Remove the member's messages.
    ///
    /// The value is the number of events that will be redacted.
    RemoveMessages(usize),
}

/// Show a dialog to confirm the given "destructive" action on the given room
/// member.
///
/// Returns a tuple `(confirm, reason)`.
pub async fn confirm_room_member_destructive_action(
    member: &Member,
    action: RoomMemberDestructiveAction,
    transient_for: &gtk::Window,
) -> (bool, Option<String>) {
    let (heading, body, response) = match action {
        RoomMemberDestructiveAction::Ban => {
            // Translators: Do NOT translate the content between '{' and '}',
            // this is a variable name.
            let heading = gettext_f("Ban {user}?", &[("user", &member.display_name())]);
            let body = gettext_f(
                // Translators: Do NOT translate the content between '{' and '}',
                // this is a variable name.
                "Are you sure you want to ban {user_id}? They will not be able to join the room again until someone unbans them.",
                &[("user_id", member.user_id().as_str())]
            );
            let response = gettext("Ban");
            (heading, body, Some(response))
        }
        RoomMemberDestructiveAction::Kick => {
            let can_rejoin = member.room().anyone_can_join();

            match member.membership() {
                Membership::Invite => {
                    let heading = gettext_f(
                        // Translators: Do NOT translate the content between '{' and '}',
                        // this is a variable name.
                        "Revoke Invite for {user}?",
                        &[("user", &member.display_name())],
                    );
                    let body = if can_rejoin {
                        gettext_f(
                            // Translators: Do NOT translate the content between '{' and '}',
                            // this is a variable name.
                        "Are you sure you want to revoke the invite for {user_id}? They will still be able to join the room on their own.",
                        &[("user_id", member.user_id().as_str())]
                    )
                    } else {
                        gettext_f(
                            // Translators: Do NOT translate the content between '{' and '}',
                            // this is a variable name.
                        "Are you sure you want to revoke the invite for {user_id}? They will not be able to join the room again until someone reinvites them.",
                        &[("user_id", member.user_id().as_str())]
                    )
                    };
                    let response = gettext("Revoke Invite");
                    (heading, body, Some(response))
                }
                Membership::Knock => {
                    let heading = gettext_f(
                        // Translators: Do NOT translate the content between '{' and '}',
                        // this is a variable name.
                        "Deny Access to {user}?",
                        &[("user", &member.display_name())],
                    );
                    let body = gettext_f(
                        // Translators: Do NOT translate the content between '{' and '}',
                        // this is a variable name.
                        "Are you sure you want to deny access to {user_id}?",
                        &[("user_id", member.user_id().as_str())],
                    );
                    let response = gettext("Deny Access");
                    (heading, body, Some(response))
                }
                _ => {
                    // Translators: Do NOT translate the content between '{' and '}',
                    // this is a variable name.
                    let heading = gettext_f("Kick {user}?", &[("user", &member.display_name())]);
                    let body = if can_rejoin {
                        gettext_f(
                            // Translators: Do NOT translate the content between '{' and '}',
                            // this is a variable name.
                        "Are you sure you want to kick {user_id}? They will still be able to join the room again on their own.",
                        &[("user_id", member.user_id().as_str())]
                    )
                    } else {
                        gettext_f(
                            // Translators: Do NOT translate the content between '{' and '}',
                            // this is a variable name.
                        "Are you sure you want to kick {user_id}? They will not be able to join the room again until someone invites them.",
                        &[("user_id", member.user_id().as_str())]
                    )
                    };
                    let response = gettext("Kick");
                    (heading, body, Some(response))
                }
            }
        }
        RoomMemberDestructiveAction::RemoveMessages(count) => {
            let n = u32::try_from(count).unwrap_or(u32::MAX);
            if count > 0 {
                let heading = gettext_f(
                    // Translators: Do NOT translate the content between '{' and '}',
                    // this is a variable name.
                    "Remove Messages Sent by {user}?",
                    &[("user", &member.display_name())],
                );
                let body = ngettext_f(
                // Translators: Do NOT translate the content between '{' and '}',
                // this is a variable name.
                "This removes all the messages received from the homeserver. Are you sure you want to remove 1 message sent by {user_id}? This cannot be undone.",
                "This removes all the messages received from the homeserver. Are you sure you want to remove {n} messages sent by {user_id}? This cannot be undone.",
                n,
                &[("n", &n.to_string()),("user_id", member.user_id().as_str())]
            );
                let response = gettext("Remove");
                (heading, body, Some(response))
            } else {
                let heading = gettext_f(
                    // Translators: Do NOT translate the content between '{' and '}',
                    // this is a variable name.
                    "No Messages Sent by {user}",
                    &[("user", &member.display_name())],
                );
                let body = gettext_f(
                // Translators: Do NOT translate the content between '{' and '}',
                // this is a variable name.
                "There are no messages received from the homeserver sent by {user_id}. You can try to load more by going further back in the room history.",
                &[("user_id", member.user_id().as_str())]
            );
                (heading, body, None)
            }
        }
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
    confirm_dialog.add_responses(&[("cancel", &gettext("Cancel"))]);

    if let Some(response) = response {
        confirm_dialog.add_responses(&[("confirm", &response)]);
        confirm_dialog.set_response_appearance("confirm", adw::ResponseAppearance::Destructive);
    }

    let confirmed = confirm_dialog.choose_future().await == "confirm";

    // Only get the reason when the user confirmed, and filter out if the reason is
    // empty.
    let reason = confirmed
        .then(|| reason_entry.text().trim().to_owned())
        .filter(|s| !s.is_empty());

    (confirmed, reason)
}
