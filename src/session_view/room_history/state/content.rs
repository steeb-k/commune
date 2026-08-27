use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{glib, glib::clone, pango};
use matrix_sdk_ui::timeline::{
    AnyOtherStateEventContentChange, MemberProfileChange, MembershipChange, OtherState,
    RoomMembershipChange, RoomPinnedEventsChange, TimelineItemContent,
};
use ruma::{
    UserId,
    events::{
        StateEventContentChange, StateEventType,
        policy::rule::{PolicyRuleEventContent, Recommendation},
        room::{
            member::MembershipState, policy::RoomPolicyEventContent,
            server_acl::RoomServerAclEventContent,
        },
    },
};
use tracing::warn;

use super::StateCreation;
use crate::{
    gettext_f,
    prelude::*,
    session::{Event, Member},
    utils::BoundObjectWeakRef,
};

mod imp {
    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::StateContent)]
    pub struct StateContent {
        /// The state event displayed by this widget.
        #[property(get, set = Self::set_event, nullable)]
        event: glib::WeakRef<Event>,
        /// The sender of the event.
        sender: BoundObjectWeakRef<Member>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for StateContent {
        const NAME: &'static str = "ContentStateContent";
        type Type = super::StateContent;
        type ParentType = adw::Bin;
    }

    #[glib::derived_properties]
    impl ObjectImpl for StateContent {}

    impl WidgetImpl for StateContent {}
    impl BinImpl for StateContent {}

    impl StateContent {
        /// Set the event presented by this row.
        fn set_event(&self, event: Option<&Event>) {
            let Some(event) = event else {
                // Only handle when an event is set.
                return;
            };

            let sender = event.sender();
            let disambiguated_name_handler = sender.connect_disambiguated_name_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.update_content();
                }
            ));
            self.sender.set(&sender, vec![disambiguated_name_handler]);

            self.event.set(Some(event));
            self.update_content();
        }

        /// Update the content for the current state.
        fn update_content(&self) {
            let Some(event) = self.event.upgrade() else {
                return;
            };
            let Some(sender) = self.sender.obj() else {
                return;
            };

            match event.content() {
                TimelineItemContent::MembershipChange(membership_change) => {
                    self.update_with_membership_change(&membership_change, &sender);
                }
                TimelineItemContent::ProfileChange(profile_change) => {
                    self.update_with_profile_change(&profile_change, &sender);
                }
                TimelineItemContent::OtherState(other_state) => {
                    self.update_with_other_state(&other_state, &sender, &event);
                }
                _ => unreachable!(),
            }
        }

        /// Update this row with the given [`OtherState`].
        fn update_with_other_state(
            &self,
            other_state: &OtherState,
            sender: &Member,
            event: &Event,
        ) {
            let widget = match other_state.content() {
                AnyOtherStateEventContentChange::RoomCreate(content) => {
                    WidgetType::Creation(StateCreation::new(content))
                }
                AnyOtherStateEventContentChange::RoomEncryption(_) => {
                    WidgetType::Text(gettext("This room is encrypted from this point on."))
                }
                AnyOtherStateEventContentChange::RoomThirdPartyInvite(content) => {
                    let display_name = match content {
                        StateEventContentChange::Original { content, .. } => {
                            match &content.display_name {
                                s if s.is_empty() => other_state.state_key(),
                                s => s,
                            }
                        }
                        StateEventContentChange::Redacted(_) => other_state.state_key(),
                    };
                    WidgetType::Text(gettext_f(
                        // Translators: Do NOT translate the content between '{' and '}', this is a
                        // variable name.
                        "{sender} invited {user}.",
                        &[
                            ("sender", &sender.disambiguated_name()),
                            ("user", display_name),
                        ],
                    ))
                }
                AnyOtherStateEventContentChange::RoomServerAcl(content) => {
                    WidgetType::Text(server_acl_message(content, &sender.disambiguated_name()))
                }
                AnyOtherStateEventContentChange::RoomPinnedEvents(content) => {
                    // The SDK reduces the two lists to which way they differ, which
                    // is all a sentence can carry: how many were pinned, and which
                    // ones, is what the pinned messages view is for.
                    let message = match RoomPinnedEventsChange::from(content) {
                        RoomPinnedEventsChange::Added => gettext_f(
                            // Translators: Do NOT translate the content between '{' and '}',
                            // this is a variable name.
                            "{user} pinned a message.",
                            &[("user", &sender.disambiguated_name())],
                        ),
                        RoomPinnedEventsChange::Removed => gettext_f(
                            // Translators: Do NOT translate the content between '{' and '}',
                            // this is a variable name.
                            "{user} unpinned a message.",
                            &[("user", &sender.disambiguated_name())],
                        ),
                        RoomPinnedEventsChange::Changed => gettext_f(
                            // Translators: Do NOT translate the content between '{' and '}',
                            // this is a variable name.
                            "{user} changed the pinned messages.",
                            &[("user", &sender.disambiguated_name())],
                        ),
                    };
                    WidgetType::Text(message)
                }
                // The SDK's state-change enum does not carry `m.room.policy`,
                // so the event arrives as its custom variant and the server
                // name is read from the raw event instead.
                _ if other_state.content().event_type() == StateEventType::RoomPolicy => {
                    WidgetType::Text(policy_server_message(event, &sender.disambiguated_name()))
                }
                AnyOtherStateEventContentChange::PolicyRuleUser(change) => {
                    let rule = match change {
                        StateEventContentChange::Original { content, .. } => Some(&content.0),
                        StateEventContentChange::Redacted(_) => None,
                    };
                    WidgetType::Text(policy_rule_message(
                        rule,
                        &sender.disambiguated_name(),
                        PolicyRuleScope::User,
                    ))
                }
                AnyOtherStateEventContentChange::PolicyRuleRoom(change) => {
                    let rule = match change {
                        StateEventContentChange::Original { content, .. } => Some(&content.0),
                        StateEventContentChange::Redacted(_) => None,
                    };
                    WidgetType::Text(policy_rule_message(
                        rule,
                        &sender.disambiguated_name(),
                        PolicyRuleScope::Room,
                    ))
                }
                AnyOtherStateEventContentChange::PolicyRuleServer(change) => {
                    let rule = match change {
                        StateEventContentChange::Original { content, .. } => Some(&content.0),
                        StateEventContentChange::Redacted(_) => None,
                    };
                    WidgetType::Text(policy_rule_message(
                        rule,
                        &sender.disambiguated_name(),
                        PolicyRuleScope::Server,
                    ))
                }
                _ => {
                    warn!(
                        "Unsupported state event: {}",
                        other_state.content().event_type()
                    );
                    WidgetType::Text(gettext("An unsupported state event was received."))
                }
            };

            let obj = self.obj();
            match widget {
                WidgetType::Text(message) => {
                    let child = obj.child_or_else::<gtk::Label>(text);
                    child.set_label(&message);
                }
                WidgetType::Creation(widget) => obj.set_child(Some(&widget)),
            }
        }

        /// Update this row for the given membership change.
        fn update_with_membership_change(
            &self,
            membership_change: &RoomMembershipChange,
            sender: &Member,
        ) {
            let sender_display_name = sender.disambiguated_name();
            let target_display_name = match membership_change.content() {
                StateEventContentChange::Original { content, .. } => content
                    .displayname
                    .clone()
                    .unwrap_or_else(|| membership_change.user_id().to_string()),
                StateEventContentChange::Redacted(_) => membership_change.user_id().to_string(),
            };

            let supported_membership_change =
                Self::to_supported_membership_change(membership_change, sender.user_id());

            let message = match supported_membership_change {
                MembershipChange::Joined => {
                    // Translators: Do NOT translate the content between '{' and '}', this
                    // is a variable name.
                    gettext_f(
                        "{user} joined this room.",
                        &[("user", &target_display_name)],
                    )
                }
                MembershipChange::Left => {
                    // Translators: Do NOT translate the content between '{' and '}',
                    // this is a variable name.
                    gettext_f("{user} left the room.", &[("user", &target_display_name)])
                }
                MembershipChange::Banned | MembershipChange::KickedAndBanned => gettext_f(
                    // Translators: Do NOT translate the content between
                    // '{' and '}', these are variable names.
                    "{sender} banned {user}.",
                    &[
                        ("sender", &sender_display_name),
                        ("user", &target_display_name),
                    ],
                ),
                MembershipChange::Unbanned => gettext_f(
                    // Translators: Do NOT translate the content between
                    // '{' and '}', these are variable names.
                    "{sender} unbanned {user}.",
                    &[
                        ("sender", &sender_display_name),
                        ("user", &target_display_name),
                    ],
                ),
                MembershipChange::Kicked => gettext_f(
                    // Translators: Do NOT translate the content between '{' and
                    // '}', these are variable names.
                    "{sender} kicked {user} out.",
                    &[
                        ("sender", &sender_display_name),
                        ("user", &target_display_name),
                    ],
                ),
                MembershipChange::Invited | MembershipChange::KnockAccepted => gettext_f(
                    // Translators: Do NOT translate the content between '{' and '}', these are
                    // variable names.
                    "{sender} invited {user}.",
                    &[
                        ("sender", &sender_display_name),
                        ("user", &target_display_name),
                    ],
                ),
                MembershipChange::InvitationAccepted => gettext_f(
                    // Translators: Do NOT translate the content between
                    // '{' and '}', this is a variable name.
                    "{user} accepted the invite.",
                    &[("user", &target_display_name)],
                ),
                MembershipChange::InvitationRejected => gettext_f(
                    // Translators: Do NOT translate the content between
                    // '{' and '}', these are variable names.
                    "{user} declined the invite.",
                    &[
                        ("sender", &sender_display_name),
                        ("user", &target_display_name),
                    ],
                ),
                MembershipChange::InvitationRevoked => gettext_f(
                    // Translators: Do NOT translate the content between
                    // '{' and '}', these are variable names.
                    "{sender} revoked the invitation for {user}.",
                    &[
                        ("sender", &sender_display_name),
                        ("user", &target_display_name),
                    ],
                ),
                MembershipChange::Knocked =>
                // TODO: Add button to invite the user.
                {
                    gettext_f(
                        // Translators: Do NOT translate the content between '{' and '}', this
                        // is a variable name.
                        "{user} requested to be invited to this room.",
                        &[("user", &target_display_name)],
                    )
                }
                MembershipChange::KnockRetracted => gettext_f(
                    // Translators: Do NOT translate the content between
                    // '{' and '}', this is a variable name.
                    "{user} retracted their request to be invited to this room.",
                    &[("user", &target_display_name)],
                ),
                MembershipChange::KnockDenied => gettext_f(
                    // Translators: Do NOT translate the content between
                    // '{' and '}', these are variable names.
                    "{sender} denied {user}’s request to be invited to this room.",
                    &[
                        ("sender", &sender_display_name),
                        ("user", &target_display_name),
                    ],
                ),
                _ => {
                    warn!(
                        "Unsupported membership change event: {:?}",
                        membership_change.content()
                    );
                    gettext("An unsupported room member event was received.")
                }
            };

            let child = self.obj().child_or_else::<gtk::Label>(text);
            child.set_label(&message);
        }

        /// Convert a received membership change to a supported membership
        /// change.
        ///
        /// This is used to fallback to showing the membership when we do not
        /// know or do not want to show the change.
        fn to_supported_membership_change(
            membership_change: &RoomMembershipChange,
            sender: &UserId,
        ) -> MembershipChange {
            match membership_change.change().unwrap_or(MembershipChange::None) {
                MembershipChange::Joined => MembershipChange::Joined,
                MembershipChange::Left => MembershipChange::Left,
                MembershipChange::Banned => MembershipChange::Banned,
                MembershipChange::Unbanned => MembershipChange::Unbanned,
                MembershipChange::Kicked => MembershipChange::Kicked,
                MembershipChange::Invited => MembershipChange::Invited,
                MembershipChange::KickedAndBanned => MembershipChange::KickedAndBanned,
                MembershipChange::InvitationAccepted => MembershipChange::InvitationAccepted,
                MembershipChange::InvitationRejected => MembershipChange::InvitationRejected,
                MembershipChange::InvitationRevoked => MembershipChange::InvitationRevoked,
                MembershipChange::Knocked => MembershipChange::Knocked,
                MembershipChange::KnockAccepted => MembershipChange::KnockAccepted,
                MembershipChange::KnockRetracted => MembershipChange::KnockRetracted,
                MembershipChange::KnockDenied => MembershipChange::KnockDenied,
                _ => {
                    let membership = match membership_change.content() {
                        StateEventContentChange::Original { content, .. } => &content.membership,
                        StateEventContentChange::Redacted(content) => &content.membership,
                    };

                    match membership {
                        MembershipState::Ban => MembershipChange::Banned,
                        MembershipState::Invite => MembershipChange::Invited,
                        MembershipState::Join => MembershipChange::Joined,
                        MembershipState::Knock => MembershipChange::Knocked,
                        MembershipState::Leave => {
                            if membership_change.user_id() == sender {
                                MembershipChange::Left
                            } else {
                                MembershipChange::Kicked
                            }
                        }
                        _ => MembershipChange::NotImplemented,
                    }
                }
            }
        }

        fn update_with_profile_change(
            &self,
            profile_change: &MemberProfileChange,
            sender: &Member,
        ) {
            let message = if let Some(displayname) = profile_change.displayname_change() {
                if let Some(prev_name) = &displayname.old {
                    if let Some(new_name) = &displayname.new {
                        gettext_f(
                            // Translators: Do NOT translate the content between
                            // '{' and '}', this is a variable name.
                            "{previous_user_name} changed their display name to {new_user_name}.",
                            &[
                                ("previous_user_name", prev_name),
                                ("new_user_name", new_name),
                            ],
                        )
                    } else {
                        gettext_f(
                            // Translators: Do NOT translate the content between
                            // '{' and '}', this is a variable name.
                            "{previous_user_name} removed their display name.",
                            &[("previous_user_name", prev_name)],
                        )
                    }
                } else {
                    let new_name = displayname
                        .new
                        .as_ref()
                        .expect("At least one displayname is set in a display name change");
                    gettext_f(
                        // Translators: Do NOT translate the content between
                        // '{' and '}', this is a variable name.
                        "{user_id} set their display name to {new_user_name}.",
                        &[
                            ("user_id", profile_change.user_id().as_ref()),
                            ("new_user_name", new_name),
                        ],
                    )
                }
            } else if let Some(avatar_url) = profile_change.avatar_url_change() {
                let display_name = sender.disambiguated_name();

                if avatar_url.old.is_none() {
                    gettext_f(
                        // Translators: Do NOT translate the content between
                        // '{' and '}', this is a variable name.
                        "{user} set their avatar.",
                        &[("user", &display_name)],
                    )
                } else if avatar_url.new.is_none() {
                    gettext_f(
                        // Translators: Do NOT translate the content between
                        // '{' and '}', this is a variable name.
                        "{user} removed their avatar.",
                        &[("user", &display_name)],
                    )
                } else {
                    gettext_f(
                        // Translators: Do NOT translate the content between
                        // '{' and '}', this is a variable name.
                        "{user} changed their avatar.",
                        &[("user", &display_name)],
                    )
                }
            } else {
                // We don't know what changed so fall back to the membership.
                // Translators: Do NOT translate the content between '{' and '}', this
                // is a variable name.
                gettext_f(
                    "{user} joined this room.",
                    &[("user", &sender.disambiguated_name())],
                )
            };

            let child = self.obj().child_or_else::<gtk::Label>(text);
            child.set_label(&message);
        }
    }
}

glib::wrapper! {
    /// A row presenting a state event.
    pub struct StateContent(ObjectSubclass<imp::StateContent>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl StateContent {
    pub fn new() -> Self {
        glib::Object::new()
    }
}

impl Default for StateContent {
    fn default() -> Self {
        Self::new()
    }
}

impl IsABin for StateContent {}

enum WidgetType {
    Text(String),
    Creation(StateCreation),
}

/// The message describing the given change of the server access list.
///
/// Only a change of the blocked servers is spelled out: that is the common
/// moderation action, and everything else is a rule change that does not fit
/// in one line.
fn server_acl_message(
    content: &StateEventContentChange<RoomServerAclEventContent>,
    sender_name: &str,
) -> String {
    // Translators: Do NOT translate the content between '{' and '}', this is a
    // variable name.
    let generic = || {
        gettext_f(
            "{sender} changed which servers can take part in this room.",
            &[("sender", sender_name)],
        )
    };

    let StateEventContentChange::Original {
        content,
        prev_content,
    } = content
    else {
        // A redacted event no longer holds the list it set.
        return generic();
    };

    let Some(prev_content) = prev_content else {
        return gettext_f(
            // Translators: Do NOT translate the content between '{' and '}', this is a
            // variable name.
            "{sender} restricted which servers can take part in this room.",
            &[("sender", sender_name)],
        );
    };

    if content.allow != prev_content.allow
        || content.allow_ip_literals != prev_content.allow_ip_literals
    {
        return generic();
    }

    let blocked = servers_added(&prev_content.deny, &content.deny);
    let unblocked = servers_added(&content.deny, &prev_content.deny);

    match (blocked.is_empty(), unblocked.is_empty()) {
        (false, true) => gettext_f(
            // Translators: Do NOT translate the content between '{' and '}', these are
            // variable names.
            "{sender} blocked {servers} from taking part in this room.",
            &[("sender", sender_name), ("servers", &blocked.join(", "))],
        ),
        (true, false) => gettext_f(
            // Translators: Do NOT translate the content between '{' and '}', these are
            // variable names.
            "{sender} let {servers} take part in this room again.",
            &[("sender", sender_name), ("servers", &unblocked.join(", "))],
        ),
        _ => generic(),
    }
}

/// What a moderation policy rule is about.
#[derive(Debug, Clone, Copy)]
enum PolicyRuleScope {
    /// A rule about users.
    User,
    /// A rule about rooms.
    Room,
    /// A rule about servers.
    Server,
}

/// The sentence for a moderation policy rule event.
///
/// A rule the sender wrote reads with its entity, its recommendation and
/// its reason; a redacted one — which is how a rule is withdrawn — reads
/// as the removal. The only recommendation the specification defines is
/// `m.ban`; anything else is named a rule without claiming to know what
/// it asks for.
fn policy_rule_message(
    rule: Option<&PolicyRuleEventContent>,
    sender_name: &str,
    scope: PolicyRuleScope,
) -> String {
    let Some(rule) = rule else {
        return match scope {
            PolicyRuleScope::User => gettext_f(
                // Translators: Do NOT translate the content between '{' and '}', this is a
                // variable name.
                "{sender} removed a moderation rule about users.",
                &[("sender", sender_name)],
            ),
            PolicyRuleScope::Room => gettext_f(
                // Translators: Do NOT translate the content between '{' and '}', this is a
                // variable name.
                "{sender} removed a moderation rule about rooms.",
                &[("sender", sender_name)],
            ),
            PolicyRuleScope::Server => gettext_f(
                // Translators: Do NOT translate the content between '{' and '}', this is a
                // variable name.
                "{sender} removed a moderation rule about servers.",
                &[("sender", sender_name)],
            ),
        };
    };

    let vars: &[(&str, &str)] = &[
        ("sender", sender_name),
        ("entity", &rule.entity),
        ("reason", &rule.reason),
    ];

    if rule.recommendation == Recommendation::Ban {
        match scope {
            PolicyRuleScope::User => gettext_f(
                // Translators: Do NOT translate the content between '{' and '}', these are
                // variable names. The entity can use glob characters, like @spammer*:example.org.
                "{sender} recommended banning the users matching {entity}: {reason}",
                vars,
            ),
            PolicyRuleScope::Room => gettext_f(
                // Translators: Do NOT translate the content between '{' and '}', these are
                // variable names. The entity can use glob characters.
                "{sender} recommended banning the rooms matching {entity}: {reason}",
                vars,
            ),
            PolicyRuleScope::Server => gettext_f(
                // Translators: Do NOT translate the content between '{' and '}', these are
                // variable names. The entity can use glob characters.
                "{sender} recommended banning the servers matching {entity}: {reason}",
                vars,
            ),
        }
    } else {
        match scope {
            PolicyRuleScope::User => gettext_f(
                // Translators: Do NOT translate the content between '{' and '}', these are
                // variable names.
                "{sender} set a moderation rule for the users matching {entity}: {reason}",
                vars,
            ),
            PolicyRuleScope::Room => gettext_f(
                // Translators: Do NOT translate the content between '{' and '}', these are
                // variable names.
                "{sender} set a moderation rule for the rooms matching {entity}: {reason}",
                vars,
            ),
            PolicyRuleScope::Server => gettext_f(
                // Translators: Do NOT translate the content between '{' and '}', these are
                // variable names.
                "{sender} set a moderation rule for the servers matching {entity}: {reason}",
                vars,
            ),
        }
    }
}

/// The sentence for an `m.room.policy` event.
///
/// The SDK hands the event over as a custom state change, without its
/// content, so the server name is read from the raw event. An event whose
/// content does not parse — including one written empty to unset the policy
/// server, and a redacted one — reads as the policy server being removed,
/// which is what the specification says an invalid content means.
fn policy_server_message(event: &Event, sender_name: &str) -> String {
    let via = event.raw().and_then(|raw| {
        raw.get_field::<RoomPolicyEventContent>("content")
            .ok()
            .flatten()
            .map(|content| content.via)
    });

    if let Some(via) = via {
        gettext_f(
            // Translators: Do NOT translate the content between '{' and '}', these are
            // variable names. The server checks the messages of the room for spam.
            "{sender} made {server} check the messages of this room.",
            &[("sender", sender_name), ("server", via.as_str())],
        )
    } else {
        gettext_f(
            // Translators: Do NOT translate the content between '{' and '}', this is a
            // variable name.
            "{sender} stopped the checking of this room’s messages.",
            &[("sender", sender_name)],
        )
    }
}

/// The servers that are in `new` but not in `old`.
fn servers_added(old: &[String], new: &[String]) -> Vec<String> {
    new.iter().filter(|s| !old.contains(s)).cloned().collect()
}

/// Construct a `GtkLabel` for presenting a state content.
fn text() -> gtk::Label {
    gtk::Label::builder()
        .css_classes(["dimmed"])
        .wrap(true)
        .wrap_mode(pango::WrapMode::WordChar)
        .xalign(0.0)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a server ACL change from the given previous and new blocked
    /// servers, with the same allow list on both sides.
    fn deny_change(
        prev_deny: &[&str],
        deny: &[&str],
    ) -> StateEventContentChange<RoomServerAclEventContent> {
        let allow = vec!["*".to_owned()];
        StateEventContentChange::Original {
            content: RoomServerAclEventContent::new(
                true,
                allow.clone(),
                deny.iter().map(|s| (*s).to_owned()).collect(),
            ),
            prev_content: Some(RoomServerAclEventContent::new(
                true,
                allow,
                prev_deny.iter().map(|s| (*s).to_owned()).collect(),
            )),
        }
    }

    #[test]
    fn blocking_a_server_names_it() {
        let change = deny_change(&[], &["evil.example"]);

        assert_eq!(
            server_acl_message(&change, "Alice"),
            "Alice blocked evil.example from taking part in this room."
        );
    }

    #[test]
    fn unblocking_a_server_names_it() {
        let change = deny_change(&["evil.example"], &[]);

        assert_eq!(
            server_acl_message(&change, "Alice"),
            "Alice let evil.example take part in this room again."
        );
    }

    #[test]
    fn blocking_several_servers_names_them_all() {
        let change = deny_change(
            &["one.example"],
            &["one.example", "two.example", "three.example"],
        );

        assert_eq!(
            server_acl_message(&change, "Alice"),
            "Alice blocked two.example, three.example from taking part in this room."
        );
    }

    #[test]
    fn a_change_in_both_directions_stays_generic() {
        // Saying "blocked A and unblocked B" in one line is not worth the
        // translator's trouble, so it falls back.
        let change = deny_change(&["one.example"], &["two.example"]);

        assert_eq!(
            server_acl_message(&change, "Alice"),
            "Alice changed which servers can take part in this room."
        );
    }

    #[test]
    fn a_change_of_the_allow_list_stays_generic() {
        let change = StateEventContentChange::Original {
            content: RoomServerAclEventContent::new(
                true,
                vec!["*.example.org".to_owned()],
                Vec::new(),
            ),
            prev_content: Some(RoomServerAclEventContent::new(
                true,
                vec!["*".to_owned()],
                Vec::new(),
            )),
        };

        assert_eq!(
            server_acl_message(&change, "Alice"),
            "Alice changed which servers can take part in this room."
        );
    }

    #[test]
    fn the_ip_literal_switch_is_not_a_block() {
        // The blocked servers are untouched here, so the deny diff is empty and
        // the message must not claim a server was unblocked.
        let change = StateEventContentChange::Original {
            content: RoomServerAclEventContent::new(false, vec!["*".to_owned()], Vec::new()),
            prev_content: Some(RoomServerAclEventContent::new(
                true,
                vec!["*".to_owned()],
                Vec::new(),
            )),
        };

        assert_eq!(
            server_acl_message(&change, "Alice"),
            "Alice changed which servers can take part in this room."
        );
    }

    #[test]
    fn the_first_acl_of_a_room_is_a_restriction() {
        let change = StateEventContentChange::Original {
            content: RoomServerAclEventContent::new(
                true,
                vec!["*".to_owned()],
                vec!["evil.example".to_owned()],
            ),
            prev_content: None,
        };

        assert_eq!(
            server_acl_message(&change, "Alice"),
            "Alice restricted which servers can take part in this room."
        );
    }
}
