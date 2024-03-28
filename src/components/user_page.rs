use adw::{prelude::*, subclass::prelude::*};
use gettextrs::{gettext, pgettext};
use gtk::{
    glib,
    glib::{clone, closure_local},
    CompositeTemplate,
};
use ruma::{events::room::power_levels::PowerLevelUserAction, OwnedEventId};

use super::{Avatar, ButtonRow, PowerLevelSelectionRow, SpinnerButton};
use crate::{
    i18n::gettext_f,
    ngettext_f,
    prelude::*,
    session::model::{Member, Membership, Permissions, Room, User},
    toast,
    utils::{
        message_dialog::{
            confirm_mute_room_member, confirm_room_member_destructive_action,
            confirm_set_room_member_power_level_same_as_own, RoomMemberDestructiveAction,
        },
        BoundObject,
    },
    Window,
};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::{InitializingObject, Signal};
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/user_page.ui")]
    #[properties(wrapper_type = super::UserPage)]
    pub struct UserPage {
        #[template_child]
        pub avatar: TemplateChild<Avatar>,
        #[template_child]
        pub direct_chat_button: TemplateChild<SpinnerButton>,
        #[template_child]
        pub verified_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub verified_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub verify_button: TemplateChild<SpinnerButton>,
        #[template_child]
        pub room_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub room_title: TemplateChild<gtk::Label>,
        #[template_child]
        pub membership_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub membership_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub power_level_row: TemplateChild<PowerLevelSelectionRow>,
        #[template_child]
        pub invite_button: TemplateChild<SpinnerButton>,
        #[template_child]
        pub kick_box: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub kick_button: TemplateChild<ButtonRow>,
        #[template_child]
        pub ban_box: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub ban_button: TemplateChild<ButtonRow>,
        #[template_child]
        pub unban_button: TemplateChild<SpinnerButton>,
        #[template_child]
        pub remove_messages_box: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub remove_messages_button: TemplateChild<ButtonRow>,
        #[template_child]
        pub ignored_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub ignored_button: TemplateChild<SpinnerButton>,
        /// The current user.
        #[property(get, set = Self::set_user, explicit_notify, nullable)]
        pub user: BoundObject<User>,
        pub bindings: RefCell<Vec<glib::Binding>>,
        pub permissions_handler: RefCell<Option<glib::SignalHandlerId>>,
        pub room_display_name_handler: RefCell<Option<glib::SignalHandlerId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for UserPage {
        const NAME: &'static str = "UserPage";
        type Type = super::UserPage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);

            klass.install_action_async(
                "user-page.open-direct-chat",
                None,
                |widget, _, _| async move {
                    widget.open_direct_chat().await;
                },
            );

            klass.install_action_async("user-page.verify-user", None, |widget, _, _| async move {
                widget.verify_user().await;
            });
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for UserPage {
        fn signals() -> &'static [Signal] {
            static SIGNALS: Lazy<Vec<Signal>> =
                Lazy::new(|| vec![Signal::builder("close").build()]);
            SIGNALS.as_ref()
        }

        fn dispose(&self) {
            for binding in self.bindings.take() {
                binding.unbind();
            }

            if let Some(member) = self.user.obj().and_downcast::<Member>() {
                let room = member.room();

                if let Some(handler) = self.permissions_handler.take() {
                    room.permissions().disconnect(handler);
                }
                if let Some(handler) = self.room_display_name_handler.take() {
                    room.disconnect(handler);
                }
            }
        }
    }

    impl WidgetImpl for UserPage {}
    impl NavigationPageImpl for UserPage {}

    impl UserPage {
        /// Set the current user.
        fn set_user(&self, user: Option<User>) {
            let prev_user = self.user.obj();

            if prev_user == user {
                return;
            }
            let obj = self.obj();

            if let Some(member) = prev_user.and_downcast::<Member>() {
                let room = member.room();

                if let Some(handler) = self.permissions_handler.take() {
                    room.permissions().disconnect(handler);
                }
                if let Some(handler) = self.room_display_name_handler.take() {
                    room.disconnect(handler);
                }
            }
            for binding in self.bindings.take() {
                binding.unbind();
            }
            self.user.disconnect_signals();
            self.power_level_row.set_permissions(None::<Permissions>);

            if let Some(user) = user {
                let title_binding = user
                    .bind_property("display-name", &*obj, "title")
                    .sync_create()
                    .build();
                let avatar_binding = user
                    .bind_property("avatar-data", &*self.avatar, "data")
                    .sync_create()
                    .build();
                let bindings = vec![title_binding, avatar_binding];

                let verified_handler = user.connect_verified_notify(clone!(@weak obj => move |_| {
                    obj.update_verified();
                }));
                let ignored_handler =
                    user.connect_is_ignored_notify(clone!(@weak obj => move |_| {
                        obj.update_direct_chat();
                        obj.update_ignored();
                    }));
                let mut handlers = vec![verified_handler, ignored_handler];

                if let Some(member) = user.downcast_ref::<Member>() {
                    let room = member.room();

                    let permissions = room.permissions();
                    let permissions_handler =
                        permissions.connect_changed(clone!(@weak obj => move |_| {
                            obj.update_room();
                        }));
                    self.permissions_handler.replace(Some(permissions_handler));
                    self.power_level_row.set_permissions(Some(permissions));

                    let room_display_name_handler =
                        room.connect_display_name_notify(clone!(@weak obj => move |_| {
                            obj.update_room();
                        }));
                    self.room_display_name_handler
                        .replace(Some(room_display_name_handler));

                    let membership_handler =
                        member.connect_membership_notify(clone!(@weak obj => move |member| {
                            if member.membership() == Membership::Leave {
                                obj.emit_by_name::<()>("close", &[]);
                            } else {
                                obj.update_room();
                            }
                        }));
                    let power_level_handler =
                        member.connect_power_level_notify(clone!(@weak obj => move |_| {
                            obj.update_room();
                        }));
                    handlers.extend([membership_handler, power_level_handler]);
                }

                // We don't need to listen to changes of the property, it never changes after
                // construction.
                let is_own_user = user.is_own_user();
                self.ignored_row.set_visible(!is_own_user);

                self.user.set(user, handlers);
                self.bindings.replace(bindings);
            }

            obj.load_direct_chat();
            obj.update_direct_chat();
            obj.update_room();
            obj.update_verified();
            obj.update_ignored();
            obj.notify_user();
        }
    }
}

glib::wrapper! {
    /// Page to view details about a user.
    pub struct UserPage(ObjectSubclass<imp::UserPage>)
        @extends gtk::Widget, adw::NavigationPage, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl UserPage {
    /// Construct a new `UserPage` for the given user.
    pub fn new(user: &impl IsA<User>) -> Self {
        glib::Object::builder().property("user", user).build()
    }

    /// Copy the user ID to the clipboard.
    #[template_callback]
    fn copy_user_id(&self) {
        let Some(user) = self.user() else {
            return;
        };

        self.clipboard().set_text(user.user_id().as_str());
        toast!(self, gettext("Matrix user ID copied to clipboard"));
    }

    /// Update the visibility of the direct chat button.
    fn update_direct_chat(&self) {
        let is_visible = self
            .user()
            .is_some_and(|u| !u.is_own_user() && !u.is_ignored());
        self.imp().direct_chat_button.set_visible(is_visible);
    }

    /// Load whether the current user has a direct chat or not.
    fn load_direct_chat(&self) {
        self.set_direct_chat_loading(true);

        let Some(user) = self.user() else {
            return;
        };

        let direct_chat = user.direct_chat();

        let label = if direct_chat.is_some() {
            gettext("Open Direct Chat")
        } else {
            gettext("Create Direct Chat")
        };
        self.imp().direct_chat_button.set_content_label(label);

        self.set_direct_chat_loading(false);
    }

    /// Set whether the direct chat button is loading.
    fn set_direct_chat_loading(&self, loading: bool) {
        self.action_set_enabled("user-page.open-direct-chat", !loading);
        self.imp().direct_chat_button.set_loading(loading);
    }

    /// Open a direct chat with the current user.
    ///
    /// If one doesn't exist already, it is created.
    async fn open_direct_chat(&self) {
        let Some(user) = self.user() else {
            return;
        };

        self.set_direct_chat_loading(true);

        let Ok(room) = user.get_or_create_direct_chat().await else {
            toast!(self, &gettext("Could not create a new Direct Chat"));
            self.set_direct_chat_loading(false);

            return;
        };

        let Some(parent_window) = self.root().and_downcast::<gtk::Window>() else {
            return;
        };

        if let Some(main_window) = parent_window.transient_for().and_downcast::<Window>() {
            main_window.show_room(user.session().session_id(), room.room_id());
        }

        parent_window.close();
    }

    /// Update the room section.
    fn update_room(&self) {
        let imp = self.imp();

        let Some(member) = self.user().and_downcast::<Member>() else {
            imp.room_box.set_visible(false);
            return;
        };

        let membership = member.membership();
        if membership == Membership::Leave {
            imp.room_box.set_visible(false);
            return;
        }

        let room = member.room();
        let room_title = gettext_f("In {room_name}", &[("room_name", &room.display_name())]);
        imp.room_title.set_label(&room_title);

        match membership {
            Membership::Leave => unreachable!(),
            Membership::Join => {
                // Nothing to update, it should show the role row.
            }
            Membership::Invite => {
                imp.membership_label
                    // Translators: As in, 'The room member was invited'.
                    .set_label(&pgettext("member", "Invited"));
            }
            Membership::Ban => {
                imp.membership_label
                    // Translators: As in, 'The room member was banned'.
                    .set_label(&pgettext("member", "Banned"));
            }
            Membership::Knock => {
                imp.membership_label
                    // Translators: As in, 'The room member knocked to request access to the room'.
                    .set_label(&pgettext("member", "Knocked"));
            }
            Membership::Custom => {
                imp.membership_label
                    // Translators: As in, 'The room member has an unknown role'.
                    .set_label(&pgettext("member", "Unknown"));
            }
        }

        let is_role = membership == Membership::Join;
        imp.membership_row.set_visible(!is_role);
        imp.power_level_row.set_visible(is_role);

        let permissions = room.permissions();
        let user_id = member.user_id();

        imp.power_level_row.set_is_loading(false);
        imp.power_level_row
            .set_selected_power_level(member.power_level());

        let can_change_power_level = !member.is_own_user()
            && permissions.can_do_to_user(user_id, PowerLevelUserAction::ChangePowerLevel);
        imp.power_level_row.set_read_only(!can_change_power_level);

        let can_invite = matches!(membership, Membership::Knock) && permissions.can_invite();
        if can_invite {
            imp.invite_button.set_content_label(gettext("Allow Access"));
            imp.invite_button.set_visible(true);
        } else {
            imp.invite_button.set_visible(false);
        }

        let can_kick = matches!(
            membership,
            Membership::Join | Membership::Invite | Membership::Knock
        ) && permissions.can_do_to_user(user_id, PowerLevelUserAction::Kick);
        if can_kick {
            let label = match membership {
                Membership::Invite => gettext("Revoke Invite"),
                Membership::Knock => gettext("Deny Access"),
                // Translators: As in, 'Kick room member'.
                _ => gettext("Kick"),
            };
            imp.kick_button.set_title(&label);
            imp.kick_box.set_visible(true);
        } else {
            imp.kick_box.set_visible(false);
        }

        let can_ban = membership != Membership::Ban
            && permissions.can_do_to_user(user_id, PowerLevelUserAction::Ban);
        imp.ban_box.set_visible(can_ban);

        let can_unban = matches!(membership, Membership::Ban)
            && permissions.can_do_to_user(user_id, PowerLevelUserAction::Unban);
        imp.unban_button.set_visible(can_unban);

        let can_redact = !member.is_own_user() && permissions.can_redact_other();
        imp.remove_messages_box.set_visible(can_redact);

        imp.room_box.set_visible(true);
    }

    /// Reset the initial state of the buttons of the room section.
    fn reset_room(&self) {
        let imp = self.imp();

        imp.kick_button.set_is_loading(false);
        imp.kick_button.set_sensitive(true);

        imp.invite_button.set_loading(false);
        imp.invite_button.set_sensitive(true);

        imp.ban_button.set_is_loading(false);
        imp.ban_button.set_sensitive(true);

        imp.unban_button.set_loading(false);
        imp.unban_button.set_sensitive(true);

        imp.remove_messages_button.set_is_loading(false);
        imp.remove_messages_button.set_sensitive(true);
    }

    /// Set the power level of the user.
    #[template_callback]
    async fn set_power_level(&self) {
        let Some(member) = self.user().and_downcast::<Member>() else {
            return;
        };

        let row = &self.imp().power_level_row;
        let power_level = row.selected_power_level();
        let old_power_level = member.power_level();

        if old_power_level == power_level {
            // Nothing to do.
            return;
        }

        row.set_is_loading(true);
        row.set_read_only(true);

        let permissions = member.room().permissions();

        // Warn if user is muted but was not before.
        let mute_power_level = permissions.mute_power_level();
        let is_muted = power_level <= mute_power_level && old_power_level > mute_power_level;
        if is_muted && !confirm_mute_room_member(&member, self).await {
            self.update_room();
            return;
        }

        // Warn if power level is set at same level as own power level.
        let is_own_power_level = power_level == permissions.own_power_level();
        if is_own_power_level
            && !confirm_set_room_member_power_level_same_as_own(&member, self).await
        {
            self.update_room();
            return;
        }

        let user_id = member.user_id().clone();

        if permissions
            .set_user_power_level(user_id, power_level)
            .await
            .is_err()
        {
            toast!(self, gettext("Could not change the role"));
            self.update_room();
        }
    }

    /// Invite the user to the room.
    #[template_callback]
    async fn invite_user(&self) {
        let Some(member) = self.user().and_downcast::<Member>() else {
            return;
        };

        let imp = self.imp();
        imp.invite_button.set_loading(true);
        imp.kick_button.set_sensitive(false);
        imp.ban_button.set_sensitive(false);
        imp.unban_button.set_sensitive(false);

        let room = member.room();
        let user_id = member.user_id().clone();

        if room.invite(&[user_id]).await.is_err() {
            toast!(self, gettext("Could not invite user"));
        }

        self.reset_room();
    }

    /// Kick the user from the room.
    #[template_callback]
    async fn kick_user(&self) {
        let Some(member) = self.user().and_downcast::<Member>() else {
            return;
        };

        let imp = self.imp();
        imp.kick_button.set_is_loading(true);
        imp.invite_button.set_sensitive(false);
        imp.ban_button.set_sensitive(false);
        imp.unban_button.set_sensitive(false);

        let Some(response) = confirm_room_member_destructive_action(
            &member,
            RoomMemberDestructiveAction::Kick,
            self,
        )
        .await
        else {
            self.reset_room();
            return;
        };

        let room = member.room();
        let user_id = member.user_id().clone();
        if room.kick(&[(user_id, response.reason)]).await.is_err() {
            let error = match member.membership() {
                Membership::Invite => gettext("Could not revoke invite of user"),
                Membership::Knock => gettext("Could not deny access to user"),
                _ => gettext("Could not kick user"),
            };
            toast!(self, error);

            self.reset_room();
        }
    }

    /// Ban the room member.
    #[template_callback]
    async fn ban_user(&self) {
        let Some(member) = self.user().and_downcast::<Member>() else {
            return;
        };

        let imp = self.imp();
        imp.ban_button.set_is_loading(true);
        imp.invite_button.set_sensitive(false);
        imp.kick_button.set_sensitive(false);
        imp.unban_button.set_sensitive(false);

        let permissions = member.room().permissions();
        let redactable_events = if permissions.can_redact_other() {
            member.redactable_events()
        } else {
            vec![]
        };

        let Some(response) = confirm_room_member_destructive_action(
            &member,
            RoomMemberDestructiveAction::Ban(redactable_events.len()),
            self,
        )
        .await
        else {
            self.reset_room();
            return;
        };

        let room = member.room();
        let user_id = member.user_id().clone();
        if room
            .ban(&[(user_id, response.reason.clone())])
            .await
            .is_err()
        {
            toast!(self, gettext("Could not ban user"));
        }

        if response.remove_events {
            self.remove_known_messages_inner(&member.room(), redactable_events, response.reason)
                .await;
        }

        self.reset_room();
    }

    /// Unban the room member.
    #[template_callback]
    async fn unban_user(&self) {
        let Some(member) = self.user().and_downcast::<Member>() else {
            return;
        };

        let imp = self.imp();
        imp.unban_button.set_loading(true);
        imp.invite_button.set_sensitive(false);
        imp.kick_button.set_sensitive(false);
        imp.ban_button.set_sensitive(false);

        let room = member.room();
        let user_id = member.user_id().clone();

        if room.unban(&[(user_id, None)]).await.is_err() {
            toast!(self, gettext("Could not unban user"));
        }

        self.reset_room();
    }

    /// Remove the known events of the room member.
    #[template_callback]
    async fn remove_messages(&self) {
        let Some(member) = self.user().and_downcast::<Member>() else {
            return;
        };

        let imp = self.imp();
        imp.remove_messages_button.set_is_loading(true);

        let redactable_events = member.redactable_events();

        let Some(response) = confirm_room_member_destructive_action(
            &member,
            RoomMemberDestructiveAction::RemoveMessages(redactable_events.len()),
            self,
        )
        .await
        else {
            self.reset_room();
            return;
        };

        self.remove_known_messages_inner(&member.room(), redactable_events, response.reason)
            .await;

        self.reset_room();
    }

    async fn remove_known_messages_inner(
        &self,
        room: &Room,
        events: Vec<OwnedEventId>,
        reason: Option<String>,
    ) {
        if let Err(events) = room.redact(&events, reason).await {
            let n = u32::try_from(events.len()).unwrap_or(u32::MAX);
            toast!(
                self,
                ngettext_f(
                    // Translators: Do NOT translate the content between '{' and '}',
                    // this is a variable name.
                    "Could not remove 1 message sent by the user",
                    "Could not remove {n} messages sent by the user",
                    n,
                    &[("n", &n.to_string())]
                )
            );
        }
    }

    /// Update the verified row.
    fn update_verified(&self) {
        let Some(user) = self.user() else {
            return;
        };
        let imp = self.imp();

        if user.verified() {
            imp.verified_row.set_title(&gettext("Identity verified"));
            imp.verified_stack.set_visible_child_name("icon");
            self.action_set_enabled("user-page.verify-user", false);
        } else {
            self.action_set_enabled("user-page.verify-user", true);
            imp.verified_stack.set_visible_child_name("button");
            imp.verified_row
                .set_title(&gettext("Identity not verified"));
        }
    }

    /// Launch the verification for the current user.
    async fn verify_user(&self) {
        let Some(user) = self.user() else {
            return;
        };
        let imp = self.imp();

        self.action_set_enabled("user-page.verify-user", false);
        imp.verify_button.set_loading(true);
        let verification = match user.verify_identity().await {
            Ok(verification) => verification,
            Err(()) => {
                toast!(self, gettext("Could not start user verification"));
                self.action_set_enabled("user-page.verify-user", true);
                imp.verify_button.set_loading(false);
                return;
            }
        };

        let Some(parent_window) = self.root().and_downcast::<gtk::Window>() else {
            return;
        };

        if let Some(main_window) = parent_window.transient_for().and_downcast::<Window>() {
            main_window.show_verification(user.session().session_id(), verification);
        }

        parent_window.close();
    }

    /// Update the ignored row.
    fn update_ignored(&self) {
        let Some(user) = self.user() else {
            return;
        };
        let imp = self.imp();

        if user.is_ignored() {
            imp.ignored_row.set_title(&gettext("Ignored"));
            imp.ignored_button
                .set_content_label(gettext("Stop Ignoring"));
            imp.ignored_button.remove_css_class("destructive-action");
        } else {
            imp.ignored_row.set_title(&gettext("Not Ignored"));
            imp.ignored_button.set_content_label(gettext("Ignore"));
            imp.ignored_button.add_css_class("destructive-action");
        }
    }

    /// Toggle whether the user is ignored or not.
    #[template_callback]
    async fn toggle_ignored(&self) {
        let Some(user) = self.user() else {
            return;
        };

        let imp = self.imp();
        self.imp().ignored_button.set_loading(true);

        if user.is_ignored() {
            if user.stop_ignoring().await.is_err() {
                toast!(self, gettext("Could not stop ignoring user"));
            }
        } else if user.ignore().await.is_err() {
            toast!(self, gettext("Could not ignore user"));
        }

        imp.ignored_button.set_loading(false);
    }

    /// Connect to the signal emitted when the page should be closed.
    pub fn connect_close<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "close",
            true,
            closure_local!(|obj: Self| {
                f(&obj);
            }),
        )
    }
}
