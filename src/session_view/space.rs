use adw::{prelude::*, subclass::prelude::*};
use gtk::{glib, glib::clone};
use tracing::error;

use super::explore::public_room_row::PublicRoomRow;
use crate::{
    session::{RemoteRoom, Room, SpaceChildren},
    utils::{LoadingState, TemplateCallbacks, matrix::MatrixIdUri},
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
        #[template_child]
        children_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        children_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        truncated_label: TemplateChild<gtk::Label>,
        /// The space currently displayed.
        #[property(get, set = Self::set_room, explicit_notify, nullable)]
        room: RefCell<Option<Room>>,
        /// The rooms inside the space currently displayed.
        children: SpaceChildren,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Space {
        const NAME: &'static str = "ContentSpace";
        type Type = super::Space;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);
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

            self.children_list
                .bind_model(Some(&self.children.list()), |item| {
                    let row = PublicRoomRow::new();

                    if let Some(room) = item.downcast_ref::<RemoteRoom>() {
                        row.set_room(room);
                    } else {
                        error!("Space children list contains something else than a room: {item:?}");
                    }

                    row.upcast()
                });

            self.children.list().connect_items_changed(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_, _, _, _| {
                    imp.update_children_stack();
                }
            ));
            self.children.connect_loading_state_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.update_children_stack();
                }
            ));
            self.children.connect_is_truncated_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |children| {
                    imp.truncated_label.set_visible(children.is_truncated());
                }
            ));

            self.update_children_stack();
        }
    }

    impl WidgetImpl for Space {}

    impl BinImpl for Space {}

    #[gtk::template_callbacks]
    impl Space {
        /// Set the space currently displayed.
        fn set_room(&self, room: Option<Room>) {
            if *self.room.borrow() == room {
                return;
            }

            if let Some(room) = &room
                && let Some(session) = room.session()
            {
                self.children.set_space(&session, room.room_id().to_owned());
            }

            self.room.replace(room);
            self.obj().notify_room();
        }

        /// List the rooms in this space again, after a failure.
        #[template_callback]
        fn retry_children(&self) {
            self.children.reload();
        }

        /// Update the page shown for the rooms inside this space.
        fn update_children_stack(&self) {
            let is_empty = self.children.list().n_items() == 0;

            let name = match self.children.loading_state() {
                // A space with a lot of rooms in it fills the list one batch at
                // a time, so show what has arrived rather than a spinner.
                LoadingState::Initial | LoadingState::Loading if is_empty => "loading",
                LoadingState::Error if is_empty => "error",
                _ if is_empty => "empty",
                _ => "list",
            };

            self.children_stack.set_visible_child_name(name);
        }
    }
}

glib::wrapper! {
    /// A view presenting a space.
    ///
    /// A space is a room that groups other rooms together, so this lists the
    /// rooms that are inside it, with the same row that the public directory
    /// uses: each one can be opened if it has been joined, and joined if it has
    /// not.
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
