use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{glib, glib::clone, CompositeTemplate};

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
    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session/view/content/explore/public_room_row.ui")]
    #[properties(wrapper_type = super::PublicRoomRow)]
    pub struct PublicRoomRow {
        #[template_child]
        avatar: TemplateChild<Avatar>,
        #[template_child]
        display_name: TemplateChild<gtk::Label>,
        #[template_child]
        description: TemplateChild<gtk::Label>,
        #[template_child]
        alias: TemplateChild<gtk::Label>,
        #[template_child]
        members_count: TemplateChild<gtk::Label>,
        #[template_child]
        members_count_box: TemplateChild<gtk::Box>,
        #[template_child]
        button: TemplateChild<LoadingButton>,
        /// The public room displayed by this row.
        #[property(get, set= Self::set_public_room, explicit_notify)]
        public_room: BoundObject<PublicRoom>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PublicRoomRow {
        const NAME: &'static str = "PublicRoomRow";
        type Type = super::PublicRoomRow;
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
    impl ObjectImpl for PublicRoomRow {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

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

    #[gtk::template_callbacks]
    impl PublicRoomRow {
        /// Set the public room displayed by this row.
        fn set_public_room(&self, public_room: Option<PublicRoom>) {
            if self.public_room.obj() == public_room {
                return;
            }

            self.public_room.disconnect_signals();

            if let Some(public_room) = public_room {
                let pending_handler = public_room.connect_is_pending_notify(clone!(
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
            }

            self.obj().notify_public_room();
        }

        /// Update this row for the current state.
        fn update_row(&self) {
            let Some(public_room) = self.public_room.obj() else {
                return;
            };

            self.avatar.set_data(Some(public_room.avatar_data()));
            self.display_name.set_text(&public_room.display_name());

            let data = public_room.data();

            if let Some(topic) = &data.topic {
                // Detect links.
                let mut t = linkify(topic);
                // Remove trailing spaces.
                t.truncate_end_whitespaces();

                self.description.set_label(&t);
                self.description.set_visible(!t.is_empty());
            } else {
                self.description.set_visible(false);
            }

            if let Some(alias) = &data.canonical_alias {
                self.alias.set_text(alias.as_str());
            }
            self.alias.set_visible(data.canonical_alias.is_some());

            let members_count = u32::try_from(data.num_joined_members).unwrap_or(u32::MAX);
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

            self.button.set_is_loading(public_room.is_pending());
        }

        /// Join or view the public room.
        #[template_callback]
        fn join_or_view(&self) {
            let Some(public_room) = self.public_room.obj() else {
                return;
            };

            if let Some(room) = public_room.room() {
                if let Some(window) = self.obj().root().and_downcast::<Window>() {
                    window.session_view().select_room(room);
                }
            } else {
                let data = public_room.data();

                // Prefer the alias as we are sure the server can find the room that way.
                let (room_id, via) = data.canonical_alias.clone().map_or_else(
                    || {
                        let id = data.room_id.clone().into();
                        let via = public_room.server().cloned().into_iter().collect();
                        (id, via)
                    },
                    |id| (id.into(), vec![]),
                );

                let obj = self.obj();
                let room_list = public_room.room_list();
                spawn!(clone!(
                    #[weak]
                    obj,
                    async move {
                        if let Err(error) = room_list.join_by_id_or_alias(room_id, via).await {
                            toast!(obj, error);
                        }
                    }
                ));
            }
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
}

impl Default for PublicRoomRow {
    fn default() -> Self {
        Self::new()
    }
}
