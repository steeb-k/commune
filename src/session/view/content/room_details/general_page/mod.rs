use adw::{prelude::*, subclass::prelude::*};
use gettextrs::{gettext, ngettext};
use gtk::{
    gio,
    glib::{self, clone},
    pango, CompositeTemplate,
};
use matrix_sdk::RoomState;
use ruma::{
    api::client::{
        directory::{get_room_visibility, set_room_visibility},
        discovery::get_capabilities::Capabilities,
        room::{upgrade_room, Visibility},
    },
    assign,
    events::{
        room::{
            avatar::ImageInfo,
            guest_access::{GuestAccess, RoomGuestAccessEventContent},
            history_visibility::RoomHistoryVisibilityEventContent,
            power_levels::PowerLevelAction,
        },
        StateEventType,
    },
};
use tracing::error;

use super::room_upgrade_dialog::confirm_room_upgrade;
use crate::{
    components::{
        AvatarData, AvatarImage, ButtonCountRow, CheckLoadingRow, ComboLoadingRow, CopyableRow,
        CustomEntry, EditableAvatar, SpinnerButton, SwitchLoadingRow,
    },
    gettext_f,
    prelude::*,
    session::model::{
        HistoryVisibilityValue, JoinRuleValue, MemberList, NotificationsRoomSetting, Room,
    },
    spawn, spawn_tokio, toast,
    utils::{
        expression,
        media::{get_image_info, load_file},
        template_callbacks::TemplateCallbacks,
        BoundObjectWeakRef, OngoingAsyncAction,
    },
};

mod imp {
    use std::{
        cell::{Cell, RefCell},
        marker::PhantomData,
    };

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_details/general_page/mod.ui"
    )]
    #[properties(wrapper_type = super::GeneralPage)]
    pub struct GeneralPage {
        /// The presented room.
        #[property(get, set = Self::set_room, explicit_notify, nullable)]
        pub room: BoundObjectWeakRef<Room>,
        pub room_members: RefCell<Option<MemberList>>,
        #[template_child]
        pub avatar: TemplateChild<EditableAvatar>,
        #[template_child]
        pub room_name_entry: TemplateChild<gtk::Entry>,
        #[template_child]
        pub room_topic_text_view: TemplateChild<gtk::TextView>,
        #[template_child]
        pub room_topic_entry: TemplateChild<CustomEntry>,
        #[template_child]
        pub room_topic_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub edit_details_btn: TemplateChild<gtk::Button>,
        #[template_child]
        pub save_details_btn: TemplateChild<SpinnerButton>,
        #[template_child]
        pub members_row: TemplateChild<ButtonCountRow>,
        #[template_child]
        pub notifications: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub notifications_global_row: TemplateChild<CheckLoadingRow>,
        #[template_child]
        pub notifications_all_row: TemplateChild<CheckLoadingRow>,
        #[template_child]
        pub notifications_mentions_row: TemplateChild<CheckLoadingRow>,
        #[template_child]
        pub notifications_mute_row: TemplateChild<CheckLoadingRow>,
        #[template_child]
        pub addresses_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub edit_addresses_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub no_addresses_label: TemplateChild<gtk::Label>,
        pub canonical_alias_row: RefCell<Option<CopyableRow>>,
        pub alt_aliases_rows: RefCell<Vec<CopyableRow>>,
        #[template_child]
        pub join_rule: TemplateChild<ComboLoadingRow>,
        #[template_child]
        pub guest_access: TemplateChild<SwitchLoadingRow>,
        #[template_child]
        pub publish: TemplateChild<SwitchLoadingRow>,
        #[template_child]
        pub history_visibility: TemplateChild<ComboLoadingRow>,
        #[template_child]
        pub encryption: TemplateChild<SwitchLoadingRow>,
        #[template_child]
        pub upgrade_button: TemplateChild<SpinnerButton>,
        #[template_child]
        pub room_federated: TemplateChild<adw::ActionRow>,
        /// Whether edit mode is enabled.
        #[property(get, set = Self::set_edit_mode_enabled, explicit_notify)]
        pub edit_mode_enabled: Cell<bool>,
        /// The notifications setting for the room.
        #[property(get = Self::notifications_setting, set = Self::set_notifications_setting, explicit_notify, builder(NotificationsRoomSetting::default()))]
        pub notifications_setting: PhantomData<NotificationsRoomSetting>,
        /// Whether the notifications section is busy.
        #[property(get)]
        pub notifications_loading: Cell<bool>,
        /// Whether the room is published in the directory.
        #[property(get)]
        pub is_published: Cell<bool>,
        pub changing_avatar: RefCell<Option<OngoingAsyncAction<String>>>,
        pub changing_name: RefCell<Option<OngoingAsyncAction<String>>>,
        pub changing_topic: RefCell<Option<OngoingAsyncAction<String>>>,
        pub expr_watches: RefCell<Vec<gtk::ExpressionWatch>>,
        pub notifications_settings_handlers: RefCell<Vec<glib::SignalHandlerId>>,
        pub membership_handler: RefCell<Option<glib::SignalHandlerId>>,
        pub permissions_handler: RefCell<Option<glib::SignalHandlerId>>,
        pub canonical_alias_handler: RefCell<Option<glib::SignalHandlerId>>,
        pub alt_aliases_handler: RefCell<Option<glib::SignalHandlerId>>,
        pub join_rule_handler: RefCell<Option<glib::SignalHandlerId>>,
        pub capabilities: RefCell<Capabilities>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for GeneralPage {
        const NAME: &'static str = "ContentRoomDetailsGeneralPage";
        type Type = super::GeneralPage;
        type ParentType = adw::PreferencesPage;

        fn class_init(klass: &mut Self::Class) {
            CopyableRow::ensure_type();

            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);
            TemplateCallbacks::bind_template_callbacks(klass);

            klass
                .install_property_action("room.set-notifications-setting", "notifications-setting");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for GeneralPage {
        fn dispose(&self) {
            self.obj().disconnect_all();
        }
    }

    impl WidgetImpl for GeneralPage {}
    impl PreferencesPageImpl for GeneralPage {}

    impl GeneralPage {
        /// Set the presented room.
        fn set_room(&self, room: Option<Room>) {
            let Some(room) = room else {
                // Just ignore when room is missing.
                return;
            };
            let obj = self.obj();

            obj.disconnect_all();

            let avatar_data = room.avatar_data();
            let expr_watch = AvatarData::this_expression("image")
                .chain_property::<AvatarImage>("uri")
                .watch(
                    Some(&avatar_data),
                    clone!(@weak obj, @weak avatar_data => move || {
                        obj.avatar_changed(avatar_data.image().and_then(|i| i.uri()));
                    }),
                );
            self.expr_watches.borrow_mut().push(expr_watch);

            let membership_handler =
                room.own_member()
                    .connect_membership_notify(clone!(@weak obj => move |_| {
                        obj.update_sections();
                    }));
            self.membership_handler.replace(Some(membership_handler));

            let permissions_handler =
                room.permissions()
                    .connect_changed(clone!(@weak obj => move |_| {
                        obj.update_upgrade_button();
                        obj.update_edit_addresses_button();
                        obj.update_join_rule();
                        obj.update_guest_access();
                        obj.update_history_visibility();
                        obj.update_encryption();

                        spawn!(async move {
                            obj.update_publish().await;
                        });
                    }));
            self.permissions_handler.replace(Some(permissions_handler));

            let aliases = room.aliases();
            let canonical_alias_handler =
                aliases.connect_canonical_alias_string_notify(clone!(@weak obj => move |_| {
                    obj.update_addresses();
                }));
            self.canonical_alias_handler
                .replace(Some(canonical_alias_handler));

            let alt_aliases_handler = aliases.alt_aliases_model().connect_items_changed(
                clone!(@weak obj => move |_,_,_,_| {
                    obj.update_addresses();
                }),
            );
            self.alt_aliases_handler.replace(Some(alt_aliases_handler));

            let join_rule_handler =
                room.join_rule()
                    .connect_changed(clone!(@weak obj => move |_| {
                        obj.update_join_rule();
                    }));
            self.join_rule_handler.replace(Some(join_rule_handler));

            let room_handler_ids = vec![
                room.connect_name_notify(clone!(@weak obj => move |room| {
                    obj.name_changed(room.name());
                })),
                room.connect_topic_notify(clone!(@weak obj => move |room| {
                    obj.topic_changed(room.topic());
                })),
                room.connect_joined_members_count_notify(clone!(@weak obj => move |room| {
                    obj.member_count_changed(room.joined_members_count());
                })),
                room.connect_notifications_setting_notify(clone!(@weak obj => move |_| {
                    obj.update_notifications();
                })),
                room.connect_is_tombstoned_notify(clone!(@weak obj => move |_| {
                    obj.update_upgrade_button();
                })),
                room.connect_guests_allowed_notify(clone!(@weak obj => move |_| {
                    obj.update_guest_access();
                })),
                room.connect_history_visibility_notify(clone!(@weak obj => move |_| {
                    obj.update_history_visibility();
                })),
                room.connect_is_encrypted_notify(clone!(@weak obj => move |_| {
                    obj.update_encryption();
                })),
            ];

            obj.member_count_changed(room.joined_members_count());
            obj.init_avatar();
            obj.init_edit_mode(&room);

            // Keep strong reference to members list.
            self.room_members
                .replace(Some(room.get_or_create_members()));

            self.room.set(&room, room_handler_ids);
            obj.notify_room();

            if let Some(session) = room.session() {
                let settings = session.notifications().settings();
                let notifications_settings_handlers = vec![
                    settings.connect_account_enabled_notify(clone!(@weak obj => move |_| {
                        obj.update_notifications();
                    })),
                    settings.connect_session_enabled_notify(clone!(@weak obj => move |_| {
                        obj.update_notifications();
                    })),
                ];

                self.notifications_settings_handlers
                    .replace(notifications_settings_handlers);
            }

            obj.update_notifications();
            obj.update_edit_addresses_button();
            obj.update_addresses();
            obj.update_federated();
            obj.update_sections();
            obj.update_join_rule();
            obj.update_guest_access();
            obj.update_publish_title();
            obj.update_history_visibility();
            obj.update_encryption();
            obj.update_upgrade_button();

            spawn!(clone!(@weak obj => async move {
                obj.update_publish().await;
            }));

            self.load_capabilities();
        }

        /// Set whether edit mode is enabled.
        fn set_edit_mode_enabled(&self, enabled: bool) {
            if self.edit_mode_enabled.get() == enabled {
                return;
            }
            let obj = self.obj();

            obj.enable_details(enabled);
            self.edit_mode_enabled.set(enabled);
            obj.notify_edit_mode_enabled();
        }

        /// The notifications setting for the room.
        fn notifications_setting(&self) -> NotificationsRoomSetting {
            self.room
                .obj()
                .map(|r| r.notifications_setting())
                .unwrap_or_default()
        }

        /// Set the notifications setting for the room.
        fn set_notifications_setting(&self, setting: NotificationsRoomSetting) {
            if self.notifications_setting() == setting {
                return;
            }

            self.obj().notifications_setting_changed(setting);
        }

        /// Fetch the capabilities of the homeserver.
        fn load_capabilities(&self) {
            let Some(room) = self.room.obj() else {
                return;
            };
            let client = room.matrix_room().client();

            spawn!(
                glib::Priority::LOW,
                clone!(@weak self as imp => async move {
                    let handle = spawn_tokio!(async move {
                        client.get_capabilities().await
                    });
                    match handle.await.unwrap() {
                        Ok(capabilities) => {
                            imp.capabilities.replace(capabilities);
                        }
                        Err(error) => {
                            error!("Could not get server capabilities: {error}");
                            imp.capabilities.take();
                        }
                    }
                })
            );
        }
    }
}

glib::wrapper! {
    /// Preference Window to display and update room details.
    pub struct GeneralPage(ObjectSubclass<imp::GeneralPage>)
        @extends gtk::Widget, adw::PreferencesPage, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl GeneralPage {
    pub fn new(room: &Room) -> Self {
        glib::Object::builder().property("room", room).build()
    }

    /// The members of the room.
    pub fn room_members(&self) -> MemberList {
        self.imp().room_members.borrow().clone().unwrap()
    }

    /// Update the visible sections according to the current state.
    fn update_sections(&self) {
        let Some(room) = self.room() else {
            return;
        };
        let imp = self.imp();

        let is_joined = room.is_joined();
        imp.notifications.set_visible(is_joined);
    }

    fn init_avatar(&self) {
        let avatar = &*self.imp().avatar;
        avatar.connect_edit_avatar(clone!(@weak self as obj => move |_, file| {
            spawn!(async move {
                obj.change_avatar(file).await;
            });
        }));
        avatar.connect_remove_avatar(clone!(@weak self as obj => move |_| {
            spawn!(async move {
                obj.remove_avatar().await;
            });
        }));
    }

    fn avatar_changed(&self, uri: Option<String>) {
        let imp = self.imp();

        if let Some(action) = imp.changing_avatar.borrow().as_ref() {
            if uri.as_ref() != action.as_value() {
                // This is not the change we expected, maybe another device did a change too.
                // Let's wait for another change.
                return;
            }
        } else {
            // No action is ongoing, we don't need to do anything.
            return;
        };

        // Reset the state.
        imp.changing_avatar.take();
        imp.avatar.success();
        if uri.is_none() {
            toast!(self, gettext("Avatar removed successfully"));
        } else {
            toast!(self, gettext("Avatar changed successfully"));
        }
    }

    async fn change_avatar(&self, file: gio::File) {
        let Some(room) = self.room() else {
            error!("Cannot change avatar with missing room");
            return;
        };
        let matrix_room = room.matrix_room();
        if matrix_room.state() != RoomState::Joined {
            error!("Cannot change avatar of room not joined");
            return;
        }

        let imp = self.imp();
        let avatar = &imp.avatar;
        avatar.edit_in_progress();

        let (data, info) = match load_file(&file).await {
            Ok(res) => res,
            Err(error) => {
                error!("Could not load room avatar file: {error}");
                toast!(self, gettext("Could not load file"));
                avatar.reset();
                return;
            }
        };

        let base_image_info = get_image_info(&file).await;
        let image_info = assign!(ImageInfo::new(), {
            width: base_image_info.width,
            height: base_image_info.height,
            size: info.size.map(Into::into),
            mimetype: Some(info.mime.to_string()),
        });

        let Some(session) = room.session() else {
            return;
        };
        let client = session.client();
        let handle = spawn_tokio!(async move { client.media().upload(&info.mime, data).await });

        let uri = match handle.await.unwrap() {
            Ok(res) => res.content_uri,
            Err(error) => {
                error!("Could not upload room avatar: {error}");
                toast!(self, gettext("Could not upload avatar"));
                avatar.reset();
                return;
            }
        };

        let (action, weak_action) = OngoingAsyncAction::set(uri.to_string());
        imp.changing_avatar.replace(Some(action));

        let matrix_room = matrix_room.clone();
        let handle =
            spawn_tokio!(async move { matrix_room.set_avatar_url(&uri, Some(image_info)).await });

        // We don't need to handle the success of the request, we should receive the
        // change via sync.
        if let Err(error) = handle.await.unwrap() {
            // Because this action can finish in avatar_changed, we must only act if this is
            // still the current action.
            if weak_action.is_ongoing() {
                imp.changing_avatar.take();
                error!("Could not change room avatar: {error}");
                toast!(self, gettext("Could not change avatar"));
                avatar.reset();
            }
        }
    }

    async fn remove_avatar(&self) {
        let Some(room) = self.room() else {
            error!("Cannot remove avatar with missing room");
            return;
        };
        let matrix_room = room.matrix_room();
        if matrix_room.state() != RoomState::Joined {
            error!("Cannot remove avatar of room not joined");
            return;
        }

        // Ask for confirmation.
        let confirm_dialog = adw::AlertDialog::builder()
            .default_response("cancel")
            .heading(gettext("Remove Avatar?"))
            .body(gettext(
                "Do you really want to remove the avatar for this room?",
            ))
            .build();
        confirm_dialog.add_responses(&[
            ("cancel", &gettext("Cancel")),
            ("remove", &gettext("Remove")),
        ]);
        confirm_dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);

        if confirm_dialog.choose_future(self).await != "remove" {
            return;
        }

        let imp = self.imp();
        let avatar = &*imp.avatar;
        avatar.removal_in_progress();

        let (action, weak_action) = OngoingAsyncAction::remove();
        imp.changing_avatar.replace(Some(action));

        let matrix_room = matrix_room.clone();
        let handle = spawn_tokio!(async move { matrix_room.remove_avatar().await });

        // We don't need to handle the success of the request, we should receive the
        // change via sync.
        if let Err(error) = handle.await.unwrap() {
            // Because this action can finish in avatar_changed, we must only act if this is
            // still the current action.
            if weak_action.is_ongoing() {
                imp.changing_avatar.take();
                error!("Could not remove room avatar: {error}");
                toast!(self, gettext("Could not remove avatar"));
                avatar.reset();
            }
        }
    }

    fn enable_details(&self, enabled: bool) {
        let imp = self.imp();

        if enabled {
            imp.room_topic_text_view
                .set_justification(gtk::Justification::Left);
            imp.room_name_entry.set_xalign(0.0);
            imp.room_name_entry.set_halign(gtk::Align::Center);
            imp.room_name_entry.set_sensitive(true);
            imp.room_name_entry.set_width_chars(25);
            imp.room_topic_entry.set_sensitive(true);
            imp.room_topic_label.set_visible(true);
        } else {
            imp.room_topic_text_view
                .set_justification(gtk::Justification::Center);
            imp.room_name_entry.set_xalign(0.5);
            imp.room_name_entry.set_sensitive(false);
            imp.room_name_entry.set_halign(gtk::Align::Fill);
            imp.room_name_entry.set_width_chars(-1);
            imp.room_topic_entry.set_sensitive(false);
            imp.room_topic_label.set_visible(false);
        }
    }

    fn init_edit_mode(&self, room: &Room) {
        let imp = self.imp();

        self.enable_details(false);

        // Hide edit controls when the user is not eligible to perform the actions.
        let permissions = room.permissions();
        let room_name_changeable = permissions.property_expression("can-change-name");
        let room_topic_changeable = permissions.property_expression("can-change-topic");
        let edit_mode_disabled = expression::not(self.property_expression("edit-mode-enabled"));

        let details_changeable = expression::or(room_name_changeable, room_topic_changeable);
        let edit_details_visible = expression::and(edit_mode_disabled, details_changeable);

        let expr_watch =
            edit_details_visible.bind(&*imp.edit_details_btn, "visible", gtk::Widget::NONE);
        imp.expr_watches.borrow_mut().push(expr_watch);
    }

    /// Finish the details changes if none are ongoing.
    fn finish_details_changes(&self) {
        let imp = self.imp();

        if imp.changing_name.borrow().is_some() {
            return;
        }
        if imp.changing_topic.borrow().is_some() {
            return;
        }

        self.set_edit_mode_enabled(false);
        imp.save_details_btn.set_loading(false);
    }

    fn name_changed(&self, name: Option<String>) {
        let imp = self.imp();

        if let Some(action) = imp.changing_name.borrow().as_ref() {
            if name.as_ref() != action.as_value() {
                // This is not the change we expected, maybe another device did a change too.
                // Let's wait for another change.
                return;
            }
        } else {
            // No action is ongoing, we don't need to do anything.
            return;
        };

        toast!(self, gettext("Room name saved successfully"));

        // Reset state.
        imp.changing_name.take();
        self.finish_details_changes();
    }

    fn topic_changed(&self, topic: Option<String>) {
        let imp = self.imp();

        // It is not possible to remove a topic so we process the empty string as
        // `None`. We need to cancel that here.
        let topic = topic.unwrap_or_default();

        if let Some(action) = imp.changing_topic.borrow().as_ref() {
            if Some(&topic) != action.as_value() {
                // This is not the change we expected, maybe another device did a change too.
                // Let's wait for another change.
                return;
            }
        } else {
            // No action is ongoing, we don't need to do anything.
            return;
        };

        toast!(self, gettext("Room topic saved successfully"));

        // Reset state.
        imp.changing_topic.take();
        self.finish_details_changes();
    }

    #[template_callback]
    fn edit_details_clicked(&self) {
        self.set_edit_mode_enabled(true);
    }

    #[template_callback]
    async fn save_details_clicked(&self) {
        let Some(room) = self.room() else {
            error!("Cannot save details with missing room");
            return;
        };
        let imp = self.imp();

        imp.save_details_btn.set_loading(true);
        self.enable_details(false);
        self.set_edit_mode_enabled(false);

        let raw_name = imp.room_name_entry.text();
        let trimmed_name = raw_name.trim();
        let name = (!trimmed_name.is_empty()).then(|| trimmed_name.to_owned());

        let topic_buffer = imp.room_topic_text_view.buffer();
        let raw_topic = topic_buffer
            .text(&topic_buffer.start_iter(), &topic_buffer.end_iter(), false)
            .to_string();
        let topic = raw_topic.trim().to_owned();

        let name_changed = if let Some(name) = &name {
            *name != room.display_name()
        } else {
            room.name().is_some()
        };
        let topic_changed = topic != room.topic().unwrap_or_default();

        if !name_changed && !topic_changed {
            return;
        }

        let matrix_room = room.matrix_room();
        if matrix_room.state() != RoomState::Joined {
            error!("Cannot change name or topic of room not joined");
            return;
        }

        if name_changed {
            let matrix_room = matrix_room.clone();

            let (action, weak_action) = if let Some(name) = name.clone() {
                OngoingAsyncAction::set(name)
            } else {
                OngoingAsyncAction::remove()
            };
            imp.changing_name.replace(Some(action));

            let handle =
                spawn_tokio!(async move { matrix_room.set_name(name.unwrap_or_default()).await });

            // We don't need to handle the success of the request, we should receive the
            // change via sync.
            if let Err(error) = handle.await.unwrap() {
                // Because this action can finish in name_changed, we must only act if this is
                // still the current action.
                if weak_action.is_ongoing() {
                    imp.changing_name.take();
                    error!("Could not change room name: {error}");
                    toast!(self, gettext("Could not change room name"));
                    self.enable_details(true);
                    imp.save_details_btn.set_loading(false);
                    return;
                }
            }
        }

        if topic_changed {
            let matrix_room = matrix_room.clone();

            let (action, weak_action) = OngoingAsyncAction::set(topic.clone());
            imp.changing_topic.replace(Some(action));

            let handle = spawn_tokio!(async move { matrix_room.set_room_topic(&topic).await });

            // We don't need to handle the success of the request, we should receive the
            // change via sync.
            if let Err(error) = handle.await.unwrap() {
                // Because this action can finish in topic_changed, we must only act if this is
                // still the current action.
                if weak_action.is_ongoing() {
                    imp.changing_topic.take();
                    error!("Could not change room topic: {error}");
                    toast!(self, gettext("Could not change room topic"));
                    self.enable_details(true);
                    imp.save_details_btn.set_loading(false);
                }
            }
        }
    }

    fn member_count_changed(&self, n: u64) {
        let imp = self.imp();
        imp.members_row.set_count(format!("{n}"));

        let n = n.try_into().unwrap_or(u32::MAX);
        imp.members_row.set_title(&ngettext("Member", "Members", n));
    }

    fn disconnect_all(&self) {
        let imp = self.imp();

        if let Some(room) = self.room() {
            if let Some(session) = room.session() {
                let settings = session.notifications().settings();
                for handler in imp.notifications_settings_handlers.take() {
                    settings.disconnect(handler);
                }
            }

            if let Some(handler) = imp.membership_handler.take() {
                room.own_member().disconnect(handler);
            }

            if let Some(handler) = imp.permissions_handler.take() {
                room.permissions().disconnect(handler);
            }

            let aliases = room.aliases();
            if let Some(handler) = imp.canonical_alias_handler.take() {
                aliases.disconnect(handler);
            }
            if let Some(handler) = imp.alt_aliases_handler.take() {
                aliases.alt_aliases_model().disconnect(handler);
            }

            if let Some(handler) = imp.join_rule_handler.take() {
                room.join_rule().disconnect(handler);
            }
        }

        imp.room.disconnect_signals();

        for watch in imp.expr_watches.take() {
            watch.unwatch();
        }
    }

    /// Update the section about notifications.
    fn update_notifications(&self) {
        let Some(room) = self.room() else {
            return;
        };
        let Some(session) = room.session() else {
            return;
        };
        let imp = self.imp();

        // Updates the active radio button.
        self.notify_notifications_setting();

        let settings = session.notifications().settings();
        let sensitive = settings.account_enabled()
            && settings.session_enabled()
            && !self.notifications_loading();
        imp.notifications.set_sensitive(sensitive);
    }

    /// Update the loading state in the notifications section.
    fn set_notifications_loading(&self, loading: bool, setting: NotificationsRoomSetting) {
        let imp = self.imp();

        // Only show the spinner on the selected one.
        imp.notifications_global_row
            .set_is_loading(loading && setting == NotificationsRoomSetting::Global);
        imp.notifications_all_row
            .set_is_loading(loading && setting == NotificationsRoomSetting::All);
        imp.notifications_mentions_row
            .set_is_loading(loading && setting == NotificationsRoomSetting::MentionsOnly);
        imp.notifications_mute_row
            .set_is_loading(loading && setting == NotificationsRoomSetting::Mute);

        self.imp().notifications_loading.set(loading);
        self.notify_notifications_loading();
    }

    /// Handle a change of the notifications setting.
    fn notifications_setting_changed(&self, setting: NotificationsRoomSetting) {
        let Some(room) = self.room() else {
            return;
        };
        let Some(session) = room.session() else {
            return;
        };
        let imp = self.imp();

        if setting == room.notifications_setting() {
            // Nothing to do.
            return;
        }

        imp.notifications.set_sensitive(false);
        self.set_notifications_loading(true, setting);

        let settings = session.notifications().settings();
        spawn!(clone!(@weak self as obj => async move {
            if settings.set_per_room_setting(room.room_id().to_owned(), setting).await.is_err() {
                toast!(
                    obj,
                    gettext("Could not change notifications setting")
                );
            }

            obj.set_notifications_loading(false, setting);
            obj.update_notifications();
        }));
    }

    /// Update the button to edit addresses.
    fn update_edit_addresses_button(&self) {
        let Some(room) = self.room() else {
            return;
        };

        let can_edit = room.is_joined()
            && room
                .permissions()
                .is_allowed_to(PowerLevelAction::SendState(StateEventType::RoomPowerLevels));
        self.imp().edit_addresses_button.set_visible(can_edit);
    }

    /// Update the addresses group.
    fn update_addresses(&self) {
        let Some(room) = self.room() else {
            return;
        };
        let imp = self.imp();
        let aliases = room.aliases();

        let canonical_alias_string = aliases.canonical_alias_string();
        let has_canonical_alias = canonical_alias_string.is_some();

        if let Some(canonical_alias_string) = canonical_alias_string {
            let mut row_borrow = imp.canonical_alias_row.borrow_mut();
            let row = row_borrow.get_or_insert_with(|| {
                // We want the main alias always at the top but cannot add a row at the top so
                // we have to remove the other rows first.
                self.remove_alt_aliases_rows();

                let row = CopyableRow::new();
                row.set_copy_button_tooltip_text(Some(gettext("Copy address")));
                row.set_toast_text(Some(gettext("Address copied to clipboard")));

                // Mark the main alias with a tag.
                let label = gtk::Label::builder()
                    .label(gettext("Main Address"))
                    .ellipsize(pango::EllipsizeMode::End)
                    .css_classes(["public-address-tag"])
                    .valign(gtk::Align::Center)
                    .build();
                row.update_relation(&[gtk::accessible::Relation::DescribedBy(&[
                    label.upcast_ref()
                ])]);
                row.set_extra_suffix(Some(label));

                imp.addresses_group.add(&row);

                row
            });

            row.set_title(&canonical_alias_string);
        } else if let Some(row) = imp.canonical_alias_row.take() {
            imp.addresses_group.remove(&row);
        }

        let alt_aliases = aliases.alt_aliases_model();
        let alt_aliases_count = alt_aliases.n_items() as usize;
        if alt_aliases_count == 0 {
            self.remove_alt_aliases_rows();
        } else {
            let mut rows = imp.alt_aliases_rows.borrow_mut();

            for (pos, alt_alias) in alt_aliases.iter::<glib::Object>().enumerate() {
                let Some(alt_alias) = alt_alias.ok().and_downcast::<gtk::StringObject>() else {
                    break;
                };

                let row = rows.get(pos).cloned().unwrap_or_else(|| {
                    let row = CopyableRow::new();
                    row.set_copy_button_tooltip_text(Some(gettext("Copy address")));
                    row.set_toast_text(Some(gettext("Address copied to clipboard")));

                    imp.addresses_group.add(&row);
                    rows.push(row.clone());

                    row
                });

                row.set_title(&alt_alias.string());
            }

            let rows_count = rows.len();
            if alt_aliases_count < rows_count {
                for _ in alt_aliases_count..rows_count {
                    if let Some(row) = rows.pop() {
                        imp.addresses_group.remove(&row);
                    }
                }
            }
        }

        imp.no_addresses_label
            .set_visible(!has_canonical_alias && alt_aliases_count == 0);
    }

    fn remove_alt_aliases_rows(&self) {
        let imp = self.imp();

        for row in imp.alt_aliases_rows.take() {
            imp.addresses_group.remove(&row);
        }
    }

    /// Update the join rule row.
    fn update_join_rule(&self) {
        let Some(room) = self.room() else {
            return;
        };

        let row = &self.imp().join_rule;
        row.set_is_loading(false);

        let permissions = room.permissions();
        let join_rule = room.join_rule();

        let is_supported_join_rule = matches!(
            join_rule.value(),
            JoinRuleValue::Public | JoinRuleValue::Invite
        ) && !join_rule.can_knock();
        let can_change =
            permissions.is_allowed_to(PowerLevelAction::SendState(StateEventType::RoomJoinRules));

        row.set_read_only(!is_supported_join_rule || !can_change);
        row.set_selected_string(Some(join_rule.display_name()));
    }

    /// Set the join rule of the room.
    #[template_callback]
    async fn set_join_rule(&self) {
        let Some(room) = self.room() else {
            return;
        };
        let join_rule = room.join_rule();

        let row = &self.imp().join_rule;

        let value = match row.selected() {
            0 => JoinRuleValue::Invite,
            1 => JoinRuleValue::Public,
            _ => {
                return;
            }
        };

        if join_rule.value() == value {
            // Nothing to do.
            return;
        }

        row.set_is_loading(true);
        row.set_read_only(true);

        if join_rule.set_value(value).await.is_err() {
            toast!(self, gettext("Could not change who can join"));
            self.update_join_rule();
        }
    }

    /// Update the guest access row.
    fn update_guest_access(&self) {
        let Some(room) = self.room() else {
            return;
        };

        let row = &self.imp().guest_access;
        row.set_is_active(room.guests_allowed());
        row.set_is_loading(false);

        let can_change = room
            .permissions()
            .is_allowed_to(PowerLevelAction::SendState(StateEventType::RoomGuestAccess));
        row.set_read_only(!can_change);
    }

    /// Toggle the guest access.
    #[template_callback]
    async fn toggle_guest_access(&self) {
        let Some(room) = self.room() else { return };

        let row = &self.imp().guest_access;
        let guests_allowed = row.is_active();

        if room.guests_allowed() == guests_allowed {
            return;
        }

        row.set_is_loading(true);
        row.set_read_only(true);

        let guest_access = if guests_allowed {
            GuestAccess::CanJoin
        } else {
            GuestAccess::Forbidden
        };
        let content = RoomGuestAccessEventContent::new(guest_access);

        let matrix_room = room.matrix_room().clone();
        let handle = spawn_tokio!(async move { matrix_room.send_state_event(content).await });

        if let Err(error) = handle.await.unwrap() {
            error!("Could not change guest access: {error}");
            toast!(self, gettext("Could not change guest access"));
            self.update_guest_access();
        }
    }

    /// Update the title of the publish row.
    fn update_publish_title(&self) {
        let Some(room) = self.room() else {
            return;
        };

        let own_member = room.own_member();
        let server_name = own_member.user_id().server_name();

        let title = gettext_f(
            // Translators: Do NOT translate the content between '{' and '}',
            // this is a variable name.
            "Publish in the {homeserver} directory",
            &[("homeserver", server_name.as_str())],
        );
        self.imp().publish.set_title(&title);
    }

    /// Update the publish row.
    async fn update_publish(&self) {
        let Some(room) = self.room() else {
            return;
        };

        let imp = self.imp();
        let row = &imp.publish;

        // There is no clear definition of who is allowed to publish a room to the
        // directory in the Matrix spec. Let's assume it doesn't make sense unless the
        // user can change the public addresses.
        let can_change = room
            .permissions()
            .is_allowed_to(PowerLevelAction::SendState(
                StateEventType::RoomCanonicalAlias,
            ));
        row.set_read_only(!can_change);

        let matrix_room = room.matrix_room();
        let client = matrix_room.client();
        let request = get_room_visibility::v3::Request::new(matrix_room.room_id().to_owned());

        let handle = spawn_tokio!(async move { client.send(request, None).await });

        match handle.await.unwrap() {
            Ok(response) => {
                let is_published = response.visibility == Visibility::Public;
                imp.is_published.set(is_published);
                row.set_is_active(is_published);
            }
            Err(error) => {
                error!("Could not get directory visibility of room: {error}");
            }
        }

        row.set_is_loading(false);
    }

    /// Toggle whether the room is published in the room directory.
    #[template_callback]
    async fn toggle_publish(&self) {
        let Some(room) = self.room() else { return };

        let imp = self.imp();
        let row = &imp.publish;
        let publish = row.is_active();

        if imp.is_published.get() == publish {
            return;
        }

        row.set_is_loading(true);
        row.set_read_only(true);

        let visibility = if publish {
            Visibility::Public
        } else {
            Visibility::Private
        };

        let matrix_room = room.matrix_room();
        let client = matrix_room.client();
        let request =
            set_room_visibility::v3::Request::new(matrix_room.room_id().to_owned(), visibility);

        let handle = spawn_tokio!(async move { client.send(request, None).await });

        if let Err(error) = handle.await.unwrap() {
            error!("Could not change directory visibility of room: {error}");
            let text = if publish {
                gettext("Could not publish room in directory")
            } else {
                gettext("Could not unpublish room from directory")
            };
            toast!(self, text);
        }

        self.update_publish().await;
    }

    /// Update the history visibility edit button.
    fn update_history_visibility(&self) {
        let Some(room) = self.room() else {
            return;
        };

        let row = &self.imp().history_visibility;
        row.set_is_loading(false);

        let visibility = room.history_visibility();

        let text = match visibility {
            HistoryVisibilityValue::WorldReadable => {
                gettext("Anyone, even if they are not in the room")
            }
            HistoryVisibilityValue::Shared => {
                gettext("Members only, since this option was selected")
            }
            HistoryVisibilityValue::Invited => gettext("Members only, since they were invited"),
            HistoryVisibilityValue::Joined => gettext("Members only, since they joined the room"),
            HistoryVisibilityValue::Unsupported => gettext("Unsupported rule"),
        };
        row.set_selected_string(Some(text));

        let is_supported = visibility != HistoryVisibilityValue::Unsupported;
        let can_change = room
            .permissions()
            .is_allowed_to(PowerLevelAction::SendState(
                StateEventType::RoomHistoryVisibility,
            ));

        row.set_read_only(!is_supported || !can_change);
    }

    /// Set the history_visibility of the room.
    #[template_callback]
    async fn set_history_visibility(&self) {
        let Some(room) = self.room() else {
            return;
        };
        let row = &self.imp().history_visibility;

        let visibility = match row.selected() {
            0 => HistoryVisibilityValue::WorldReadable,
            1 => HistoryVisibilityValue::Shared,
            2 => HistoryVisibilityValue::Joined,
            3 => HistoryVisibilityValue::Invited,
            _ => {
                return;
            }
        };

        if room.history_visibility() == visibility {
            // Nothing to do.
            return;
        }

        row.set_is_loading(true);
        row.set_read_only(true);

        let content = RoomHistoryVisibilityEventContent::new(visibility.into());

        let matrix_room = room.matrix_room().clone();
        let handle = spawn_tokio!(async move { matrix_room.send_state_event(content).await });

        if let Err(error) = handle.await.unwrap() {
            error!("Could not change room history visibility: {error}");
            toast!(self, gettext("Could not change who can read history"));

            self.update_history_visibility();
        }
    }

    /// Update the encryption row.
    fn update_encryption(&self) {
        let Some(room) = self.room() else {
            return;
        };

        let imp = self.imp();
        let row = &imp.encryption;
        row.set_is_loading(false);

        let is_encrypted = room.is_encrypted();
        row.set_is_active(is_encrypted);

        let can_change = !is_encrypted
            && room
                .permissions()
                .is_allowed_to(PowerLevelAction::SendState(StateEventType::RoomEncryption));
        row.set_read_only(!can_change);
    }

    /// Enable encryption in the room.
    #[template_callback]
    async fn enable_encryption(&self) {
        let Some(room) = self.room() else { return };

        let imp = self.imp();
        let row = &imp.encryption;

        if room.is_encrypted() || !row.is_active() {
            // Nothing to do.
            return;
        }

        row.set_is_loading(true);
        row.set_read_only(true);

        // Ask for confirmation.
        let dialog = adw::AlertDialog::builder()
                .heading(gettext("Enable Encryption?"))
                .body(gettext("Enabling encryption will prevent new members to read the history before they arrived. This cannot be disabled later."))
                .default_response("cancel")
                .build();
        dialog.add_responses(&[
            ("cancel", &gettext("Cancel")),
            ("enable", &gettext("Enable")),
        ]);
        dialog.set_response_appearance("enable", adw::ResponseAppearance::Destructive);

        if dialog.choose_future(self).await != "enable" {
            self.update_encryption();
            return;
        };

        if room.enable_encryption().await.is_err() {
            toast!(self, gettext("Could not enable encryption"));
            self.update_encryption();
        }
    }

    /// Update the room upgrade button.
    fn update_upgrade_button(&self) {
        let Some(room) = self.room() else {
            return;
        };

        let can_upgrade = !room.is_tombstoned()
            && room
                .permissions()
                .is_allowed_to(PowerLevelAction::SendState(StateEventType::RoomTombstone));
        self.imp().upgrade_button.set_visible(can_upgrade);
    }

    /// Update the room federation row.
    fn update_federated(&self) {
        let Some(room) = self.room() else {
            return;
        };

        let subtitle = if room.federated() {
            // Translators: As in, 'Room federated'.
            gettext("Federated")
        } else {
            // Translators: As in, 'Room not federated'.
            gettext("Not federated")
        };

        self.imp().room_federated.set_subtitle(&subtitle);
    }

    /// Upgrade the room to a new version.
    #[template_callback]
    async fn upgrade(&self) {
        let Some(room) = self.room() else {
            return;
        };
        let imp = self.imp();

        // TODO: Hide upgrade button if room already upgraded?
        imp.upgrade_button.set_loading(true);
        let room_versions_capability = imp.capabilities.borrow().room_versions.clone();

        let Some(new_version) = confirm_room_upgrade(room_versions_capability, self).await else {
            imp.upgrade_button.set_loading(false);
            return;
        };

        let client = room.matrix_room().client();
        let request = upgrade_room::v3::Request::new(room.room_id().to_owned(), new_version);

        let handle = spawn_tokio!(async move { client.send(request, None).await });

        match handle.await.unwrap() {
            Ok(_) => {
                toast!(self, gettext("Room upgraded successfully"));
            }
            Err(error) => {
                error!("Could not upgrade room: {error}");
                toast!(self, gettext("Could not upgrade room"));
                imp.upgrade_button.set_loading(false);
            }
        }
    }
}
