use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{
    gio,
    glib::{self, clone, closure_local},
    CompositeTemplate, ListScrollFlags,
};
use tracing::error;

mod icon_item_row;
mod room_row;
mod row;
mod section_row;
mod verification_row;

use self::{
    icon_item_row::IconItemRow, room_row::RoomRow, row::Row, section_row::SidebarSectionRow,
    verification_row::VerificationRow,
};
use super::{account_settings::AccountSettingsSubpage, AccountSettings};
use crate::{
    account_switcher::AccountSwitcherButton,
    components::OfflineBanner,
    session::model::{
        CryptoIdentityState, RecoveryState, RoomCategory, Selection, SessionVerificationState,
        SidebarListModel, SidebarSection, TargetRoomCategory, User,
    },
    utils::expression,
};

mod imp {
    use std::{
        cell::{Cell, OnceCell, RefCell},
        sync::LazyLock,
    };

    use glib::subclass::{InitializingObject, Signal};

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session/view/sidebar/mod.ui")]
    #[properties(wrapper_type = super::Sidebar)]
    pub struct Sidebar {
        #[template_child]
        pub header_bar: TemplateChild<adw::HeaderBar>,
        #[template_child]
        pub security_banner: TemplateChild<adw::Banner>,
        #[template_child]
        pub scrolled_window: TemplateChild<gtk::ScrolledWindow>,
        #[template_child]
        pub listview: TemplateChild<gtk::ListView>,
        #[template_child]
        pub room_search_entry: TemplateChild<gtk::SearchEntry>,
        #[template_child]
        pub room_search: TemplateChild<gtk::SearchBar>,
        #[template_child]
        pub room_row_menu: TemplateChild<gio::MenuModel>,
        pub room_row_popover: OnceCell<gtk::PopoverMenu>,
        /// The logged-in user.
        #[property(get, set = Self::set_user, explicit_notify, nullable)]
        pub user: RefCell<Option<User>>,
        /// The category of the source that activated drop mode.
        pub drop_source_category: Cell<Option<RoomCategory>>,
        /// The category of the drop target that is currently hovered.
        pub drop_active_target_category: Cell<Option<TargetRoomCategory>>,
        /// The list model of this sidebar.
        #[property(get, set = Self::set_list_model, explicit_notify, nullable)]
        pub list_model: glib::WeakRef<SidebarListModel>,
        pub expr_watch: RefCell<Option<gtk::ExpressionWatch>>,
        session_handler: RefCell<Option<glib::SignalHandlerId>>,
        security_handlers: RefCell<Vec<glib::SignalHandlerId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Sidebar {
        const NAME: &'static str = "Sidebar";
        type Type = super::Sidebar;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            AccountSwitcherButton::ensure_type();
            OfflineBanner::ensure_type();

            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);

            klass.set_css_name("sidebar");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for Sidebar {
        fn signals() -> &'static [Signal] {
            static SIGNALS: LazyLock<Vec<Signal>> = LazyLock::new(|| {
                vec![
                    Signal::builder("drop-source-category-changed").build(),
                    Signal::builder("drop-active-target-category-changed").build(),
                ]
            });
            SIGNALS.as_ref()
        }

        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            let factory = gtk::SignalListItemFactory::new();
            factory.connect_setup(clone!(
                #[weak]
                obj,
                move |_, item| {
                    let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                        error!("List item factory did not receive a list item: {item:?}");
                        return;
                    };
                    let row = Row::new(&obj);
                    item.set_child(Some(&row));
                    item.bind_property("item", &row, "item").build();
                }
            ));
            self.listview.set_factory(Some(&factory));

            self.listview.connect_activate(move |listview, pos| {
                let Some(model) = listview.model().and_downcast::<Selection>() else {
                    return;
                };
                let Some(item) = model.item(pos) else {
                    return;
                };

                if let Some(section) = item.downcast_ref::<SidebarSection>() {
                    section.set_is_expanded(!section.is_expanded());
                } else {
                    model.set_selected(pos);
                }
            });

            obj.property_expression("list-model")
                .chain_property::<SidebarListModel>("selection-model")
                .bind(&*self.listview, "model", None::<&glib::Object>);

            // FIXME: Remove this hack once https://gitlab.gnome.org/GNOME/gtk/-/issues/4938 is resolved
            self.scrolled_window
                .vscrollbar()
                .first_child()
                .unwrap()
                .set_overflow(gtk::Overflow::Hidden);
        }

        fn dispose(&self) {
            if let Some(expr_watch) = self.expr_watch.take() {
                expr_watch.unwatch();
            }

            if let Some(user) = self.user.take() {
                let session = user.session();
                if let Some(handler) = self.session_handler.take() {
                    session.disconnect(handler);
                }

                let security = session.security();
                for handler in self.security_handlers.take() {
                    security.disconnect(handler);
                }
            }
        }
    }

    impl WidgetImpl for Sidebar {}
    impl NavigationPageImpl for Sidebar {}

    impl Sidebar {
        /// Set the logged-in user.
        fn set_user(&self, user: Option<User>) {
            let prev_user = self.user.borrow().clone();
            if prev_user == user {
                return;
            }

            if let Some(user) = prev_user {
                let session = user.session();
                if let Some(handler) = self.session_handler.take() {
                    session.disconnect(handler);
                }

                let security = session.security();
                for handler in self.security_handlers.take() {
                    security.disconnect(handler);
                }
            }

            if let Some(user) = &user {
                let session = user.session();

                let offline_handler = session.connect_is_offline_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_security_banner();
                    }
                ));
                self.session_handler.replace(Some(offline_handler));

                let security = session.security();
                let crypto_identity_handler =
                    security.connect_crypto_identity_state_notify(clone!(
                        #[weak(rename_to = imp)]
                        self,
                        move |_| {
                            imp.update_security_banner();
                        }
                    ));
                let verification_handler = security.connect_verification_state_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_security_banner();
                    }
                ));
                let recovery_handler = security.connect_recovery_state_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_security_banner();
                    }
                ));

                self.security_handlers.replace(vec![
                    crypto_identity_handler,
                    verification_handler,
                    recovery_handler,
                ]);
            }

            self.user.replace(user);

            self.update_security_banner();
            self.obj().notify_user();
        }

        /// Set the list model of the sidebar.
        fn set_list_model(&self, list_model: Option<&SidebarListModel>) {
            if self.list_model.upgrade().as_ref() == list_model {
                return;
            }
            let obj = self.obj();

            if let Some(expr_watch) = self.expr_watch.take() {
                expr_watch.unwatch();
            }

            if let Some(list_model) = list_model {
                let expr_watch = expression::normalize_string(
                    self.room_search_entry.property_expression("text"),
                )
                .bind(&list_model.string_filter(), "search", None::<&glib::Object>);
                self.expr_watch.replace(Some(expr_watch));
            }

            self.list_model.set(list_model);
            obj.notify_list_model();
        }

        /// Update the security banner.
        fn update_security_banner(&self) {
            let Some(session) = self.user.borrow().as_ref().map(User::session) else {
                return;
            };

            if session.is_offline() {
                // Only show one banner at a time.
                // The user will not be able to solve security issues while offline anyway.
                self.security_banner.set_revealed(false);
                return;
            }

            let security = session.security();
            let crypto_identity_state = security.crypto_identity_state();
            let verification_state = security.verification_state();
            let recovery_state = security.recovery_state();

            if crypto_identity_state == CryptoIdentityState::Unknown
                || verification_state == SessionVerificationState::Unknown
                || recovery_state == RecoveryState::Unknown
            {
                // Do not show the banner prematurely, unknown states should solve themselves.
                self.security_banner.set_revealed(false);
                return;
            }

            if verification_state == SessionVerificationState::Verified
                && recovery_state == RecoveryState::Enabled
            {
                // No need for the banner.
                self.security_banner.set_revealed(false);
                return;
            }

            let (title, button) = if crypto_identity_state == CryptoIdentityState::Missing {
                (gettext("No crypto identity"), gettext("Enable"))
            } else if verification_state == SessionVerificationState::Unverified {
                (gettext("Crypto identity incomplete"), gettext("Verify"))
            } else {
                match recovery_state {
                    RecoveryState::Disabled => {
                        (gettext("Account recovery disabled"), gettext("Enable"))
                    }
                    RecoveryState::Incomplete => {
                        (gettext("Account recovery incomplete"), gettext("Recover"))
                    }
                    _ => unreachable!(),
                }
            };

            self.security_banner.set_title(&title);
            self.security_banner.set_button_label(Some(&button));
            self.security_banner.set_revealed(true);
        }
    }
}

glib::wrapper! {
    pub struct Sidebar(ObjectSubclass<imp::Sidebar>)
        @extends gtk::Widget, adw::NavigationPage, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl Sidebar {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn room_search_bar(&self) -> gtk::SearchBar {
        self.imp().room_search.clone()
    }

    /// Open the proper security flow to fix the current issue.
    #[template_callback]
    fn fix_security_issue(&self) {
        let Some(session) = self.user().map(|u| u.session()) else {
            return;
        };

        let dialog = AccountSettings::new(&session);

        // Show the security tab if the user uses the back button.
        dialog.set_visible_page_name("security");

        let security = session.security();
        let crypto_identity_state = security.crypto_identity_state();
        let verification_state = security.verification_state();

        let subpage = if crypto_identity_state == CryptoIdentityState::Missing
            || verification_state == SessionVerificationState::Unverified
        {
            AccountSettingsSubpage::CryptoIdentitySetup
        } else {
            AccountSettingsSubpage::RecoverySetup
        };
        dialog.show_subpage(subpage);

        dialog.present(Some(self));
    }

    /// The category of the source that activated drop mode.
    pub fn drop_source_category(&self) -> Option<RoomCategory> {
        self.imp().drop_source_category.get()
    }

    /// Set the category of the source that activated drop mode.
    fn set_drop_source_category(&self, source_category: Option<RoomCategory>) {
        let imp = self.imp();

        if self.drop_source_category() == source_category {
            return;
        }

        imp.drop_source_category.set(source_category);

        if source_category.is_some() {
            imp.listview.add_css_class("drop-mode");
        } else {
            imp.listview.remove_css_class("drop-mode");
        }

        let Some(item_list) = self.list_model().map(|model| model.item_list()) else {
            return;
        };

        item_list.set_show_all_for_room_category(source_category);
        self.emit_by_name::<()>("drop-source-category-changed", &[]);
    }

    /// The category of the drop target that is currently hovered.
    pub fn drop_active_target_category(&self) -> Option<TargetRoomCategory> {
        self.imp().drop_active_target_category.get()
    }

    /// Set the category of the drop target that is currently hovered.
    fn set_drop_active_target_category(&self, target_category: Option<TargetRoomCategory>) {
        if self.drop_active_target_category() == target_category {
            return;
        }

        self.imp().drop_active_target_category.set(target_category);
        self.emit_by_name::<()>("drop-active-target-category-changed", &[]);
    }

    /// The shared popover for a room row in the sidebar.
    pub fn room_row_popover(&self) -> &gtk::PopoverMenu {
        let imp = self.imp();
        imp.room_row_popover.get_or_init(|| {
            let popover = gtk::PopoverMenu::builder()
                .menu_model(&*imp.room_row_menu)
                .has_arrow(false)
                .halign(gtk::Align::Start)
                .build();
            popover.update_property(&[gtk::accessible::Property::Label(&gettext("Context Menu"))]);

            popover
        })
    }

    /// The `adw::HeaderBar` of the sidebar.
    pub fn header_bar(&self) -> &adw::HeaderBar {
        &self.imp().header_bar
    }

    /// Scroll to the currently selected element (room) of the sidebar.
    pub fn scroll_to_selection(&self) {
        let imp = self.imp();
        let Some(list_model) = self.list_model() else {
            return;
        };

        let selected = list_model.selection_model().selected();

        if selected != gtk::INVALID_LIST_POSITION {
            imp.listview
                .scroll_to(selected, ListScrollFlags::FOCUS, None);
        }
    }

    /// Connect to the signal emitted when the drop source category changed.
    pub fn connect_drop_source_category_changed<F: Fn(&Self) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "drop-source-category-changed",
            true,
            closure_local!(move |obj: Self| {
                f(&obj);
            }),
        )
    }

    /// Connect to the signal emitted when the drop active target category
    /// changed.
    pub fn connect_drop_active_target_category_changed<F: Fn(&Self) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "drop-active-target-category-changed",
            true,
            closure_local!(move |obj: Self| {
                f(&obj);
            }),
        )
    }
}
