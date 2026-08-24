use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{glib, glib::clone};
use ruma::events::{
    StateEventType,
    room::{
        join_rules::{AllowRule, JoinRule as MatrixJoinRule, Restricted},
        power_levels::PowerLevelAction,
    },
};

use crate::{
    components::{
        CheckLoadingRow, LoadingButton, SpacePickerDialog, SpaceRequirement,
        UnsavedChangesResponse, unsaved_changes_dialog,
    },
    prelude::*,
    session::{JoinRuleValue, Room},
    toast,
};

mod imp {
    use std::cell::{Cell, RefCell};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session_view/room_details/join_rule_subpage.ui")]
    #[properties(wrapper_type = super::JoinRuleSubpage)]
    pub struct JoinRuleSubpage {
        #[template_child]
        save_button: TemplateChild<LoadingButton>,
        #[template_child]
        info_box: TemplateChild<gtk::Box>,
        #[template_child]
        info_image: TemplateChild<gtk::Image>,
        #[template_child]
        info_description: TemplateChild<gtk::Label>,
        #[template_child]
        membership_row: TemplateChild<CheckLoadingRow>,
        #[template_child]
        space_box: TemplateChild<gtk::ListBox>,
        #[template_child]
        space_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        space_row_arrow: TemplateChild<gtk::Image>,
        #[template_child]
        knock_box: TemplateChild<gtk::ListBox>,
        #[template_child]
        knock_row: TemplateChild<adw::SwitchRow>,
        /// The presented room.
        #[property(get, set = Self::set_room, explicit_notify, nullable)]
        room: glib::WeakRef<Room>,
        /// The local value of the join rule.
        #[property(get, set = Self::set_local_value, explicit_notify, builder(JoinRuleValue::default()))]
        local_value: Cell<JoinRuleValue>,
        /// Whether the join rule was changed by the user.
        #[property(get)]
        changed: Cell<bool>,
        /// The space the user picked, if they picked one.
        ///
        /// `None` means they have not touched it, and the room's saved allow
        /// list stands.
        picked_space: RefCell<Option<Room>>,
        permissions_handler: RefCell<Option<glib::SignalHandlerId>>,
        join_rule_handler: RefCell<Option<glib::SignalHandlerId>>,
        membership_room_handler: RefCell<Option<glib::SignalHandlerId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for JoinRuleSubpage {
        const NAME: &'static str = "RoomDetailsJoinRuleSubpage";
        type Type = super::JoinRuleSubpage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            CheckLoadingRow::ensure_type();

            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);

            klass.install_property_action("join-rule.set-value", "local-value");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for JoinRuleSubpage {
        fn dispose(&self) {
            self.disconnect_signals();
        }
    }

    impl WidgetImpl for JoinRuleSubpage {}
    impl NavigationPageImpl for JoinRuleSubpage {}

    #[gtk::template_callbacks]
    impl JoinRuleSubpage {
        /// Set the presented room.
        fn set_room(&self, room: Option<&Room>) {
            let Some(room) = room else {
                // Just ignore when room is missing.
                return;
            };

            self.disconnect_signals();

            let permissions_handler = room.permissions().connect_changed(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.update();
                }
            ));
            self.permissions_handler.replace(Some(permissions_handler));

            let join_rule_handler = room.join_rule().connect_changed(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.update();
                }
            ));
            self.join_rule_handler.replace(Some(join_rule_handler));

            // The name of the room a restricted rule points at is resolved
            // asynchronously, so the row's title has to follow it.
            let membership_room_handler = room.join_rule().connect_display_name_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.update_membership_row();
                }
            ));
            self.membership_room_handler
                .replace(Some(membership_room_handler));

            let authorization = &room.rules().authorization;
            let supports_knocking = authorization.knocking;
            let supports_restricted = authorization.restricted_join_rule;

            if !supports_knocking || !supports_restricted {
                self.info_description.set_label(&gettext("The version of this room does not support all possibilities. Upgrade this room to the latest version to see more options."));
                self.info_image.set_icon_name(Some("about-symbolic"));
            }

            self.info_box
                .set_visible(!supports_knocking || !supports_restricted);
            self.knock_box.set_visible(supports_knocking);
            // The restricted rule arrived in room version 8, so an older room
            // cannot be given one — and the picker must not offer to try.
            self.membership_row.set_visible(supports_restricted);

            self.picked_space.take();
            self.room.set(Some(room));

            self.update();
            self.obj().notify_room();
        }

        /// Update the subpage.
        fn update(&self) {
            let Some(room) = self.room.upgrade() else {
                return;
            };

            let join_rule = room.join_rule();
            self.set_local_value(join_rule.value());
            self.knock_row.set_active(join_rule.can_knock());

            self.picked_space.take();

            self.update_membership_row();
            self.update_space_row();
            self.update_knock_sensitive();

            self.save_button.set_is_loading(false);
            self.update_changed();
        }

        /// Update the row naming the space the room is restricted to.
        ///
        /// It follows the choice, not the saved rule: a space picked here has
        /// to be visible before it is saved, and the name of a space that only
        /// the saved rule knows about resolves asynchronously.
        fn update_membership_row(&self) {
            self.update_space_row();
        }

        /// The space the rule would allow the members of.
        ///
        /// This is the space the user picked, or — if they have not picked one
        /// — the one the room's saved rule already points at.
        fn membership_space_name(&self) -> Option<String> {
            if let Some(room) = self.picked_space.borrow().as_ref() {
                return Some(room.display_name());
            }

            self.room
                .upgrade()?
                .join_rule()
                .membership_room()
                .map(|room| room.display_name())
        }

        /// Update the row for choosing a space.
        fn update_space_row(&self) {
            let is_membership = self.local_value.get() == JoinRuleValue::RoomMembership;
            self.space_box
                .set_visible(is_membership && self.membership_row.is_visible());

            if !is_membership {
                return;
            }

            let subtitle = self.membership_space_name().unwrap_or_else(||
                // Translators: This is the subtitle of a row for choosing the
                // space whose members can join a room, when none is chosen.
                gettext("None chosen"));
            self.space_row.set_subtitle(&subtitle);

            // Without permission the row still says which space, and stops
            // pretending it can be changed.
            let can_change = self.can_change();
            self.space_row.set_activatable(can_change);
            self.space_row_arrow.set_visible(can_change);
        }

        /// Ask the user which space's members may join.
        #[template_callback]
        async fn choose_space(&self) {
            let Some(room) = self.room.upgrade() else {
                return;
            };
            let Some(session) = room.session() else {
                return;
            };

            // A room restricted to itself would admit nobody new, and a space
            // cannot be the answer to its own membership question.
            // Pointing a join rule at a space needs no power in it: the rule
            // is state in the room being restricted, not in the space.
            let Some(space) = SpacePickerDialog::choose(
                &*self.obj(),
                &session,
                Some(&room),
                SpaceRequirement::Joined,
            )
            .await
            else {
                return;
            };

            self.picked_space.replace(Some(space));

            self.update_space_row();
            self.update_knock_sensitive();
            self.update_changed();
        }

        /// Set the local value of the join rule.
        fn set_local_value(&self, value: JoinRuleValue) {
            if self.local_value.get() == value {
                return;
            }

            self.local_value.set(value);

            self.update_space_row();
            self.update_knock_sensitive();

            self.update_changed();
            self.obj().notify_local_value();
        }

        /// Update whether the knock switch can be toggled.
        ///
        /// It only applies to the rules that support it, and only when we are
        /// allowed to change the rule at all: otherwise it is a switch that
        /// moves and can never be saved.
        fn update_knock_sensitive(&self) {
            // `knock_restricted` arrived two room versions after `restricted`,
            // so a room can be old enough to take one and not the other.
            let room_supports_knock_restricted = self
                .room
                .upgrade()
                .is_some_and(|room| room.rules().authorization.knock_restricted_join_rule);

            let rule_supports_knocking = match self.local_value.get() {
                JoinRuleValue::Invite => true,
                JoinRuleValue::RoomMembership => room_supports_knock_restricted,
                _ => false,
            };

            self.knock_box
                .set_sensitive(rule_supports_knocking && self.can_change());
        }

        /// Whether we can change the join rule.
        fn can_change(&self) -> bool {
            let Some(room) = self.room.upgrade() else {
                return false;
            };

            if !room.join_rule().value().can_be_edited() {
                return false;
            }

            room.permissions()
                .is_allowed_to(PowerLevelAction::SendState(StateEventType::RoomJoinRules))
        }

        /// Whether users can request invites.
        fn can_knock(&self) -> bool {
            self.knock_box.is_visible()
                && self.knock_box.is_sensitive()
                && self.knock_row.is_active()
        }

        /// The rooms allowed by the current restricted rule of the room, if it
        /// has one.
        fn current_restricted(&self) -> Option<Restricted> {
            match self.room.upgrade()?.join_rule().matrix_join_rule()? {
                MatrixJoinRule::Restricted(restricted)
                | MatrixJoinRule::KnockRestricted(restricted) => Some(restricted),
                _ => None,
            }
        }

        /// The rooms the rule would allow, as this page holds them.
        ///
        /// A space picked here replaces the whole allow list. A list left
        /// alone is carried over verbatim, however many rooms are in it, so
        /// that changing whether people may knock never changes who may join.
        fn restricted(&self) -> Option<Restricted> {
            if let Some(space) = self.picked_space.borrow().as_ref() {
                return Some(Restricted::new(vec![AllowRule::room_membership(
                    space.room_id().to_owned(),
                )]));
            }

            self.current_restricted()
        }

        /// Compute the new join rule from the current state.
        fn new_join_rule(&self) -> Option<MatrixJoinRule> {
            compute_join_rule(self.local_value.get(), self.can_knock(), self.restricted())
        }

        /// Update whether the join rule was changed by the user.
        #[template_callback]
        fn update_changed(&self) {
            let Some(room) = self.room.upgrade() else {
                return;
            };

            let changed = if self.can_change() {
                let current_join_rule = room
                    .join_rule()
                    .matrix_join_rule()
                    .unwrap_or(MatrixJoinRule::Invite);

                // No rule at all means the page is in a state that cannot be
                // sent — the membership rule with no space chosen — which is
                // not a change, it is an unfinished one.
                self.new_join_rule()
                    .is_some_and(|new_join_rule| current_join_rule != new_join_rule)
            } else {
                false
            };

            self.changed.set(changed);
            self.obj().notify_changed();
        }

        /// Save the changes of this page.
        #[template_callback]
        async fn save(&self) {
            if !self.changed.get() {
                // Nothing to do.
                return;
            }

            let Some(room) = self.room.upgrade() else {
                return;
            };

            let Some(rule) = self.new_join_rule() else {
                // `changed` is false in this state, so this is unreachable.
                return;
            };

            self.save_button.set_is_loading(true);

            if room.join_rule().set_matrix_join_rule(rule).await.is_err() {
                toast!(self.obj(), gettext("Could not change who can join"));
                self.save_button.set_is_loading(false);
            }
        }

        /// Go back to the previous page in the room details.
        ///
        /// If there are changes in the page, ask the user to confirm.
        #[template_callback]
        async fn go_back(&self) {
            let obj = self.obj();
            let mut reset_after = false;

            if self.changed.get() {
                match unsaved_changes_dialog(&*obj).await {
                    UnsavedChangesResponse::Save => self.save().await,
                    UnsavedChangesResponse::Discard => reset_after = true,
                    UnsavedChangesResponse::Cancel => return,
                }
            }

            let _ = obj.activate_action("navigation.pop", None);

            if reset_after {
                self.update();
            }
        }

        /// Disconnect all the signal handlers.
        fn disconnect_signals(&self) {
            if let Some(room) = self.room.upgrade() {
                if let Some(handler) = self.permissions_handler.take() {
                    room.permissions().disconnect(handler);
                }

                if let Some(handler) = self.join_rule_handler.take() {
                    room.join_rule().disconnect(handler);
                }

                if let Some(handler) = self.membership_room_handler.take() {
                    room.join_rule().disconnect(handler);
                }
            }
        }
    }
}

/// Compute the join rule to send from the state of the subpage.
///
/// `restricted` is the allow list the page holds — the space that was picked,
/// or the room's saved list if none was.
///
/// Returns `None` when the page describes no rule that can be sent: the room
/// membership rule with nothing to allow. That is a half-finished choice, not
/// a rule, and it must never be turned into a restricted rule that admits
/// nobody — a state this page could not undo.
fn compute_join_rule(
    value: JoinRuleValue,
    can_knock: bool,
    restricted: Option<Restricted>,
) -> Option<MatrixJoinRule> {
    let rule = match value {
        JoinRuleValue::RoomMembership => {
            let restricted = restricted?;

            if can_knock {
                MatrixJoinRule::KnockRestricted(restricted)
            } else {
                MatrixJoinRule::Restricted(restricted)
            }
        }
        JoinRuleValue::Public => MatrixJoinRule::Public,
        // An unsupported rule cannot be selected, so it can only be the invite
        // rule here.
        _ => {
            if can_knock {
                MatrixJoinRule::Knock
            } else {
                MatrixJoinRule::Invite
            }
        }
    };

    Some(rule)
}

glib::wrapper! {
    /// Subpage to select the join rule of a room.
    pub struct JoinRuleSubpage(ObjectSubclass<imp::JoinRuleSubpage>)
        @extends gtk::Widget, adw::NavigationPage,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl JoinRuleSubpage {
    /// Construct a new `JoinRuleSubpage` for the given room.
    pub fn new(room: &Room) -> Self {
        glib::Object::builder().property("room", room).build()
    }
}

#[cfg(test)]
mod tests {
    use ruma::{
        events::room::join_rules::{AllowRule, JoinRule as MatrixJoinRule, Restricted},
        owned_room_id,
    };

    use super::compute_join_rule;
    use crate::session::JoinRuleValue;

    /// A restricted rule allowing the members of one space.
    fn space_membership() -> Restricted {
        Restricted::new(vec![AllowRule::room_membership(owned_room_id!(
            "!space:example.org"
        ))])
    }

    /// A restricted rule allowing the members of two spaces.
    fn two_space_membership() -> Restricted {
        Restricted::new(vec![
            AllowRule::room_membership(owned_room_id!("!space:example.org")),
            AllowRule::room_membership(owned_room_id!("!other:example.org")),
        ])
    }

    #[test]
    fn invite_rule_follows_the_knock_switch() {
        assert_eq!(
            compute_join_rule(JoinRuleValue::Invite, false, None),
            Some(MatrixJoinRule::Invite)
        );
        assert_eq!(
            compute_join_rule(JoinRuleValue::Invite, true, None),
            Some(MatrixJoinRule::Knock)
        );
    }

    #[test]
    fn public_rule_ignores_the_knock_switch() {
        assert_eq!(
            compute_join_rule(JoinRuleValue::Public, false, None),
            Some(MatrixJoinRule::Public)
        );
        // Knocking is meaningless on a room anyone can join, and the switch is
        // insensitive there, but the value must not leak into the rule.
        assert_eq!(
            compute_join_rule(JoinRuleValue::Public, true, None),
            Some(MatrixJoinRule::Public)
        );
    }

    #[test]
    fn room_membership_rule_keeps_the_allow_list() {
        let restricted = space_membership();

        assert_eq!(
            compute_join_rule(
                JoinRuleValue::RoomMembership,
                false,
                Some(restricted.clone())
            ),
            Some(MatrixJoinRule::Restricted(restricted.clone()))
        );
        assert_eq!(
            compute_join_rule(
                JoinRuleValue::RoomMembership,
                true,
                Some(restricted.clone())
            ),
            Some(MatrixJoinRule::KnockRestricted(restricted))
        );
    }

    #[test]
    fn an_allow_list_of_several_spaces_is_carried_over_whole() {
        // The picker replaces the whole list, but a list left alone must
        // survive a change to the knock switch intact — dropping a space
        // would quietly lock people out.
        let restricted = two_space_membership();

        assert_eq!(
            compute_join_rule(
                JoinRuleValue::RoomMembership,
                true,
                Some(restricted.clone())
            ),
            Some(MatrixJoinRule::KnockRestricted(restricted))
        );
    }

    #[test]
    fn turning_knocking_on_and_off_is_lossless() {
        let restricted = space_membership();

        // The round trip a user makes when they flip the switch twice must give
        // back the rule they started from, allow list and all.
        let with_knocking = compute_join_rule(
            JoinRuleValue::RoomMembership,
            true,
            Some(restricted.clone()),
        );
        let Some(MatrixJoinRule::KnockRestricted(carried_over)) = with_knocking else {
            panic!("expected a knock restricted rule");
        };

        assert_eq!(
            compute_join_rule(JoinRuleValue::RoomMembership, false, Some(carried_over)),
            Some(MatrixJoinRule::Restricted(restricted))
        );
    }

    #[test]
    fn room_membership_without_an_allow_list_is_not_a_rule() {
        // The page reaches this state now: the rule is selected and no space
        // has been chosen yet. It must produce nothing at all — neither a
        // restricted rule that admits nobody, which this page could not undo,
        // nor a silent fall back to a different rule than the one on screen.
        assert_eq!(
            compute_join_rule(JoinRuleValue::RoomMembership, false, None),
            None
        );
        assert_eq!(
            compute_join_rule(JoinRuleValue::RoomMembership, true, None),
            None
        );
    }

    #[test]
    fn unsupported_rule_is_never_sent() {
        assert_eq!(
            compute_join_rule(JoinRuleValue::Unsupported, false, None),
            Some(MatrixJoinRule::Invite)
        );
    }
}
