use gtk::{glib, prelude::*, subclass::prelude::*};
use ruma::thirdparty::ProtocolInstance;

mod imp {
    use std::cell::{OnceCell, RefCell};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::Server)]
    pub struct Server {
        /// The name of the server that is displayed in the list.
        #[property(get, construct_only)]
        pub name: OnceCell<String>,
        /// The ID of the network that is used during search.
        #[property(get, construct_only)]
        pub network: OnceCell<String>,
        /// The server name that is used during search.
        #[property(get, construct_only)]
        pub server: RefCell<Option<String>>,
        /// Whether this server can be deleted from the list.
        #[property(get, construct_only)]
        pub deletable: OnceCell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Server {
        const NAME: &'static str = "Server";
        type Type = super::Server;
    }

    #[glib::derived_properties]
    impl ObjectImpl for Server {}
}

glib::wrapper! {
    pub struct Server(ObjectSubclass<imp::Server>);
}

impl Server {
    pub fn with_default_server(name: &str) -> Self {
        glib::Object::builder()
            .property("name", name)
            .property("network", "matrix")
            .property("deletable", false)
            .build()
    }

    pub fn with_third_party_protocol(protocol_id: &str, instance: &ProtocolInstance) -> Self {
        let name = format!("{} ({protocol_id})", instance.desc);
        glib::Object::builder()
            .property("name", &name)
            .property("network", &instance.instance_id)
            .property("deletable", false)
            .build()
    }

    pub fn with_custom_matrix_server(server: &str) -> Self {
        glib::Object::builder()
            .property("name", server)
            .property("network", "matrix")
            .property("server", server)
            .property("deletable", true)
            .build()
    }
}
