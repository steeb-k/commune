mod category_row;
mod icon_item_row;
mod room_row;
mod row;
mod verification_row;

use adw::{prelude::*, subclass::prelude::*};
use gtk::{gio, glib, glib::clone, CompositeTemplate};
use tracing::error;

use self::{
    category_row::CategoryRow, icon_item_row::IconItemRow, room_row::RoomRow, row::Row,
    verification_row::VerificationRow,
};
use crate::{
    account_switcher::AccountSwitcherButton,
    session::model::{
        Category, CategoryType, IconItem, IdentityVerification, Room, RoomType, Selection,
        SidebarListModel, User,
    },
};

mod imp {
    use std::{
        cell::{Cell, OnceCell, RefCell},
        marker::PhantomData,
    };

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session/view/sidebar/mod.ui")]
    #[properties(wrapper_type = super::Sidebar)]
    pub struct Sidebar {
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
        #[template_child]
        pub offline_banner: TemplateChild<adw::Banner>,
        pub room_row_popover: OnceCell<gtk::PopoverMenu>,
        /// The logged-in user.
        #[property(get, set = Self::set_user, explicit_notify, nullable)]
        pub user: RefCell<Option<User>>,
        /// The type of the source that activated drop mode.
        pub drop_source_type: Cell<Option<RoomType>>,
        /// The `CategoryType` of the source that activated drop mode.
        #[property(get = Self::drop_source_category_type, builder(CategoryType::default()))]
        pub drop_source_category_type: PhantomData<CategoryType>,
        /// The type of the drop target that is currently hovered.
        pub drop_active_target_type: Cell<Option<RoomType>>,
        /// The `CategoryType` of the drop target that is currently hovered.
        #[property(get = Self::drop_active_target_category_type, builder(CategoryType::default()))]
        pub drop_active_target_category_type: PhantomData<CategoryType>,
        /// The list model of this sidebar.
        #[property(get, set = Self::set_list_model, explicit_notify, nullable)]
        pub list_model: glib::WeakRef<SidebarListModel>,
        pub bindings: RefCell<Vec<glib::Binding>>,
        pub offline_handler_id: RefCell<Option<glib::SignalHandlerId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Sidebar {
        const NAME: &'static str = "Sidebar";
        type Type = super::Sidebar;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            AccountSwitcherButton::static_type();

            Self::bind_template(klass);
            klass.set_css_name("sidebar");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for Sidebar {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            let factory = gtk::SignalListItemFactory::new();
            factory.connect_setup(clone!(@weak obj => move |_, item| {
                let item = match item.downcast_ref::<gtk::ListItem>() {
                    Some(item) => item,
                    None => {
                        error!("List item factory did not receive a list item: {item:?}");
                        return;
                    }
                };
                let row = Row::new(&obj);
                item.set_child(Some(&row));
                item.bind_property("item", &row, "list-row").build();
            }));
            self.listview.set_factory(Some(&factory));

            self.listview.connect_activate(move |listview, pos| {
                let model: Option<Selection> = listview.model().and_downcast();
                let row: Option<gtk::TreeListRow> =
                    model.as_ref().and_then(|m| m.item(pos)).and_downcast();

                let (model, row) = match (model, row) {
                    (Some(model), Some(row)) => (model, row),
                    _ => return,
                };

                match row.item() {
                    Some(o) if o.is::<Category>() => row.set_expanded(!row.is_expanded()),
                    Some(o) if o.is::<Room>() => model.set_selected(pos),
                    Some(o) if o.is::<IconItem>() => model.set_selected(pos),
                    Some(o) if o.is::<IdentityVerification>() => model.set_selected(pos),
                    _ => {}
                }
            });

            // FIXME: Remove this hack once https://gitlab.gnome.org/GNOME/gtk/-/issues/4938 is resolved
            self.scrolled_window
                .vscrollbar()
                .first_child()
                .unwrap()
                .set_overflow(gtk::Overflow::Hidden);
        }
    }

    impl WidgetImpl for Sidebar {
        fn focus(&self, direction_type: gtk::DirectionType) -> bool {
            // WORKAROUND: This works around the tab behavior `gtk::ListViews have`
            // See: https://gitlab.gnome.org/GNOME/gtk/-/issues/4840
            let focus_child = self
                .obj()
                .focus_child()
                .and_then(|w| w.focus_child())
                .and_then(|w| w.focus_child());
            if focus_child.map_or(false, |w| w.is::<gtk::ListView>())
                && matches!(
                    direction_type,
                    gtk::DirectionType::TabForward | gtk::DirectionType::TabBackward
                )
            {
                false
            } else {
                self.parent_focus(direction_type)
            }
        }
    }

    impl NavigationPageImpl for Sidebar {}

    impl Sidebar {
        /// Set the logged-in user.
        fn set_user(&self, user: Option<User>) {
            let prev_user = self.user.borrow().clone();
            if prev_user == user {
                return;
            }

            if let Some(prev_user) = prev_user {
                if let Some(handler_id) = self.offline_handler_id.take() {
                    prev_user.session().disconnect(handler_id);
                }
            }

            if let Some(user) = &user {
                let session = user.session();
                let handler_id =
                    session.connect_offline_notify(clone!(@weak self as imp => move |session| {
                        imp.offline_banner.set_revealed(session.offline());
                    }));
                self.offline_banner.set_revealed(session.offline());

                self.offline_handler_id.replace(Some(handler_id));
            }

            self.user.replace(user);
            self.obj().notify_user();
        }

        /// Set the list model of the sidebar.
        fn set_list_model(&self, list_model: Option<SidebarListModel>) {
            if self.list_model.upgrade() == list_model {
                return;
            }
            let obj = self.obj();

            for binding in self.bindings.take() {
                binding.unbind();
            }

            if let Some(list_model) = &list_model {
                let bindings = vec![
                    obj.bind_property(
                        "drop-source-category-type",
                        &list_model.item_list(),
                        "show-all-for-category",
                    )
                    .sync_create()
                    .build(),
                    list_model
                        .string_filter()
                        .bind_property("search", &*self.room_search_entry, "text")
                        .sync_create()
                        .bidirectional()
                        .build(),
                ];

                self.bindings.replace(bindings);
            }

            self.listview
                .set_model(list_model.as_ref().map(|m| m.selection_model()).as_ref());
            self.list_model.set(list_model.as_ref());
            obj.notify_list_model();
        }

        /// The `CategoryType` of the source that activated drop mode.
        fn drop_source_category_type(&self) -> CategoryType {
            self.drop_source_type
                .get()
                .map(Into::into)
                .unwrap_or_default()
        }

        /// The `CategoryType` of the drop target that is currently hovered.
        fn drop_active_target_category_type(&self) -> CategoryType {
            self.drop_active_target_type
                .get()
                .map(Into::into)
                .unwrap_or_default()
        }
    }
}

glib::wrapper! {
    pub struct Sidebar(ObjectSubclass<imp::Sidebar>)
        @extends gtk::Widget, adw::NavigationPage, @implements gtk::Accessible;
}

impl Sidebar {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn room_search_bar(&self) -> gtk::SearchBar {
        self.imp().room_search.clone()
    }

    /// The type of the source that activated drop mode.
    pub fn drop_source_type(&self) -> Option<RoomType> {
        self.imp().drop_source_type.get()
    }

    /// Set the type of the source that activated drop mode.
    fn set_drop_source_type(&self, source_type: Option<RoomType>) {
        let imp = self.imp();

        if self.drop_source_type() == source_type {
            return;
        }

        imp.drop_source_type.set(source_type);

        if source_type.is_some() {
            imp.listview.add_css_class("drop-mode");
        } else {
            imp.listview.remove_css_class("drop-mode");
        }

        self.notify_drop_source_category_type();
    }

    /// The type of the drop target that is currently hovered.
    pub fn drop_active_target_type(&self) -> Option<RoomType> {
        self.imp().drop_active_target_type.get()
    }

    /// Set the type of the drop target that is currently hovered.
    fn set_drop_active_target_type(&self, target_type: Option<RoomType>) {
        if self.drop_active_target_type() == target_type {
            return;
        }

        self.imp().drop_active_target_type.set(target_type);
        self.notify_drop_active_target_category_type();
    }

    pub fn room_row_popover(&self) -> &gtk::PopoverMenu {
        let imp = self.imp();
        imp.room_row_popover.get_or_init(|| {
            gtk::PopoverMenu::builder()
                .menu_model(&*imp.room_row_menu)
                .has_arrow(false)
                .halign(gtk::Align::Start)
                .build()
        })
    }
}
