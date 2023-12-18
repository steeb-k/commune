use gtk::{glib, prelude::*, subclass::prelude::*, CompositeTemplate};

use super::server::Server;

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session/view/content/explore/server_row.ui")]
    #[properties(wrapper_type = super::ExploreServerRow)]
    pub struct ExploreServerRow {
        /// The server displayed by this row.
        #[property(get, construct_only)]
        pub server: RefCell<Option<Server>>,
        #[template_child]
        pub remove_button: TemplateChild<gtk::Button>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ExploreServerRow {
        const NAME: &'static str = "ExploreServerRow";
        type Type = super::ExploreServerRow;
        type ParentType = gtk::ListBoxRow;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for ExploreServerRow {
        fn constructed(&self) {
            self.parent_constructed();

            if let Some(server) = self.obj().server().and_then(|s| s.server()) {
                self.remove_button.set_action_target(Some(&server));
                self.remove_button
                    .set_action_name(Some("explore-servers-popover.remove-server"));
            }
        }
    }

    impl WidgetImpl for ExploreServerRow {}
    impl ListBoxRowImpl for ExploreServerRow {}
}

glib::wrapper! {
    /// A row representing a server to explore.
    pub struct ExploreServerRow(ObjectSubclass<imp::ExploreServerRow>)
        @extends gtk::Widget, gtk::ListBoxRow, @implements gtk::Accessible;
}

impl ExploreServerRow {
    pub fn new(server: &Server) -> Self {
        glib::Object::builder().property("server", server).build()
    }
}
