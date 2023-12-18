use adw::subclass::prelude::*;
use gtk::{
    glib,
    glib::{clone, FromVariant},
    prelude::*,
    CompositeTemplate,
};
use ruma::ServerName;

use super::{ExploreServerRow, Server, ServerList};
use crate::session::model::Session;

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session/view/content/explore/servers_popover.ui")]
    #[properties(wrapper_type = super::ExploreServersPopover)]
    pub struct ExploreServersPopover {
        /// The current session.
        #[property(get, set = Self::set_session, explicit_notify)]
        pub session: glib::WeakRef<Session>,
        /// The server list.
        #[property(get)]
        pub server_list: RefCell<Option<ServerList>>,
        #[template_child]
        pub listbox: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub server_entry: TemplateChild<gtk::Entry>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ExploreServersPopover {
        const NAME: &'static str = "ContentExploreServersPopover";
        type Type = super::ExploreServersPopover;
        type ParentType = gtk::Popover;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);

            klass.install_action(
                "explore-servers-popover.add-server",
                None,
                move |obj, _, _| {
                    obj.add_server();
                },
            );
            klass.install_action(
                "explore-servers-popover.remove-server",
                Some("s"),
                move |obj, _, variant| {
                    if let Some(variant) = variant.and_then(String::from_variant) {
                        obj.remove_server(&variant);
                    }
                },
            );
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for ExploreServersPopover {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            self.server_entry
                .connect_changed(clone!(@weak obj => move |_| {
                    obj.update_add_server_state()
                }));
            self.server_entry
                .connect_activate(clone!(@weak obj => move |_| {
                    obj.add_server()
                }));

            obj.update_add_server_state();
        }
    }

    impl WidgetImpl for ExploreServersPopover {}
    impl PopoverImpl for ExploreServersPopover {}

    impl ExploreServersPopover {
        /// Set the current session.
        fn set_session(&self, session: Option<Session>) {
            if session == self.session.upgrade() {
                return;
            }

            self.session.set(session.as_ref());
            self.obj().notify_session();
        }
    }
}

glib::wrapper! {
    /// A popover that lists the servers that can be explored.
    pub struct ExploreServersPopover(ObjectSubclass<imp::ExploreServersPopover>)
        @extends gtk::Widget, gtk::Popover, @implements gtk::Accessible;
}

impl ExploreServersPopover {
    pub fn new(session: &Session) -> Self {
        glib::Object::builder().property("session", session).build()
    }

    /// Initialize the list of servers.
    pub fn init(&self) {
        let Some(session) = &self.session() else {
            return;
        };

        let imp = self.imp();
        let server_list = ServerList::new(session);

        imp.listbox.bind_model(Some(&server_list), |obj| {
            ExploreServerRow::new(obj.downcast_ref::<Server>().unwrap()).upcast()
        });

        // Select the first server by default.
        imp.listbox.select_row(imp.listbox.row_at_index(0).as_ref());

        imp.server_list.replace(Some(server_list));
        self.notify_server_list();
    }

    /// The server that is currently selected, if any.
    pub fn selected_server(&self) -> Option<Server> {
        self.imp()
            .listbox
            .selected_row()
            .and_downcast::<ExploreServerRow>()
            .and_then(|row| row.server())
    }

    pub fn connect_selected_server_changed<F: Fn(&Self, Option<Server>) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.imp()
            .listbox
            .connect_row_selected(clone!(@weak self as obj => move |_, row| {
                f(&obj, row.and_then(|row| row.downcast_ref::<ExploreServerRow>()).and_then(|row| row.server()));
            }))
    }

    /// Whether the server currently in the text entry can be added.
    fn can_add_server(&self) -> bool {
        let server = self.imp().server_entry.text();
        ServerName::parse(server.as_str()).is_ok()
            // Don't allow duplicates
            && self
                .server_list()
                .filter(|l| !l.contains_matrix_server(&server))
                .is_some()
    }

    /// Update the state of the action to add a server according to the current
    /// state.
    fn update_add_server_state(&self) {
        self.action_set_enabled("explore-servers-popover.add-server", self.can_add_server())
    }

    /// Add the server currently in the text entry.
    fn add_server(&self) {
        if !self.can_add_server() {
            return;
        }
        let Some(server_list) = self.server_list() else {
            return;
        };

        let imp = self.imp();

        let server = imp.server_entry.text();
        imp.server_entry.set_text("");

        server_list.add_custom_matrix_server(server.into());
        imp.listbox.select_row(
            imp.listbox
                .row_at_index(server_list.n_items() as i32 - 1)
                .as_ref(),
        );
    }

    /// Remove the given server.
    fn remove_server(&self, server: &str) {
        let Some(server_list) = self.server_list() else {
            return;
        };

        let imp = self.imp();

        // If the selected server is gonna be removed, select the first one.
        if self.selected_server().and_then(|s| s.server()).as_deref() == Some(server) {
            imp.listbox.select_row(imp.listbox.row_at_index(0).as_ref());
        }

        server_list.remove_custom_matrix_server(server);
    }
}
