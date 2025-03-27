use adw::{prelude::BinExt, subclass::prelude::BinImpl};
use gettextrs::gettext;
use gtk::{glib, glib::clone, prelude::*, subclass::prelude::*, CompositeTemplate};
use ruma::ServerName;

use super::PublicRoom;
use crate::{
    components::{Avatar, LoadingButton},
    gettext_f, ngettext_f,
    prelude::*,
    spawn, toast,
    utils::{matrix::MatrixIdUri, string::linkify, BoundObject},
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
        pub members_count_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub button: TemplateChild<LoadingButton>,
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
            let obj = self.obj();

            self.button.connect_clicked(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.obj().join_or_view();
                }
            ));

            self.description.connect_activate_link(clone!(
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

                if public_room.matrix_public_room().is_some() {
                    let pending_handler = public_room.connect_pending_notify(clone!(
                        #[weak(rename_to = imp)]
                        self,
                        move |_| {
                            imp.update_button();
                        }
                    ));
                    let room_handler = public_room.connect_room_notify(clone!(
                        #[weak(rename_to = imp)]
                        self,
                        move |_| {
                            imp.update_button();
                        }
                    ));

                    self.public_room
                        .set(public_room, vec![pending_handler, room_handler]);

                    self.update_button();
                    self.update_row();
                } else if self.original_child.borrow().is_none() {
                    let spinner = adw::Spinner::new();
                    spinner.set_margin_top(12);
                    spinner.set_margin_bottom(12);
                    spinner.set_width_request(24);
                    spinner.set_height_request(24);
                    self.original_child.replace(obj.child());
                    obj.set_child(Some(&spinner));
                }
            }

            obj.notify_public_room();
        }

        /// Update this row for the current state.
        fn update_row(&self) {
            let Some(public_room) = self.public_room.obj() else {
                return;
            };
            let Some(matrix_public_room) = public_room.matrix_public_room() else {
                return;
            };

            self.avatar.set_data(Some(public_room.avatar_data()));

            self.display_name.set_text(&public_room.display_name());

            if let Some(topic) = &matrix_public_room.topic {
                // Detect links.
                let mut t = linkify(topic);
                // Remove trailing spaces.
                t.truncate_end_whitespaces();
                self.description.set_label(&t);
            }
            self.description
                .set_visible(matrix_public_room.topic.is_some());

            if let Some(alias) = &matrix_public_room.canonical_alias {
                self.alias.set_text(alias.as_str());
            }
            self.alias
                .set_visible(matrix_public_room.canonical_alias.is_some());

            let members_count =
                u32::try_from(matrix_public_room.num_joined_members).unwrap_or(u32::MAX);
            self.members_count.set_text(&members_count.to_string());
            let members_count_tooltip = ngettext_f(
                // Translators: Do NOT translate the content between '{' and '}',
                // this is a variable name.
                "1 member",
                "{n} members",
                members_count,
                &[("n", &members_count.to_string())],
            );
            self.members_count_box
                .set_tooltip_text(Some(&members_count_tooltip));
        }

        /// Update the join/view button of this row.
        fn update_button(&self) {
            let Some(public_room) = self.public_room.obj() else {
                return;
            };

            let room_joined = public_room.room().is_some();

            let label = if room_joined {
                // Translators: This is a verb, as in 'View Room'.
                gettext("View")
            } else {
                gettext("Join")
            };
            self.button.set_content_label(label);

            let room_name = public_room.display_name();
            let accessible_desc = if room_joined {
                gettext_f("View {room_name}", &[("room_name", &room_name)])
            } else {
                gettext_f("Join {room_name}", &[("room_name", &room_name)])
            };
            self.button
                .update_property(&[gtk::accessible::Property::Description(&accessible_desc)]);

            self.button.set_is_loading(public_room.pending());
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

    /// Join or view the public room.
    pub fn join_or_view(&self) {
        let Some(public_room) = self.public_room() else {
            return;
        };

        if let Some(room) = public_room.room() {
            if let Some(window) = self.root().and_downcast::<Window>() {
                window.session_view().select_room(room);
            }
        } else if let Some(matrix_public_room) = public_room.matrix_public_room() {
            // Prefer the alias as we are sure the server can find the room that way.
            let (room_id, via) = matrix_public_room.canonical_alias.clone().map_or_else(
                || {
                    let id = matrix_public_room.room_id.clone().into();
                    let via = ServerName::parse(public_room.server())
                        .ok()
                        .into_iter()
                        .collect();
                    (id, via)
                },
                |id| (id.into(), vec![]),
            );

            let room_list = public_room.room_list();
            spawn!(clone!(
                #[weak(rename_to = obj)]
                self,
                async move {
                    if let Err(error) = room_list.join_by_id_or_alias(room_id, via).await {
                        toast!(obj, error);
                    }
                }
            ));
        }
    }
}
