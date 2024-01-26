use std::convert::From;

use adw::{prelude::*, subclass::prelude::*};
use gettextrs::{gettext, ngettext};
use gtk::{
    gio,
    glib::{self, clone},
    CompositeTemplate,
};
use matrix_sdk::RoomState;
use ruma::{
    api::client::{discovery::get_capabilities::Capabilities, room::upgrade_room},
    assign,
    events::{
        room::{avatar::ImageInfo, power_levels::PowerLevelAction},
        StateEventType,
    },
};
use tracing::error;

use super::room_upgrade_dialog::confirm_room_upgrade;
use crate::{
    components::{CheckLoadingRow, CustomEntry, EditableAvatar, SpinnerButton},
    session::model::{AvatarData, AvatarImage, MemberList, NotificationsRoomSetting, Room},
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
        pub members_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub members_count: TemplateChild<gtk::Label>,
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
        pub room_id: TemplateChild<adw::ActionRow>,
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
        pub changing_avatar: RefCell<Option<OngoingAsyncAction<String>>>,
        pub changing_name: RefCell<Option<OngoingAsyncAction<String>>>,
        pub changing_topic: RefCell<Option<OngoingAsyncAction<String>>>,
        pub expr_watches: RefCell<Vec<gtk::ExpressionWatch>>,
        pub notifications_settings_handlers: RefCell<Vec<glib::SignalHandlerId>>,
        pub membership_handler: RefCell<Option<glib::SignalHandlerId>>,
        pub permissions_handler: RefCell<Option<glib::SignalHandlerId>>,
        pub capabilities: RefCell<Capabilities>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for GeneralPage {
        const NAME: &'static str = "ContentRoomDetailsGeneralPage";
        type Type = super::GeneralPage;
        type ParentType = adw::PreferencesPage;

        fn class_init(klass: &mut Self::Class) {
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
                    }));
            self.permissions_handler.replace(Some(permissions_handler));

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
            obj.update_federated();
            obj.update_sections();
            obj.update_upgrade_button();
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
            spawn!(
                clone!(@weak obj => async move {
                    obj.change_avatar(file).await;
                })
            );
        }));
        avatar.connect_remove_avatar(clone!(@weak self as obj => move |_| {
            spawn!(
                clone!(@weak obj => async move {
                    obj.remove_avatar().await;
                })
            );
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
        let Some(window) = self.root().and_downcast::<gtk::Window>() else {
            return;
        };

        // Ask for confirmation.
        let confirm_dialog = adw::MessageDialog::builder()
            .transient_for(&window)
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

        if confirm_dialog.choose_future().await != "remove" {
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
    fn save_details_clicked(&self) {
        self.imp().save_details_btn.set_loading(true);
        self.enable_details(false);

        spawn!(clone!(@weak self as obj => async move {
            obj.save_details().await;
        }));
        self.set_edit_mode_enabled(false);
    }

    async fn save_details(&self) {
        let Some(room) = self.room() else {
            error!("Cannot save details with missing room");
            return;
        };
        let imp = self.imp();

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
        imp.members_count.set_text(&format!("{n}"));

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
        spawn!(
            clone!(@weak self as obj, @weak room, @weak settings => async move {
                if settings.set_per_room_setting(room.room_id().to_owned(), setting).await.is_err() {
                    toast!(
                        obj,
                        gettext("Could not change notifications setting")
                    );
                }

                obj.set_notifications_loading(false, setting);
                obj.update_notifications();
            })
        );
    }

    /// Copy the room ID to the clipboard.
    #[template_callback]
    fn copy_room_id(&self) {
        let text = self.imp().room_id.subtitle().unwrap_or_default();
        self.clipboard().set_text(&text);
        toast!(self, gettext("Matrix room ID copied to clipboard"));
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
    fn upgrade(&self) {
        spawn!(clone!(@weak self as obj => async move {
            obj.upgrade_inner().await;
        }));
    }

    async fn upgrade_inner(&self) {
        let Some(room) = self.room() else {
            return;
        };
        let Some(window) = self.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let imp = self.imp();

        // TODO: Hide upgrade button if room already upgraded?
        imp.upgrade_button.set_loading(true);
        let room_versions_capability = imp.capabilities.borrow().room_versions.clone();

        let Some(new_version) = confirm_room_upgrade(room_versions_capability, &window).await
        else {
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
                toast!(self, gettext("Failed to upgrade room"));
                imp.upgrade_button.set_loading(false);
            }
        }
    }
}
