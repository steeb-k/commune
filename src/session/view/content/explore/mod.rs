mod public_room;
mod public_room_list;
mod public_room_row;
mod server;
mod server_list;
mod server_row;
mod servers_popover;

use adw::subclass::prelude::*;
use gtk::{glib, glib::clone, prelude::*, CompositeTemplate};

pub use self::{
    public_room::PublicRoom, public_room_list::PublicRoomList, public_room_row::PublicRoomRow,
    servers_popover::ExploreServersPopover,
};
use self::{server::Server, server_list::ServerList, server_row::ExploreServerRow};
use crate::session::model::Session;

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session/view/content/explore/mod.ui")]
    #[properties(wrapper_type = super::Explore)]
    pub struct Explore {
        /// The current session.
        #[property(get, set = Self::set_session, explicit_notify)]
        pub session: glib::WeakRef<Session>,
        #[template_child]
        pub header_bar: TemplateChild<adw::HeaderBar>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub search_entry: TemplateChild<gtk::SearchEntry>,
        #[template_child]
        pub servers_button: TemplateChild<gtk::MenuButton>,
        #[template_child]
        pub servers_popover: TemplateChild<ExploreServersPopover>,
        #[template_child]
        pub listview: TemplateChild<gtk::ListView>,
        #[template_child]
        pub scrolled_window: TemplateChild<gtk::ScrolledWindow>,
        pub public_room_list: RefCell<Option<PublicRoomList>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Explore {
        const NAME: &'static str = "ContentExplore";
        type Type = super::Explore;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            PublicRoom::ensure_type();
            PublicRoomList::ensure_type();
            PublicRoomRow::ensure_type();

            Self::bind_template(klass);

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
            let obj = self.obj();
            let adj = self.scrolled_window.vadjustment();

            adj.connect_value_changed(clone!(
                #[weak(rename_to = imp)]
                self,
                move |adj| {
                    if adj.upper() - adj.value() < adj.page_size() * 2.0 {
                        if let Some(public_room_list) = &*imp.public_room_list.borrow() {
                            public_room_list.load_public_rooms(false);
                        }
                    }
                }
            ));

            self.search_entry.connect_search_changed(clone!(
                #[weak]
                obj,
                move |_| {
                    obj.trigger_search();
                }
            ));

            self.servers_popover.connect_selected_server_changed(clone!(
                #[weak]
                obj,
                move |_, server| {
                    if let Some(server) = server {
                        obj.imp().servers_button.set_label(&server.name());
                        obj.trigger_search();
                    }
                }
            ));
        }
    }

    impl WidgetImpl for Explore {
        fn grab_focus(&self) -> bool {
            self.search_entry.grab_focus()
        }
    }

    impl BinImpl for Explore {}

    impl Explore {
        /// Set the current session.
        fn set_session(&self, session: Option<&Session>) {
            if session == self.session.upgrade().as_ref() {
                return;
            }
            let obj = self.obj();

            if let Some(session) = session {
                let public_room_list = PublicRoomList::new(session);
                self.listview
                    .set_model(Some(&gtk::NoSelection::new(Some(public_room_list.clone()))));

                public_room_list.connect_loading_notify(clone!(
                    #[weak]
                    obj,
                    move |_| {
                        obj.update_visible_child();
                    }
                ));

                public_room_list.connect_empty_notify(clone!(
                    #[weak]
                    obj,
                    move |_| {
                        obj.update_visible_child();
                    }
                ));

                self.public_room_list.replace(Some(public_room_list));
                obj.update_visible_child();
            }

            self.session.set(session);
            obj.notify_session();
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

    pub fn init(&self) {
        let imp = self.imp();

        imp.servers_popover.init();

        if let Some(server) = imp.servers_popover.selected_server() {
            imp.servers_button.set_label(&server.name());
        }

        if let Some(public_room_list) = &*imp.public_room_list.borrow() {
            public_room_list.init();
        }
    }

    /// The header bar of the explorer.
    pub fn header_bar(&self) -> &adw::HeaderBar {
        &self.imp().header_bar
    }

    /// Update the visible child according to the current state.
    fn update_visible_child(&self) {
        let imp = self.imp();
        if let Some(public_room_list) = &*imp.public_room_list.borrow() {
            if public_room_list.loading() {
                imp.stack.set_visible_child_name("loading");
            } else if public_room_list.empty() {
                imp.stack.set_visible_child_name("empty");
            } else {
                imp.stack.set_visible_child_name("results");
            }
        }
    }

    fn trigger_search(&self) {
        let imp = self.imp();
        if let Some(public_room_list) = &*imp.public_room_list.borrow() {
            let text = imp.search_entry.text().into();
            let server = imp
                .servers_popover
                .selected_server()
                .expect("a server is selected");
            public_room_list.search(Some(text), &server);
        };
    }
}
