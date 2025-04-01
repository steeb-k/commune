use adw::{prelude::*, subclass::prelude::*};
use gtk::{gio, glib, glib::clone, CompositeTemplate};
use tracing::error;

mod public_room;
mod public_room_list;
mod public_room_row;
mod server;
mod server_list;
mod server_row;
mod servers_popover;

pub use self::{
    public_room::PublicRoom, public_room_list::PublicRoomList, public_room_row::PublicRoomRow,
    servers_popover::ExploreServersPopover,
};
use self::{server::ExploreServer, server_list::ExploreServerList, server_row::ExploreServerRow};
use crate::{
    components::LoadingRow,
    session::model::Session,
    utils::{BoundObject, LoadingState},
};

mod imp {
    use std::cell::OnceCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session/view/content/explore/mod.ui")]
    #[properties(wrapper_type = super::Explore)]
    pub struct Explore {
        #[template_child]
        pub(super) header_bar: TemplateChild<adw::HeaderBar>,
        #[template_child]
        stack: TemplateChild<gtk::Stack>,
        #[template_child]
        search_entry: TemplateChild<gtk::SearchEntry>,
        #[template_child]
        servers_button: TemplateChild<gtk::MenuButton>,
        #[template_child]
        servers_popover: TemplateChild<ExploreServersPopover>,
        #[template_child]
        listview: TemplateChild<gtk::ListView>,
        #[template_child]
        scrolled_window: TemplateChild<gtk::ScrolledWindow>,
        /// The current session.
        #[property(get, set = Self::set_session, explicit_notify)]
        session: glib::WeakRef<Session>,
        /// The list of public rooms.
        public_room_list: BoundObject<PublicRoomList>,
        /// The items added at the end of the list.
        end_items: OnceCell<gio::ListStore>,
        /// The full list model.
        full_model: OnceCell<gio::ListStore>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Explore {
        const NAME: &'static str = "ContentExplore";
        type Type = super::Explore;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            PublicRoom::ensure_type();
            PublicRoomRow::ensure_type();

            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);

            klass.set_accessible_role(gtk::AccessibleRole::Group);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for Explore {
        fn constructed(&self) {
            self.parent_constructed();

            self.servers_popover.connect_selected_server_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.server_changed();
                }
            ));

            let adj = self.scrolled_window.vadjustment();
            adj.connect_value_changed(clone!(
                #[weak(rename_to = imp)]
                self,
                move |adj| {
                    if adj.upper() - adj.value() < adj.page_size() * 2.0 {
                        if let Some(public_room_list) = imp.public_room_list.obj() {
                            public_room_list.load_more();
                        }
                    }
                }
            ));

            let factory = gtk::SignalListItemFactory::new();
            factory.connect_bind(move |_, list_item| {
                let Some(list_item) = list_item.downcast_ref::<gtk::ListItem>() else {
                    error!("List item factory did not receive a list item: {list_item:?}");
                    return;
                };
                list_item.set_activatable(false);
                list_item.set_selectable(false);

                let Some(item) = list_item.item() else {
                    return;
                };

                if let Some(public_room) = item.downcast_ref::<PublicRoom>() {
                    let public_room_row = if let Some(public_room_row) =
                        list_item.child().and_downcast::<PublicRoomRow>()
                    {
                        public_room_row
                    } else {
                        let public_room_row = PublicRoomRow::new();
                        list_item.set_child(Some(&public_room_row));
                        public_room_row
                    };

                    public_room_row.set_public_room(public_room);
                } else if let Some(loading_row) = item.downcast_ref::<LoadingRow>() {
                    list_item.set_child(Some(loading_row));
                }
            });
            self.listview.set_factory(Some(&factory));

            let flattened_model = gtk::FlattenListModel::new(Some(self.full_model().clone()));
            self.listview
                .set_model(Some(&gtk::NoSelection::new(Some(flattened_model))));
        }
    }

    impl WidgetImpl for Explore {
        fn grab_focus(&self) -> bool {
            self.search_entry.grab_focus()
        }
    }

    impl BinImpl for Explore {}

    #[gtk::template_callbacks]
    impl Explore {
        /// Set the current session.
        fn set_session(&self, session: Option<&Session>) {
            if self.session.upgrade().as_ref() == session {
                return;
            }

            self.public_room_list.disconnect_signals();

            if let Some(session) = session {
                let public_room_list = PublicRoomList::new(session);

                let full_model = self.full_model();
                if full_model.n_items() == 2 {
                    full_model.splice(0, 1, &[public_room_list.clone()]);
                } else {
                    full_model.insert(0, &public_room_list);
                }

                let loading_state_handler = public_room_list.connect_loading_state_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_visible_child();
                    }
                ));

                let items_changed_handler = public_room_list.connect_items_changed(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_, _, _, _| {
                        imp.update_visible_child();
                    }
                ));

                self.public_room_list.set(
                    public_room_list,
                    vec![loading_state_handler, items_changed_handler],
                );
                self.update_visible_child();
            }

            self.session.set(session);
            self.obj().notify_session();
        }

        /// The items added at the end of the list.
        fn end_items(&self) -> &gio::ListStore {
            self.end_items
                .get_or_init(gio::ListStore::new::<LoadingRow>)
        }

        /// The full list model.
        fn full_model(&self) -> &gio::ListStore {
            self.full_model.get_or_init(|| {
                let model = gio::ListStore::new::<gio::ListModel>();
                model.append(self.end_items());
                model
            })
        }

        /// Make sure that the view is initialized.
        ///
        /// If it is already initialized, this is a noop.
        pub(super) fn init(&self) {
            self.servers_popover.load();
        }

        /// Update the visible child according to the current state.
        fn update_visible_child(&self) {
            let Some(public_room_list) = self.public_room_list.obj() else {
                return;
            };

            let loading_state = public_room_list.loading_state();
            let is_empty = public_room_list.is_empty();

            // Create or remove the loading row, as needed.
            let end_items = self.end_items();
            if matches!(loading_state, LoadingState::Loading) && !is_empty {
                if end_items.n_items() == 0 {
                    // We need a loading row.
                    end_items.append(&LoadingRow::new());
                }
            } else if end_items.n_items() > 0 {
                // We do not need a loading row.
                end_items.remove(0);
            }

            // Update the visible page.
            let page_name = match loading_state {
                LoadingState::Initial | LoadingState::Loading => {
                    if is_empty {
                        "loading"
                    } else {
                        "results"
                    }
                }
                LoadingState::Ready => {
                    if is_empty {
                        "empty"
                    } else {
                        "results"
                    }
                }
                LoadingState::Error => "error",
            };
            self.stack.set_visible_child_name(page_name);
        }

        /// Trigger a search with the current term.
        #[template_callback]
        fn trigger_search(&self) {
            let Some(public_room_list) = self.public_room_list.obj() else {
                return;
            };

            let text = self.search_entry.text().into();
            let server = self
                .servers_popover
                .selected_server()
                .expect("a server should be selected");
            public_room_list.search(Some(text), &server);
        }

        /// Handle when the selected server changed.
        fn server_changed(&self) {
            if let Some(server) = self.servers_popover.selected_server() {
                self.servers_button.set_label(&server.name());
                self.trigger_search();
            }
        }
    }
}

glib::wrapper! {
    /// A view to explore rooms in the public directory of homeservers.
    pub struct Explore(ObjectSubclass<imp::Explore>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl Explore {
    pub fn new(session: &Session) -> Self {
        glib::Object::builder().property("session", session).build()
    }

    /// Make sure that the view is initialized.
    ///
    /// If it is already initialized, this is a noop.
    pub(crate) fn init(&self) {
        self.imp().init();
    }

    /// The header bar of the explorer.
    pub(crate) fn header_bar(&self) -> &adw::HeaderBar {
        &self.imp().header_bar
    }
}
