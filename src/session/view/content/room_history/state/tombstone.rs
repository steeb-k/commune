use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{glib, glib::clone, CompositeTemplate};

use crate::{session::model::Room, toast, utils::BoundObjectWeakRef, Window};

mod imp {
    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_history/state/tombstone.ui"
    )]
    #[properties(wrapper_type = super::StateTombstone)]
    pub struct StateTombstone {
        #[template_child]
        new_room_btn: TemplateChild<gtk::Button>,
        /// The [`Room`] this event belongs to.
        #[property(get, set = Self::set_room, construct_only)]
        room: BoundObjectWeakRef<Room>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for StateTombstone {
        const NAME: &'static str = "ContentStateTombstone";
        type Type = super::StateTombstone;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for StateTombstone {}

    impl WidgetImpl for StateTombstone {}
    impl BinImpl for StateTombstone {}

    #[gtk::template_callbacks]
    impl StateTombstone {
        /// Set the room this event belongs to.
        fn set_room(&self, room: &Room) {
            let successor_handler = room.connect_successor_id_string_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |room| {
                    imp.new_room_btn.set_visible(room.successor_id().is_some());
                }
            ));
            self.new_room_btn.set_visible(room.successor_id().is_some());

            let successor_room_handler = room.connect_successor_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |room| {
                    imp.update_button_label(room);
                }
            ));
            self.update_button_label(room);

            self.room
                .set(room, vec![successor_handler, successor_room_handler]);
        }

        /// Update the button of the label.
        fn update_button_label(&self, room: &Room) {
            let label = if room.successor().is_some() {
                // Translators: This is a verb, as in 'View Room'.
                gettext("View")
            } else {
                gettext("Join")
            };
            self.new_room_btn.set_label(&label);
        }

        /// Join or view the successor of this event's room.
        #[template_callback]
        async fn join_or_view_successor(&self) {
            let Some(room) = self.room.obj() else {
                return;
            };
            let Some(session) = room.session() else {
                return;
            };
            let room_list = session.room_list();
            let obj = self.obj();

            // Join or view the room with the given identifier.
            if let Some(successor) = room.successor() {
                let Some(window) = obj.root().and_downcast::<Window>() else {
                    return;
                };

                window.session_view().select_room(successor);
            } else if let Some(successor_id) = room.successor_id().map(ToOwned::to_owned) {
                let via = successor_id
                    .server_name()
                    .map(ToOwned::to_owned)
                    .into_iter()
                    .collect();

                if let Err(error) = room_list
                    .join_by_id_or_alias(successor_id.into(), via)
                    .await
                {
                    toast!(obj, error);
                }
            }
        }
    }
}

glib::wrapper! {
    /// A widget presenting a room tombstone state event.
    pub struct StateTombstone(ObjectSubclass<imp::StateTombstone>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl StateTombstone {
    /// Construct a new `StateTombstone` with the given room.
    pub fn new(room: &Room) -> Self {
        glib::Object::builder().property("room", room).build()
    }
}
