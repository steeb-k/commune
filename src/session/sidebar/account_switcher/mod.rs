use gtk::{
    gio::{self, ListModel, ListStore},
    glib,
    prelude::*,
    subclass::prelude::*,
    CompositeTemplate, SelectionModel,
};
use std::convert::TryFrom;

use super::account_switcher::item::{ExtraItemObj, Item as AccountSwitcherItem};

pub mod add_account;
pub mod item;
pub mod user_entry;

mod imp {
    use super::*;
    use glib::subclass::InitializingObject;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/sidebar-account-switcher.ui")]
    pub struct AccountSwitcher {
        #[template_child]
        pub entries: TemplateChild<gtk::ListView>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AccountSwitcher {
        const NAME: &'static str = "AccountSwitcher";
        type Type = super::AccountSwitcher;
        type ParentType = gtk::Popover;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for AccountSwitcher {
        fn constructed(&self, obj: &Self::Type) {
            self.parent_constructed(obj);

            self.entries.connect_activate(|list_view, index| {
                list_view
                    .model()
                    .and_then(|model| model.item(index))
                    .map(AccountSwitcherItem::try_from)
                    .and_then(Result::ok)
                    .map(|item| match item {
                        AccountSwitcherItem::User(session_page) => {
                            let session_widget = session_page.child();
                            session_widget
                                .parent()
                                .unwrap()
                                .downcast::<gtk::Stack>()
                                .unwrap()
                                .set_visible_child(&session_widget);
                        }
                        AccountSwitcherItem::AddAccount => {
                            list_view.activate_action("app.new-login", None);
                        }
                        _ => {}
                    });
            });

            // There is no permanent stuff to take care of,
            // so only bind and unbind are connected.
            let ref factory = gtk::SignalListItemFactory::new();
            factory.connect_bind(|_, list_item| {
                list_item.set_selectable(false);
                let child = list_item
                    .item()
                    .map(AccountSwitcherItem::try_from)
                    .and_then(Result::ok)
                    .as_ref()
                    .map(|item| {
                        match item {
                            AccountSwitcherItem::Separator => {
                                list_item.set_activatable(false);
                            }
                            _ => {}
                        }

                        item
                    })
                    .map(AccountSwitcherItem::build_widget);

                list_item.set_child(child.as_ref());
            });

            factory.connect_unbind(|_, list_item| {
                list_item.set_child(gtk::NONE_WIDGET);
            });

            self.entries.set_factory(Some(factory));
        }
    }

    impl WidgetImpl for AccountSwitcher {}
    impl PopoverImpl for AccountSwitcher {}
}

glib::wrapper! {
    pub struct AccountSwitcher(ObjectSubclass<imp::AccountSwitcher>)
        @extends gtk::Widget, gtk::Popover, @implements gtk::Accessible, gio::ListModel;
}

impl AccountSwitcher {
    pub fn set_logged_in_users(&self, sessions_stack_pages: &SelectionModel) {
        let entries = imp::AccountSwitcher::from_instance(self).entries.get();

        let ref end_items = ExtraItemObj::list_store();
        let ref items_split = ListStore::new(ListModel::static_type());
        items_split.append(sessions_stack_pages);
        items_split.append(end_items);
        let ref items = gtk::FlattenListModel::new(Some(items_split));
        let ref selectable_items = gtk::NoSelection::new(Some(items));

        entries.set_model(Some(selectable_items));
    }
}
