mod account_switcher;
mod category;
mod category_row;
mod category_type;
mod entry;
mod entry_row;
mod entry_type;
mod item_list;
mod room_row;
mod row;
mod selection;
mod verification_row;

pub use self::category::Category;
use self::category_row::CategoryRow;
pub use self::category_type::CategoryType;
pub use self::entry::Entry;
use self::entry_row::EntryRow;
pub use self::entry_type::EntryType;
pub use self::item_list::ItemList;
use self::room_row::RoomRow;
use self::row::Row;
use self::selection::Selection;
use self::verification_row::VerificationRow;

use adw::subclass::prelude::BinImpl;
use gtk::{gio, glib, prelude::*, subclass::prelude::*, CompositeTemplate, SelectionModel};

use crate::components::Avatar;
use crate::session::room::Room;
use crate::session::verification::IdentityVerification;
use crate::session::Session;
use crate::session::User;
use account_switcher::AccountSwitcher;

mod imp {
    use super::*;
    use glib::subclass::InitializingObject;
    use once_cell::sync::Lazy;
    use std::cell::{Cell, RefCell};

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/sidebar.ui")]
    pub struct Sidebar {
        pub compact: Cell<bool>,
        pub selected_item: RefCell<Option<glib::Object>>,
        #[template_child]
        pub headerbar: TemplateChild<adw::HeaderBar>,
        #[template_child]
        pub account_switcher: TemplateChild<AccountSwitcher>,
        #[template_child]
        pub listview: TemplateChild<gtk::ListView>,
        #[template_child]
        pub room_search_entry: TemplateChild<gtk::SearchEntry>,
        #[template_child]
        pub room_search: TemplateChild<gtk::SearchBar>,
        pub user: RefCell<Option<User>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Sidebar {
        const NAME: &'static str = "Sidebar";
        type Type = super::Sidebar;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            RoomRow::static_type();
            Row::static_type();
            Avatar::static_type();
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for Sidebar {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpec::new_object(
                        "user",
                        "User",
                        "The logged in user",
                        User::static_type(),
                        glib::ParamFlags::READWRITE | glib::ParamFlags::EXPLICIT_NOTIFY,
                    ),
                    glib::ParamSpec::new_boolean(
                        "compact",
                        "Compact",
                        "Whether a compact view is used",
                        false,
                        glib::ParamFlags::READWRITE,
                    ),
                    glib::ParamSpec::new_object(
                        "item-list",
                        "Item List",
                        "The list of items in the sidebar",
                        ItemList::static_type(),
                        glib::ParamFlags::READWRITE | glib::ParamFlags::EXPLICIT_NOTIFY,
                    ),
                    glib::ParamSpec::new_object(
                        "selected-item",
                        "Selected Item",
                        "The selected item in this sidebar",
                        glib::Object::static_type(),
                        glib::ParamFlags::READWRITE | glib::ParamFlags::EXPLICIT_NOTIFY,
                    ),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(
            &self,
            obj: &Self::Type,
            _id: usize,
            value: &glib::Value,
            pspec: &glib::ParamSpec,
        ) {
            match pspec.name() {
                "compact" => {
                    let compact = value.get().unwrap();
                    self.compact.set(compact);
                }
                "user" => {
                    obj.set_user(value.get().unwrap());
                }
                "item-list" => {
                    obj.set_item_list(value.get().unwrap());
                }
                "selected-item" => {
                    let selected_item = value.get().unwrap();
                    obj.set_selected_item(selected_item);
                }
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "compact" => self.compact.get().to_value(),
                "user" => obj.user().to_value(),
                "selected-item" => obj.selected_item().to_value(),
                _ => unimplemented!(),
            }
        }

        fn constructed(&self, obj: &Self::Type) {
            self.parent_constructed(obj);

            self.listview.get().connect_activate(move |listview, pos| {
                let model: Option<Selection> = listview.model().and_then(|o| o.downcast().ok());
                let row: Option<gtk::TreeListRow> = model
                    .as_ref()
                    .and_then(|m| m.item(pos))
                    .and_then(|o| o.downcast().ok());

                let (model, row) = match (model, row) {
                    (Some(model), Some(row)) => (model, row),
                    _ => return,
                };

                match row.item() {
                    Some(o) if o.is::<Category>() => row.set_expanded(!row.is_expanded()),
                    Some(o) if o.is::<Room>() => model.set_selected(pos),
                    Some(o) if o.is::<Entry>() => model.set_selected(pos),
                    Some(o) if o.is::<IdentityVerification>() => model.set_selected(pos),
                    _ => {}
                }
            });
        }
    }

    impl WidgetImpl for Sidebar {}
    impl BinImpl for Sidebar {}
}

glib::wrapper! {
    pub struct Sidebar(ObjectSubclass<imp::Sidebar>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl Sidebar {
    pub fn new() -> Self {
        glib::Object::new(&[]).expect("Failed to create Sidebar")
    }

    pub fn selected_item(&self) -> Option<glib::Object> {
        let priv_ = imp::Sidebar::from_instance(self);
        priv_.selected_item.borrow().clone()
    }

    pub fn room_search_bar(&self) -> gtk::SearchBar {
        let priv_ = imp::Sidebar::from_instance(self);
        priv_.room_search.clone()
    }

    pub fn set_item_list(&self, item_list: Option<ItemList>) {
        let priv_ = imp::Sidebar::from_instance(self);
        let item_list = match item_list {
            Some(item_list) => item_list,
            None => {
                priv_.listview.set_model(gtk::NONE_SELECTION_MODEL);
                return;
            }
        };

        // TODO: hide empty categories
        let tree_model = gtk::TreeListModel::new(&item_list, false, true, |item| {
            item.clone().downcast::<gio::ListModel>().ok()
        });

        let room_expression = gtk::ClosureExpression::new(
            |value| {
                value[0]
                    .get::<gtk::TreeListRow>()
                    .unwrap()
                    .item()
                    .and_then(|o| o.downcast::<Room>().ok())
                    .map_or(String::new(), |o| o.display_name())
            },
            &[],
        );
        let filter = gtk::StringFilterBuilder::new()
            .match_mode(gtk::StringFilterMatchMode::Substring)
            .expression(&room_expression)
            .ignore_case(true)
            .build();
        let filter_model = gtk::FilterListModel::new(Some(&tree_model), Some(&filter));

        priv_
            .room_search_entry
            .bind_property("text", &filter, "search")
            .flags(glib::BindingFlags::SYNC_CREATE)
            .build();

        let selection = Selection::new(Some(&filter_model));
        self.bind_property("selected-item", &selection, "selected-item")
            .flags(glib::BindingFlags::SYNC_CREATE | glib::BindingFlags::BIDIRECTIONAL)
            .build();

        priv_.listview.set_model(Some(&selection));
    }

    pub fn set_selected_item(&self, selected_item: Option<glib::Object>) {
        let priv_ = imp::Sidebar::from_instance(self);

        if self.selected_item() == selected_item {
            return;
        }

        priv_.selected_item.replace(selected_item);
        self.notify("selected-item");
    }

    pub fn user(&self) -> Option<User> {
        let priv_ = &imp::Sidebar::from_instance(self);
        priv_.user.borrow().clone()
    }

    fn set_user(&self, user: Option<User>) {
        let priv_ = imp::Sidebar::from_instance(self);

        if self.user() == user {
            return;
        }

        priv_.user.replace(user);
        self.notify("user");
    }

    pub fn set_logged_in_users(
        &self,
        sessions_stack_pages: &SelectionModel,
        session_root: &Session,
    ) {
        imp::Sidebar::from_instance(self)
            .account_switcher
            .set_logged_in_users(sessions_stack_pages, session_root);
    }
}

impl Default for Sidebar {
    fn default() -> Self {
        Self::new()
    }
}
