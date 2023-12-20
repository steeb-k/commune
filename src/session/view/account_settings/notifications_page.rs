use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{gio, glib, glib::clone, CompositeTemplate};
use tracing::error;

use crate::{
    components::{LoadingBin, Spinner},
    i18n::gettext_f,
    session::model::{NotificationsGlobalSetting, NotificationsSettings},
    spawn, toast,
    utils::{BoundObjectWeakRef, DummyObject},
};

mod imp {
    use std::{
        cell::{Cell, RefCell},
        collections::HashMap,
        marker::PhantomData,
    };

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/account_settings/notifications_page.ui"
    )]
    #[properties(wrapper_type = super::NotificationsPage)]
    pub struct NotificationsPage {
        #[template_child]
        pub account_switch: TemplateChild<gtk::Switch>,
        #[template_child]
        pub session_row: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub global: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub global_all_bin: TemplateChild<LoadingBin>,
        #[template_child]
        pub global_all_radio: TemplateChild<gtk::CheckButton>,
        #[template_child]
        pub global_direct_bin: TemplateChild<LoadingBin>,
        #[template_child]
        pub global_direct_radio: TemplateChild<gtk::CheckButton>,
        #[template_child]
        pub global_mentions_bin: TemplateChild<LoadingBin>,
        #[template_child]
        pub global_mentions_radio: TemplateChild<gtk::CheckButton>,
        #[template_child]
        pub keywords: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub keywords_add_entry: TemplateChild<adw::EntryRow>,
        #[template_child]
        pub keywords_add_bin: TemplateChild<LoadingBin>,
        pub keywords_suffixes: RefCell<HashMap<glib::GString, LoadingBin>>,
        /// The notifications settings of the current session.
        #[property(get, set = Self::set_notifications_settings, explicit_notify)]
        pub notifications_settings: BoundObjectWeakRef<NotificationsSettings>,
        /// Whether the account section is busy.
        #[property(get)]
        pub account_loading: Cell<bool>,
        /// Whether the global section is busy.
        #[property(get)]
        pub global_loading: Cell<bool>,
        /// The global notifications setting, as a string.
        #[property(get = Self::global_setting, set = Self::set_global_setting)]
        pub global_setting: PhantomData<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for NotificationsPage {
        const NAME: &'static str = "NotificationsPage";
        type Type = super::NotificationsPage;
        type ParentType = adw::PreferencesPage;

        fn class_init(klass: &mut Self::Class) {
            Spinner::static_type();

            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);

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
            let obj = self.obj();

            self.keywords_add_entry
                .connect_changed(clone!(@weak obj => move |_| {
                    obj.update_keywords();
                }));
        }
    }

    impl WidgetImpl for NotificationsPage {}
    impl PreferencesPageImpl for NotificationsPage {}

    impl NotificationsPage {
        /// Set the notifications settings of the current session.
        fn set_notifications_settings(
            &self,
            notifications_settings: Option<&NotificationsSettings>,
        ) {
            if self.notifications_settings.obj().as_ref() == notifications_settings {
                return;
            }
            let obj = self.obj();

            self.notifications_settings.disconnect_signals();

            if let Some(settings) = notifications_settings {
                let account_enabled_handler =
                    settings.connect_account_enabled_notify(clone!(@weak obj => move |_| {
                        obj.update_account();
                    }));
                let session_enabled_handler =
                    settings.connect_session_enabled_notify(clone!(@weak obj => move |_| {
                        obj.update_session();
                    }));
                let global_setting_handler =
                    settings.connect_global_setting_notify(clone!(@weak obj => move |_| {
                        obj.update_global();
                    }));

                self.notifications_settings.set(
                    settings,
                    vec![
                        account_enabled_handler,
                        session_enabled_handler,
                        global_setting_handler,
                    ],
                );

                let extra_items = gio::ListStore::new::<glib::Object>();
                extra_items.append(&DummyObject::new("add"));

                let all_items = gio::ListStore::new::<glib::Object>();
                all_items.append(&settings.keywords_list());
                all_items.append(&extra_items);

                let flattened_list = gtk::FlattenListModel::new(Some(all_items));
                self.keywords.bind_model(
                    Some(&flattened_list),
                    clone!(@weak obj => @default-return { adw::ActionRow::new().upcast() }, move |item| obj.create_keyword_row(item)),
                );
            } else {
                self.keywords.bind_model(
                    None::<&gio::ListModel>,
                    clone!(@weak obj => @default-return { adw::ActionRow::new().upcast() }, move |item| obj.create_keyword_row(item)),
                );
            }

            obj.update_account();
            obj.notify_notifications_settings();
        }

        /// The global notifications setting, as a string.
        fn global_setting(&self) -> String {
            let Some(settings) = self.notifications_settings.obj() else {
                return String::new();
            };

            settings.global_setting().to_string()
        }

        /// Set the global notifications setting, as a string.
        fn set_global_setting(&self, default: String) {
            let default = match default.parse::<NotificationsGlobalSetting>() {
                Ok(default) => default,
                Err(_) => {
                    error!("Invalid value to set global default notifications setting: {default}");
                    return;
                }
            };

            self.obj().global_setting_changed(default);
        }
    }
}

glib::wrapper! {
    /// Preferences page to edit global notification settings.
    pub struct NotificationsPage(ObjectSubclass<imp::NotificationsPage>)
        @extends gtk::Widget, adw::PreferencesPage, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl NotificationsPage {
    pub fn new(notifications_settings: &NotificationsSettings) -> Self {
        glib::Object::builder()
            .property("notifications-settings", notifications_settings)
            .build()
    }

    /// Update the section about the account.
    fn update_account(&self) {
        let Some(settings) = self.notifications_settings() else {
            return;
        };
        let imp = self.imp();

        imp.account_switch.set_active(settings.account_enabled());
        imp.account_switch.set_sensitive(!self.account_loading());

        // Other sections will be disabled or not.
        self.update_session();
    }

    /// Update the section about the session.
    fn update_session(&self) {
        let Some(settings) = self.notifications_settings() else {
            return;
        };
        let imp = self.imp();

        imp.session_row.set_active(settings.session_enabled());
        imp.session_row.set_sensitive(settings.account_enabled());

        // Other sections will be disabled or not.
        self.update_global();
        self.update_keywords();
    }

    /// Update the section about global.
    fn update_global(&self) {
        let Some(settings) = self.notifications_settings() else {
            return;
        };
        let imp = self.imp();

        // Updates the active radio button.
        self.notify_global_setting();

        let sensitive =
            settings.account_enabled() && settings.session_enabled() && !self.global_loading();
        imp.global.set_sensitive(sensitive);
    }

    /// Update the section about keywords.
    fn update_keywords(&self) {
        let Some(settings) = self.notifications_settings() else {
            return;
        };
        let imp = self.imp();

        let sensitive = settings.account_enabled() && settings.session_enabled();
        imp.keywords.set_sensitive(sensitive);

        if !sensitive {
            // Nothing else to update.
            return;
        }

        imp.keywords_add_bin.set_sensitive(self.can_add_keyword());
    }

    fn set_account_loading(&self, loading: bool) {
        self.imp().account_loading.set(loading);
        self.notify_account_loading();
    }

    #[template_callback]
    fn account_switched(&self) {
        let Some(settings) = self.notifications_settings() else {
            return;
        };
        let imp = self.imp();

        let enabled = imp.account_switch.is_active();
        if enabled == settings.account_enabled() {
            // Nothing to do.
            return;
        }

        imp.account_switch.set_sensitive(false);
        self.set_account_loading(true);

        spawn!(clone!(@weak self as obj, @weak settings => async move {
            if settings.set_account_enabled(enabled).await.is_err() {
                let msg = if enabled {
                    gettext("Could not enable account notifications")
                } else {
                    gettext("Could not disable account notifications")
                };
                toast!(obj, msg);
            }

            obj.set_account_loading(false);
            obj.update_account();
        }));
    }

    #[template_callback]
    fn session_switched(&self) {
        let Some(settings) = self.notifications_settings() else {
            return;
        };
        let imp = self.imp();

        settings.set_session_enabled(imp.session_row.is_active());
    }

    fn set_global_loading(&self, loading: bool, setting: NotificationsGlobalSetting) {
        let imp = self.imp();

        // Only show the spinner on the selected one.
        imp.global_all_bin
            .set_is_loading(loading && setting == NotificationsGlobalSetting::All);
        imp.global_direct_bin
            .set_is_loading(loading && setting == NotificationsGlobalSetting::DirectAndMentions);
        imp.global_mentions_bin
            .set_is_loading(loading && setting == NotificationsGlobalSetting::MentionsOnly);

        self.imp().global_loading.set(loading);
        self.notify_global_loading();
    }

    #[template_callback]
    fn global_setting_changed(&self, setting: NotificationsGlobalSetting) {
        let Some(settings) = self.notifications_settings() else {
            return;
        };
        let imp = self.imp();

        if setting == settings.global_setting() {
            // Nothing to do.
            return;
        }

        imp.global.set_sensitive(false);
        self.set_global_loading(true, setting);

        spawn!(clone!(@weak self as obj, @weak settings => async move {
            if settings.set_global_setting(setting).await.is_err() {
                toast!(
                    obj,
                    gettext("Could not change global notifications setting")
                );
            }

            obj.set_global_loading(false, setting);
            obj.update_global();
        }));
    }

    /// Create a row in the keywords list for the given item.
    fn create_keyword_row(&self, item: &glib::Object) -> gtk::Widget {
        let imp = self.imp();

        if let Some(string_obj) = item.downcast_ref::<gtk::StringObject>() {
            let keyword = string_obj.string();
            let row = adw::ActionRow::builder().title(keyword.clone()).build();

            let suffix = LoadingBin::new();
            let remove_button = gtk::Button::builder()
                .icon_name("close-symbolic")
                .valign(gtk::Align::Center)
                .halign(gtk::Align::Center)
                .css_classes(["flat"])
                .tooltip_text(gettext_f("Remove “{keyword}”", &[("keyword", &keyword)]))
                .build();
            remove_button.connect_clicked(clone!(@weak self as obj, @weak row => move |_| {
                obj.remove_keyword(row.title());
            }));
            suffix.set_child(Some(remove_button));

            row.add_suffix(&suffix);
            // We need to keep track of suffixes to change their loading state.
            imp.keywords_suffixes.borrow_mut().insert(keyword, suffix);

            row.upcast()
        } else {
            // It can only be the dummy item to add a new keyword.
            imp.keywords_add_entry.clone().upcast()
        }
    }

    /// Remove the given keyword.
    fn remove_keyword(&self, keyword: glib::GString) {
        let Some(settings) = self.notifications_settings() else {
            return;
        };

        let Some(suffix) = self.imp().keywords_suffixes.borrow().get(&keyword).cloned() else {
            return;
        };

        suffix.set_is_loading(true);

        spawn!(
            clone!(@weak self as obj, @weak settings, @weak suffix => async move {
                if settings.remove_keyword(keyword.to_string()).await.is_err() {
                    toast!(
                        obj,
                        gettext("Could not remove notification keyword")
                    );
                } else {
                    // The row should be removed.
                    obj.imp().keywords_suffixes.borrow_mut().remove(&keyword);
                }

                suffix.set_is_loading(false);
            })
        );
    }

    /// Whether we can add the keyword that is currently in the entry.
    fn can_add_keyword(&self) -> bool {
        let imp = self.imp();

        // Cannot add a keyword is section is disabled.
        if !imp.keywords.is_sensitive() {
            return false;
        }

        // Cannot add a keyword if a keyword is already being added.
        if imp.keywords_add_bin.is_loading() {
            return false;
        }

        let text = imp.keywords_add_entry.text().to_lowercase();

        // Cannot add an empty keyword.
        if text.is_empty() {
            return false;
        }

        // Cannot add a keyword without the API.
        let Some(settings) = self.notifications_settings() else {
            return false;
        };

        // Cannot add a keyword that already exists.
        let keywords_list = settings.keywords_list();
        for keyword_obj in keywords_list.iter::<glib::Object>() {
            let Ok(keyword_obj) = keyword_obj else {
                break;
            };

            if let Some(keyword) = keyword_obj
                .downcast_ref::<gtk::StringObject>()
                .map(|o| o.string())
            {
                if keyword.to_lowercase() == text {
                    return false;
                }
            }
        }

        true
    }

    /// Add the keyword that is currently in the entry.
    #[template_callback]
    fn add_keyword(&self) {
        let Some(settings) = self.notifications_settings() else {
            return;
        };
        if !self.can_add_keyword() {
            return;
        }
        let imp = self.imp();

        imp.keywords_add_entry.set_sensitive(false);
        imp.keywords_add_bin.set_is_loading(true);

        spawn!(clone!(@weak self as obj, @weak settings => async move {
            let imp = obj.imp();
            let keyword = imp.keywords_add_entry.text().into();

            if settings.add_keyword(keyword).await.is_err() {
                toast!(
                    obj,
                    gettext("Could not add notification keyword")
                );
            } else {
                // Adding the keyword was successful, reset the entry.
                imp.keywords_add_entry.set_text("");
            }

            imp.keywords_add_bin.set_is_loading(false);
            imp.keywords_add_entry.set_sensitive(true);
            obj.update_keywords();
        }));
    }
}
