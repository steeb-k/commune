use adw::{prelude::*, subclass::prelude::*};
use gtk::{glib, glib::clone};

use crate::{
    session::Room,
    utils::{TemplateCallbacks, matrix::MatrixIdUri},
};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session_view/space.ui")]
    #[properties(wrapper_type = super::Space)]
    pub struct Space {
        #[template_child]
        pub(super) header_bar: TemplateChild<adw::HeaderBar>,
        #[template_child]
        room_topic: TemplateChild<gtk::Label>,
        /// The space currently displayed.
        #[property(get, set = Self::set_room, explicit_notify, nullable)]
        room: RefCell<Option<Room>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Space {
        const NAME: &'static str = "ContentSpace";
        type Type = super::Space;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            TemplateCallbacks::bind_template_callbacks(klass);

            klass.set_accessible_role(gtk::AccessibleRole::Group);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for Space {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            self.room_topic.connect_activate_link(clone!(
                #[weak]
                obj,
                #[upgrade_or]
                glib::Propagation::Proceed,
                move |_, uri| {
                    if MatrixIdUri::parse(uri).is_ok() {
                        let _ =
                            obj.activate_action("session.show-matrix-uri", Some(&uri.to_variant()));
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                }
            ));
        }
    }

    impl WidgetImpl for Space {}

    impl BinImpl for Space {}

    impl Space {
        /// Set the space currently displayed.
        fn set_room(&self, room: Option<Room>) {
            if *self.room.borrow() == room {
                return;
            }

            self.room.replace(room);
            self.obj().notify_room();
        }
    }
}

glib::wrapper! {
    /// A view presenting a space.
    ///
    /// A space is a room that groups other rooms together. Nothing here can
    /// browse the rooms it holds yet, but the space no longer falls through to
    /// the room history, where its timeline is always empty.
    pub struct Space(ObjectSubclass<imp::Space>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Space {
    /// The header bar of the space view.
    pub fn header_bar(&self) -> &adw::HeaderBar {
        &self.imp().header_bar
    }
}
