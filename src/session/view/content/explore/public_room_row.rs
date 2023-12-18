use adw::{prelude::BinExt, subclass::prelude::BinImpl};
use gettextrs::gettext;
use gtk::{glib, glib::clone, prelude::*, subclass::prelude::*, CompositeTemplate};
use ruma::ServerName;

use super::PublicRoom;
use crate::{
    components::{Avatar, Spinner, SpinnerButton},
    prelude::*,
    spawn, toast,
    utils::BoundObject,
    Window,
};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session/view/content/explore/public_room_row.ui")]
    #[properties(wrapper_type = super::PublicRoomRow)]
    pub struct PublicRoomRow {
        /// The public room displayed by this row.
        #[property(get, set= Self::set_public_room, explicit_notify)]
        pub public_room: BoundObject<PublicRoom>,
        #[template_child]
        pub avatar: TemplateChild<Avatar>,
        #[template_child]
        pub display_name: TemplateChild<gtk::Label>,
        #[template_child]
        pub description: TemplateChild<gtk::Label>,
        #[template_child]
        pub alias: TemplateChild<gtk::Label>,
        #[template_child]
        pub members_count: TemplateChild<gtk::Label>,
        #[template_child]
        pub button: TemplateChild<SpinnerButton>,
        pub original_child: RefCell<Option<gtk::Widget>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PublicRoomRow {
        const NAME: &'static str = "ContentPublicRoomRow";
        type Type = super::PublicRoomRow;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for PublicRoomRow {
        fn constructed(&self) {
            self.parent_constructed();
            self.button
                .connect_clicked(clone!(@weak self as imp => move |_| {
                    imp.obj().join_or_view();
                }));
        }
    }

    impl WidgetImpl for PublicRoomRow {}
    impl BinImpl for PublicRoomRow {}

    impl PublicRoomRow {
        /// Set the public room displayed by this row.
        fn set_public_room(&self, public_room: Option<PublicRoom>) {
            if self.public_room.obj() == public_room {
                return;
            }
            let obj = self.obj();

            self.public_room.disconnect_signals();

            if let Some(public_room) = public_room {
                if let Some(child) = self.original_child.take() {
                    obj.set_child(Some(&child));
                }
                if let Some(matrix_public_room) = public_room.matrix_public_room() {
                    self.avatar
                        .set_data(Some(public_room.avatar_data().clone()));

                    let display_name = matrix_public_room
                        .name
                        .as_deref()
                        // FIXME: display some other identification for this room
                        .unwrap_or("Room without name");
                    self.display_name.set_text(display_name);

                    if let Some(topic) = &matrix_public_room.topic {
                        self.description.set_text(topic);
                    }
                    self.description
                        .set_visible(matrix_public_room.topic.is_some());

                    if let Some(alias) = &matrix_public_room.canonical_alias {
                        self.alias.set_text(alias.as_str());
                    }
                    self.alias
                        .set_visible(matrix_public_room.canonical_alias.is_some());

                    self.members_count
                        .set_text(&matrix_public_room.num_joined_members.to_string());

                    let pending_handler = public_room.connect_pending_notify(
                        clone!(@weak obj => move |public_room| {
                                obj.update_button(public_room);
                        }),
                    );

                    let room_handler =
                        public_room.connect_room_notify(clone!(@weak obj => move |public_room| {
                            obj.update_button(public_room);
                        }));

                    obj.update_button(&public_room);
                    self.public_room
                        .set(public_room, vec![pending_handler, room_handler]);
                } else if self.original_child.borrow().is_none() {
                    let spinner = Spinner::default();
                    spinner.set_margin_top(12);
                    spinner.set_margin_bottom(12);
                    self.original_child.replace(obj.child());
                    obj.set_child(Some(&spinner));
                }
            }

            obj.notify_public_room();
        }
    }
}

glib::wrapper! {
    /// A row representing a room for a homeserver's public directory.
    pub struct PublicRoomRow(ObjectSubclass<imp::PublicRoomRow>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl PublicRoomRow {
    pub fn new() -> Self {
        glib::Object::new()
    }

    fn update_button(&self, public_room: &PublicRoom) {
        let button = &self.imp().button;
        if public_room.room().is_some() {
            // Translators: This is a verb, as in 'View Room'.
            button.set_label(gettext("View"));
        } else {
            button.set_label(gettext("Join"));
        }

        button.set_loading(public_room.pending());
    }

    /// Join or view the public room.
    pub fn join_or_view(&self) {
        let Some(public_room) = self.public_room() else {
            return;
        };
        let room_list = public_room.room_list();
        let Some(session) = room_list.session() else {
            return;
        };

        if let Some(room) = public_room.room() {
            if let Some(window) = self.root().and_downcast::<Window>() {
                window.show_room(session.session_id(), room.room_id());
            }
        } else if let Some(matrix_public_room) = public_room.matrix_public_room() {
            // Prefer the alias as we are sure the server can find the room that way.
            let (room_id, via) = matrix_public_room
                .canonical_alias
                .clone()
                .map(|id| (id.into(), vec![]))
                .unwrap_or_else(|| {
                    let id = matrix_public_room.room_id.clone().into();
                    let via = ServerName::parse(public_room.server())
                        .ok()
                        .into_iter()
                        .collect();
                    (id, via)
                });

            spawn!(clone!(@weak self as obj, @weak room_list => async move {
                if let Err(error) = room_list.join_by_id_or_alias(room_id, via).await {
                    toast!(obj, error);
                }
            }));
        }
    }
}
