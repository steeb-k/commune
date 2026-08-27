use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{gio, glib, glib::clone};

use crate::{
    components::{CheckLoadingRow, EntryAddRow, RemovableRow, SwitchLoadingRow},
    i18n::gettext_f,
    session::{NotificationsGlobalSetting, NotificationsSettings, NotificationsSpecialRule},
    spawn, toast,
    utils::{BoundObjectWeakRef, PlaceholderObject, SingleItemListModel},
};

mod imp {
    use std::{cell::Cell, marker::PhantomData};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/account_settings/notifications_page.ui")]
    #[properties(wrapper_type = super::NotificationsPage)]
    pub struct NotificationsPage {
        #[template_child]
        account_row: TemplateChild<SwitchLoadingRow>,
        #[template_child]
        session_row: TemplateChild<adw::SwitchRow>,
        #[template_child]
        global: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        global_all_row: TemplateChild<CheckLoadingRow>,
        #[template_child]
        global_direct_row: TemplateChild<CheckLoadingRow>,
        #[template_child]
        global_mentions_row: TemplateChild<CheckLoadingRow>,
        #[template_child]
        special_rules: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        mention_rule_row: TemplateChild<SwitchLoadingRow>,
        #[template_child]
        room_mention_rule_row: TemplateChild<SwitchLoadingRow>,
        #[template_child]
        invite_rule_row: TemplateChild<SwitchLoadingRow>,
        #[template_child]
        call_rule_row: TemplateChild<SwitchLoadingRow>,
        #[template_child]
        keywords: TemplateChild<gtk::ListBox>,
        #[template_child]
        keywords_add_row: TemplateChild<EntryAddRow>,
        // The Android-only background-delivery section. The children exist on
        // every platform — the template does — but stay hidden and untouched
        // outside Android.
        #[template_child]
        delivery_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        delivery_row: TemplateChild<adw::ComboRow>,
        #[template_child]
        push_status_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        push_status_next: TemplateChild<gtk::Image>,
        /// The notifications settings of the current session.
        #[property(get, set = Self::set_notifications_settings, explicit_notify)]
        notifications_settings: BoundObjectWeakRef<NotificationsSettings>,
        /// Whether the account section is busy.
        #[property(get)]
        account_loading: Cell<bool>,
        /// Whether the global section is busy.
        #[property(get)]
        global_loading: Cell<bool>,
        /// The global notifications setting, as a string.
        #[property(get = Self::global_setting, set = Self::set_global_setting)]
        global_setting: PhantomData<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for NotificationsPage {
        const NAME: &'static str = "NotificationsPage";
        type Type = super::NotificationsPage;
        type ParentType = adw::PreferencesPage;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);

            klass.install_property_action("notifications.set-global-default", "global-setting");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for NotificationsPage {
        fn constructed(&self) {
            self.parent_constructed();

            #[cfg(target_os = "android")]
            self.init_delivery_group();
        }
    }

    impl WidgetImpl for NotificationsPage {}
    impl PreferencesPageImpl for NotificationsPage {}

    /// The Android-only background-delivery section — the settings home of
    /// step 5 of `doc/android-push-plan.md`, so that the choice the one-time
    /// setup dialog offers stays reachable: the person who taps through a
    /// setup screen is not the person who later installs ntfy.
    #[cfg(target_os = "android")]
    impl NotificationsPage {
        /// The `background-delivery` values, in the combo's row order.
        const DELIVERY_MODES: &'static [&'static str] = &["auto", "push", "service"];

        /// Show and fill the background-delivery section.
        fn init_delivery_group(&self) {
            let choices = gtk::StringList::new(&[
                // Translators: How new messages reach the device while the
                // app is closed: push when possible, the background service
                // otherwise.
                &gettext("Automatic"),
                // Translators: Same context: never fall back to the
                // background service.
                &gettext("Push only"),
                // Translators: Same context: never rely on push.
                &gettext("Keep Commune running"),
            ]);
            self.delivery_row.set_model(Some(&choices));

            let mode = crate::Application::default()
                .settings()
                .string("background-delivery");
            let index = Self::DELIVERY_MODES
                .iter()
                .position(|known| *known == mode)
                .unwrap_or(0);
            self.delivery_row.set_selected(index as u32);

            // Re-detect whenever the page comes back on screen: the person
            // who left for the store comes back here.
            self.obj().connect_map(|obj| {
                obj.imp().update_push_status();
            });

            self.update_push_status();
            self.delivery_group.set_visible(true);
        }

        /// Write the chosen mode, when it changed.
        fn apply_delivery_mode(&self) {
            let Some(mode) = Self::DELIVERY_MODES.get(self.delivery_row.selected() as usize) else {
                return;
            };

            let settings = crate::Application::default().settings();
            if settings.string("background-delivery") != *mode
                && let Err(error) = settings.set_string("background-delivery", mode)
            {
                tracing::error!("Could not save the background delivery mode: {error}");
            }
        }

        /// Reflect the current push delivery status in its row.
        fn update_push_status(&self) {
            use crate::utils::android_push::{self, DeliveryStatus};

            let status = android_push::delivery_status();
            let subtitle = match status {
                DeliveryStatus::Delivering => {
                    gettext("Connected — new messages arrive through the notification app")
                }
                DeliveryStatus::Pending => {
                    gettext("Waiting for the notification app and the homeserver")
                }
                DeliveryStatus::NoDistributor => {
                    gettext("Needs a small notification app — tap to get ntfy")
                }
            };
            self.push_status_row.set_subtitle(&subtitle);

            let needs_app = status == DeliveryStatus::NoDistributor;
            self.push_status_row.set_activatable(needs_app);
            self.push_status_next.set_visible(needs_app);
        }

        /// The push status row was tapped: the door to getting a distributor.
        fn push_status_tapped(&self) {
            use crate::utils::android_push::{self, DeliveryStatus};

            if android_push::delivery_status() == DeliveryStatus::NoDistributor
                && let Some(window) = self.obj().root().and_downcast::<gtk::Window>()
            {
                android_push::open_ntfy_store(&window);
            }
            self.update_push_status();
        }
    }

    #[gtk::template_callbacks]
    impl NotificationsPage {
        /// Set the notifications settings of the current session.
        fn set_notifications_settings(
            &self,
            notifications_settings: Option<&NotificationsSettings>,
        ) {
            if self.notifications_settings.obj().as_ref() == notifications_settings {
                return;
            }

            self.notifications_settings.disconnect_signals();

            if let Some(settings) = notifications_settings {
                let account_enabled_handler = settings.connect_account_enabled_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_account();
                    }
                ));
                let session_enabled_handler = settings.connect_session_enabled_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_session();
                    }
                ));
                let global_setting_handler = settings.connect_global_setting_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_global();
                    }
                ));

                let mention_rule_handler = settings.connect_mention_rule_enabled_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_special_rules();
                    }
                ));
                let room_mention_rule_handler =
                    settings.connect_room_mention_rule_enabled_notify(clone!(
                        #[weak(rename_to = imp)]
                        self,
                        move |_| {
                            imp.update_special_rules();
                        }
                    ));
                let invite_rule_handler = settings.connect_invite_rule_enabled_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_special_rules();
                    }
                ));
                let call_rule_handler = settings.connect_call_rule_enabled_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_special_rules();
                    }
                ));

                self.notifications_settings.set(
                    settings,
                    vec![
                        account_enabled_handler,
                        session_enabled_handler,
                        global_setting_handler,
                        mention_rule_handler,
                        room_mention_rule_handler,
                        invite_rule_handler,
                        call_rule_handler,
                    ],
                );

                let extra_items = SingleItemListModel::new(Some(&PlaceholderObject::new("add")));

                let all_items = gio::ListStore::new::<glib::Object>();
                all_items.append(&settings.keywords_list());
                all_items.append(&extra_items);

                let flattened_list = gtk::FlattenListModel::new(Some(all_items));
                self.keywords.bind_model(
                    Some(&flattened_list),
                    clone!(
                        #[weak(rename_to = imp)]
                        self,
                        #[upgrade_or_else]
                        || { adw::ActionRow::new().upcast() },
                        move |item| imp.create_keyword_row(item)
                    ),
                );
            } else {
                self.keywords.bind_model(
                    None::<&gio::ListModel>,
                    clone!(
                        #[weak(rename_to = imp)]
                        self,
                        #[upgrade_or_else]
                        || { adw::ActionRow::new().upcast() },
                        move |item| imp.create_keyword_row(item)
                    ),
                );
            }

            self.update_account();
            self.obj().notify_notifications_settings();
        }

        /// Update the account row.
        fn update_account(&self) {
            let Some(settings) = self.notifications_settings.obj() else {
                return;
            };

            let checked = settings.account_enabled();
            self.account_row.set_is_active(checked);
            self.account_row.set_sensitive(!self.account_loading.get());

            // Other sections will be disabled or not.
            self.update_session();
        }

        /// Set the loading state of the account row.
        fn set_account_loading(&self, loading: bool) {
            self.account_loading.set(loading);
            self.obj().notify_account_loading();
        }

        /// Set the account setting.
        #[template_callback]
        async fn set_account_enabled(&self) {
            let Some(settings) = self.notifications_settings.obj() else {
                return;
            };

            let enabled = self.account_row.is_active();
            if enabled == settings.account_enabled() {
                // Nothing to do.
                return;
            }

            self.account_row.set_sensitive(false);
            self.set_account_loading(true);

            if settings.set_account_enabled(enabled).await.is_err() {
                let msg = if enabled {
                    gettext("Could not enable account notifications")
                } else {
                    gettext("Could not disable account notifications")
                };
                toast!(self.obj(), msg);
            }

            self.set_account_loading(false);
            self.update_account();
        }

        /// Update the session row.
        fn update_session(&self) {
            let Some(settings) = self.notifications_settings.obj() else {
                return;
            };

            self.session_row.set_active(settings.session_enabled());
            self.session_row.set_sensitive(settings.account_enabled());

            // Other sections will be disabled or not.
            self.update_global();
            self.update_special_rules();
            self.update_keywords();
        }

        /// Set the session setting.
        #[template_callback]
        fn set_session_enabled(&self) {
            let Some(settings) = self.notifications_settings.obj() else {
                return;
            };

            settings.set_session_enabled(self.session_row.is_active());
        }

        /// The global notifications setting, as a string.
        fn global_setting(&self) -> String {
            let Some(settings) = self.notifications_settings.obj() else {
                return String::new();
            };

            settings.global_setting().as_str().to_owned()
        }

        /// Update the global section.
        fn update_global(&self) {
            let Some(settings) = self.notifications_settings.obj() else {
                return;
            };

            // Updates the active radio button.
            self.obj().notify_global_setting();

            let sensitive = settings.account_enabled()
                && settings.session_enabled()
                && !self.global_loading.get();
            self.global.set_sensitive(sensitive);
        }

        /// Set the global setting, as a string.
        fn set_global_setting(&self, default: &str) {
            let default = NotificationsGlobalSetting::from_str(default);

            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    imp.set_global_setting_inner(default).await;
                }
            ));
        }

        /// Propagate the global setting.
        async fn set_global_setting_inner(&self, setting: NotificationsGlobalSetting) {
            let Some(settings) = self.notifications_settings.obj() else {
                return;
            };

            if setting == settings.global_setting() {
                // Nothing to do.
                return;
            }

            self.global.set_sensitive(false);
            self.set_global_loading(true, setting);

            if settings.set_global_setting(setting).await.is_err() {
                toast!(
                    self.obj(),
                    gettext("Could not change global notifications setting"),
                );
            }

            self.set_global_loading(false, setting);
            self.update_global();
        }

        /// Set the loading state of the global section.
        fn set_global_loading(&self, loading: bool, setting: NotificationsGlobalSetting) {
            // Only show the spinner on the selected one.
            self.global_all_row
                .set_is_loading(loading && setting == NotificationsGlobalSetting::All);
            self.global_direct_row.set_is_loading(
                loading && setting == NotificationsGlobalSetting::DirectAndMentions,
            );
            self.global_mentions_row
                .set_is_loading(loading && setting == NotificationsGlobalSetting::MentionsOnly);

            self.global_loading.set(loading);
            self.obj().notify_global_loading();
        }

        /// Update the section about the special push rules.
        fn update_special_rules(&self) {
            let Some(settings) = self.notifications_settings.obj() else {
                return;
            };

            self.mention_rule_row
                .set_is_active(settings.mention_rule_enabled());
            self.room_mention_rule_row
                .set_is_active(settings.room_mention_rule_enabled());
            self.invite_rule_row
                .set_is_active(settings.invite_rule_enabled());
            self.call_rule_row
                .set_is_active(settings.call_rule_enabled());

            let sensitive = settings.account_enabled() && settings.session_enabled();
            self.special_rules.set_sensitive(sensitive);
        }

        /// Toggle the given special push rule to match the given row.
        async fn toggle_special_rule(
            &self,
            row: &SwitchLoadingRow,
            rule: NotificationsSpecialRule,
        ) {
            let Some(settings) = self.notifications_settings.obj() else {
                return;
            };

            let enabled = row.is_active();
            if enabled == settings.special_rule_enabled(rule) {
                // Nothing to do.
                return;
            }

            row.set_sensitive(false);
            row.set_is_loading(true);

            if settings
                .set_special_rule_enabled(rule, enabled)
                .await
                .is_err()
            {
                toast!(self.obj(), gettext("Could not change notification rule"));
            }

            row.set_is_loading(false);
            row.set_sensitive(true);
            self.update_special_rules();
        }

        /// Toggle notifications for mentions of the user.
        #[template_callback]
        async fn set_mention_rule_enabled(&self) {
            self.toggle_special_rule(
                &self.mention_rule_row.clone(),
                NotificationsSpecialRule::UserMention,
            )
            .await;
        }

        /// Toggle notifications for mentions of the whole room.
        #[template_callback]
        async fn set_room_mention_rule_enabled(&self) {
            self.toggle_special_rule(
                &self.room_mention_rule_row.clone(),
                NotificationsSpecialRule::RoomMention,
            )
            .await;
        }

        /// Toggle notifications for invites.
        #[template_callback]
        async fn set_invite_rule_enabled(&self) {
            self.toggle_special_rule(
                &self.invite_rule_row.clone(),
                NotificationsSpecialRule::Invite,
            )
            .await;
        }

        /// Toggle notifications for incoming calls.
        #[template_callback]
        async fn set_call_rule_enabled(&self) {
            self.toggle_special_rule(&self.call_rule_row.clone(), NotificationsSpecialRule::Call)
                .await;
        }

        /// Update the section about keywords.
        #[template_callback]
        fn update_keywords(&self) {
            let Some(settings) = self.notifications_settings.obj() else {
                return;
            };

            let sensitive = settings.account_enabled() && settings.session_enabled();
            self.keywords.set_sensitive(sensitive);

            if !sensitive {
                // Nothing else to update.
                return;
            }

            self.keywords_add_row
                .set_inhibit_add(!self.can_add_keyword());
        }

        /// Create a row in the keywords list for the given item.
        fn create_keyword_row(&self, item: &glib::Object) -> gtk::Widget {
            let Some(string_obj) = item.downcast_ref::<gtk::StringObject>() else {
                // It can only be the dummy item to add a new keyword.
                return self.keywords_add_row.clone().upcast();
            };

            let keyword = string_obj.string();
            let row = RemovableRow::new();
            row.set_title(&keyword);
            row.set_remove_button_tooltip_text(Some(gettext_f(
                "Remove “{keyword}”",
                &[("keyword", &keyword)],
            )));

            row.connect_remove(clone!(
                #[weak(rename_to = imp)]
                self,
                move |row| {
                    imp.remove_keyword(row);
                }
            ));

            row.upcast()
        }

        /// Remove the keyword from the given row.
        fn remove_keyword(&self, row: &RemovableRow) {
            let Some(settings) = self.notifications_settings.obj() else {
                return;
            };

            row.set_is_loading(true);

            let obj = self.obj();
            spawn!(clone!(
                #[weak]
                obj,
                #[weak]
                row,
                async move {
                    if settings.remove_keyword(row.title().into()).await.is_err() {
                        toast!(obj, gettext("Could not remove notification keyword"));
                    }

                    row.set_is_loading(false);
                }
            ));
        }

        /// The background delivery mode combo changed.
        ///
        /// Bound on every platform because the template is; only Android has
        /// anything to do — which is also why `self` goes unused everywhere
        /// else.
        #[template_callback]
        #[cfg_attr(not(target_os = "android"), allow(clippy::unused_self))]
        fn delivery_changed(&self) {
            #[cfg(target_os = "android")]
            self.apply_delivery_mode();
        }

        /// The push status row was activated.
        #[template_callback]
        #[cfg_attr(not(target_os = "android"), allow(clippy::unused_self))]
        fn push_status_activated(&self) {
            #[cfg(target_os = "android")]
            self.push_status_tapped();
        }

        /// Whether we can add the keyword that is currently in the entry.
        fn can_add_keyword(&self) -> bool {
            // Cannot add a keyword if section is disabled.
            if !self.keywords.is_sensitive() {
                return false;
            }

            // Cannot add a keyword if a keyword is already being added.
            if self.keywords_add_row.is_loading() {
                return false;
            }

            let text = self.keywords_add_row.text().to_lowercase();

            // Cannot add an empty keyword.
            if text.is_empty() {
                return false;
            }

            // Cannot add a keyword without the API.
            let Some(settings) = self.notifications_settings.obj() else {
                return false;
            };

            // Cannot add a keyword that already exists.
            let keywords_list = settings.keywords_list();
            for keyword_obj in keywords_list.iter::<glib::Object>() {
                let Ok(keyword_obj) = keyword_obj else {
                    break;
                };

                if keyword_obj
                    .downcast_ref::<gtk::StringObject>()
                    .map(gtk::StringObject::string)
                    .is_some_and(|keyword| keyword.to_lowercase() == text)
                {
                    return false;
                }
            }

            true
        }

        /// Add the keyword that is currently in the entry.
        #[template_callback]
        async fn add_keyword(&self) {
            if !self.can_add_keyword() {
                return;
            }

            let Some(settings) = self.notifications_settings.obj() else {
                return;
            };

            self.keywords_add_row.set_is_loading(true);

            let keyword = self.keywords_add_row.text().into();

            if settings.add_keyword(keyword).await.is_err() {
                toast!(self.obj(), gettext("Could not add notification keyword"));
            } else {
                // Adding the keyword was successful, reset the entry.
                self.keywords_add_row.set_text("");
            }

            self.keywords_add_row.set_is_loading(false);
            self.update_keywords();
        }
    }
}

glib::wrapper! {
    /// Preferences page to edit global notification settings.
    pub struct NotificationsPage(ObjectSubclass<imp::NotificationsPage>)
        @extends gtk::Widget, adw::PreferencesPage,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl NotificationsPage {
    pub fn new(notifications_settings: &NotificationsSettings) -> Self {
        glib::Object::builder()
            .property("notifications-settings", notifications_settings)
            .build()
    }
}
