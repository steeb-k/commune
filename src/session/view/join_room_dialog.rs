use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{gdk, glib, glib::clone, CompositeTemplate};
use ruma::{
    api::client::space::{get_hierarchy, SpaceHierarchyRoomsChunk},
    assign,
    matrix_uri::MatrixId,
    uint, MatrixToUri, MatrixUri, OwnedRoomOrAliasId, OwnedServerName, RoomOrAliasId,
};
use tracing::{debug, error};

use crate::{
    components::{Avatar, SpinnerButton, ToastableWindow},
    i18n::ngettext_f,
    prelude::*,
    session::model::{AvatarData, AvatarImage, AvatarUriSource, RoomIdentifier, Session},
    spawn, spawn_tokio, toast, Window,
};

mod imp {
    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session/view/join_room_dialog.ui")]
    #[properties(wrapper_type = super::JoinRoomDialog)]
    pub struct JoinRoomDialog {
        /// The current session.
        #[property(get, set = Self::set_session, construct_only)]
        pub session: glib::WeakRef<Session>,
        #[template_child]
        pub go_back_btn: TemplateChild<gtk::Button>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub entry_page: TemplateChild<gtk::Box>,
        #[template_child]
        pub search_entry: TemplateChild<gtk::SearchEntry>,
        #[template_child]
        pub look_up_btn: TemplateChild<SpinnerButton>,
        #[template_child]
        pub room_avatar: TemplateChild<Avatar>,
        #[template_child]
        pub room_name: TemplateChild<gtk::Label>,
        #[template_child]
        pub room_alias: TemplateChild<gtk::Label>,
        #[template_child]
        pub room_topic: TemplateChild<gtk::Label>,
        #[template_child]
        pub room_members_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub room_members_count: TemplateChild<gtk::Label>,
        #[template_child]
        pub join_btn: TemplateChild<SpinnerButton>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for JoinRoomDialog {
        const NAME: &'static str = "JoinRoomDialog";
        type Type = super::JoinRoomDialog;
        type ParentType = ToastableWindow;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);

            klass.add_binding(
                gdk::Key::Escape,
                gdk::ModifierType::empty(),
                |obj, _| {
                    obj.go_back();
                    true
                },
                None,
            );
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for JoinRoomDialog {}

    impl WidgetImpl for JoinRoomDialog {}
    impl WindowImpl for JoinRoomDialog {}
    impl AdwWindowImpl for JoinRoomDialog {}
    impl ToastableWindowImpl for JoinRoomDialog {}

    impl JoinRoomDialog {
        /// Set the current session.
        fn set_session(&self, session: Option<Session>) {
            self.session.set(session.as_ref());

            let obj = self.obj();
            obj.notify_session();
            obj.update_entry_page();
        }

        /// Whether we can go back to the previous screen.
        pub fn can_go_back(&self) -> bool {
            self.stack.visible_child_name().as_deref() == Some("details")
        }

        /// Set the currently visible page.
        pub fn set_visible_page(&self, page_name: &str) {
            self.stack.set_visible_child_name(page_name);
            self.go_back_btn.set_visible(self.can_go_back());
        }
    }
}

glib::wrapper! {
    /// Dialog to join a room.
    pub struct JoinRoomDialog(ObjectSubclass<imp::JoinRoomDialog>)
        @extends gtk::Widget, gtk::Window, adw::Window, ToastableWindow, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl JoinRoomDialog {
    pub fn new(parent_window: Option<&impl IsA<gtk::Window>>, session: &Session) -> Self {
        glib::Object::builder()
            .property("transient-for", parent_window)
            .property("session", session)
            .build()
    }

    /// Update the state of the entry page.
    #[template_callback]
    fn update_entry_page(&self) {
        let imp = self.imp();

        let Some(session) = self.session() else {
            imp.entry_page.set_sensitive(false);
            return;
        };
        imp.entry_page.set_sensitive(true);

        let Some((identifier, _)) = parse_room(&imp.search_entry.text()) else {
            imp.look_up_btn.set_sensitive(false);
            return;
        };
        imp.look_up_btn.set_sensitive(true);

        if session
            .room_list()
            .joined_room(&identifier.into())
            .is_some()
        {
            // Translators: This is a verb, as in 'View Room'.
            imp.look_up_btn.set_label(gettext("View"));
        } else {
            // Translators: This is a verb, as in 'Look up Room'.
            imp.look_up_btn.set_label(gettext("Look Up"));
        }
    }

    /// Look up the room that was entered, if it is valid.
    ///
    /// If the room is not, this will open it instead.
    #[template_callback]
    fn look_up_room(&self) {
        let Some(session) = self.session() else {
            return;
        };
        let imp = self.imp();

        let Some((identifier, _)) = parse_room(&imp.search_entry.text()) else {
            return;
        };
        let Some(window) = self.transient_for().and_downcast::<Window>() else {
            return;
        };

        imp.look_up_btn.set_loading(true);
        imp.entry_page.set_sensitive(false);

        // Join or view the room with the given identifier.
        let identifier = RoomIdentifier::from(identifier);
        if let Some(room) = session.room_list().joined_room(&identifier) {
            window.session_view().select_room(Some(room));
            self.close();
        } else {
            spawn!(clone!(@weak self as obj => async move {
                obj.look_up_room_inner(identifier).await;
            }));
        }
    }

    async fn look_up_room_inner(&self, identifier: RoomIdentifier) {
        let Some(session) = self.session() else {
            return;
        };
        let imp = self.imp();

        // Reset state before switching to possible pages.
        imp.go_back_btn.set_sensitive(true);
        imp.join_btn.set_loading(false);

        let client = session.client();

        let room_id = match identifier.clone() {
            RoomIdentifier::Id(room_id) => room_id,
            RoomIdentifier::Alias(alias) => {
                let client_clone = client.clone();
                let handle =
                    spawn_tokio!(async move { client_clone.resolve_room_alias(&alias).await });

                match handle.await.unwrap() {
                    Ok(response) => response.room_id,
                    Err(error) => {
                        error!("Failed to resolve room alias `{identifier}`: {error}");
                        self.fill_details_not_found(&identifier);
                        return;
                    }
                }
            }
        };

        // FIXME: The space hierarchy endpoint gives us the room details we want, but it
        // doesn't work if the room is not known by the homeserver. We need MSC3266 for
        // a proper endpoint.
        let request = assign!(get_hierarchy::v1::Request::new(room_id.clone()), {
            // We are only interested in the single room.
            limit: Some(uint!(1))
        });
        let handle = spawn_tokio!(async move { client.send(request, None).await });

        match handle.await.unwrap() {
            Ok(response) => {
                if let Some(chunk) = response
                    .rooms
                    .into_iter()
                    .next()
                    .filter(|c| c.room_id == room_id)
                {
                    self.fill_details_found(&session, chunk);
                    return;
                } else {
                    debug!("Endpoint did not return requested room");
                }
            }
            Err(error) => {
                error!("Failed to get room details for room `{identifier}`: {error}");
            }
        }

        self.fill_details_not_found(&identifier);
    }

    /// Fill the details with the given result.
    fn fill_details_found(&self, session: &Session, chunk: SpaceHierarchyRoomsChunk) {
        let imp = self.imp();

        let name = if let Some(name) = chunk.name {
            if let Some(alias) = chunk.canonical_alias {
                imp.room_alias.set_label(alias.as_str());
                imp.room_alias.set_visible(true);
            }

            name
        } else if let Some(alias) = chunk.canonical_alias {
            imp.room_alias.set_visible(false);

            alias.to_string()
        } else {
            imp.room_alias.set_visible(false);

            chunk.room_id.to_string()
        };
        imp.room_name.set_label(&name);

        let avatar_data = AvatarData::new();
        avatar_data.set_display_name(Some(name));

        if let Some(avatar_url) = chunk.avatar_url {
            let image = AvatarImage::new(session, Some(&avatar_url), AvatarUriSource::Room);
            avatar_data.set_image(Some(image));
        }

        imp.room_avatar.set_data(Some(avatar_data));

        if let Some(topic) = chunk.topic {
            imp.room_topic.set_label(&topic);
            imp.room_topic.set_visible(true);
        } else {
            imp.room_topic.set_visible(false);
        }

        let members_count = u32::try_from(chunk.num_joined_members).unwrap_or(u32::MAX);
        imp.room_members_count.set_label(&members_count.to_string());

        let members_tooltip = ngettext_f(
            // Translators: Do NOT translate the content between '{' and '}',
            // this is a variable name.
            "1 member",
            "{n} members",
            members_count,
            &[("n", &members_count.to_string())],
        );
        imp.room_members_box
            .set_tooltip_text(Some(&members_tooltip));
        imp.room_members_box.set_visible(true);

        imp.set_visible_page("details");
    }

    /// Fill the details when no result is available
    fn fill_details_not_found(&self, identifier: &RoomIdentifier) {
        let imp = self.imp();

        let name = identifier.to_string();
        imp.room_name.set_label(&name);

        let avatar_data = AvatarData::new();
        avatar_data.set_display_name(Some(name));
        imp.room_avatar.set_data(Some(avatar_data));

        imp.room_topic.set_label(&gettext(
            "The room details cannot be previewed. It can be because the room is not known by the homeserver or because its details are private. You can still try to join it."
        ));
        imp.room_topic.set_visible(true);

        imp.room_alias.set_visible(false);
        imp.room_members_box.set_visible(false);

        imp.set_visible_page("details");
    }

    /// Join the room that was entered, if it is valid.
    #[template_callback]
    fn join_room(&self) {
        let Some((room_id, via)) = parse_room(&self.imp().search_entry.text()) else {
            return;
        };
        let Some(session) = self.session() else {
            return;
        };
        let imp = self.imp();

        imp.go_back_btn.set_sensitive(false);
        imp.join_btn.set_loading(true);

        // Join the room with the given identifier.
        let room_list = session.room_list();
        spawn!(clone!(@weak self as obj, @weak room_list => async move {
            match room_list.join_by_id_or_alias(room_id, via).await {
                Ok(room_id) => {
                    if let Some(room) = room_list.get_wait(&room_id).await {
                        if let Some(window) = obj.transient_for().and_downcast_ref::<Window>() {
                            window.session_view().select_room(Some(room));
                        }
                    }

                    obj.close();
                }
                Err(error) => {
                    toast!(obj, error);

                    let imp = obj.imp();
                    imp.join_btn.set_loading(false);
                    imp.go_back_btn.set_sensitive(true);
                }
            }
        }));
    }

    /// Go back to the previous screen.
    ///
    /// If we can't go back, closes the window.
    #[template_callback]
    fn go_back(&self) {
        let imp = self.imp();

        if imp.can_go_back() {
            // There is only one screen to go back to.
            imp.look_up_btn.set_loading(false);
            imp.entry_page.set_sensitive(true);
            imp.set_visible_page("entry");
        } else {
            self.close()
        }
    }
}

fn parse_room(room: &str) -> Option<(OwnedRoomOrAliasId, Vec<OwnedServerName>)> {
    MatrixUri::parse(room)
        .ok()
        .and_then(|uri| match uri.id() {
            MatrixId::Room(room_id) => Some((room_id.clone().into(), uri.via().to_owned())),
            MatrixId::RoomAlias(room_alias) => {
                Some((room_alias.clone().into(), uri.via().to_owned()))
            }
            _ => None,
        })
        .or_else(|| {
            MatrixToUri::parse(room)
                .ok()
                .and_then(|uri| match uri.id() {
                    MatrixId::Room(room_id) => Some((room_id.clone().into(), uri.via().to_owned())),
                    MatrixId::RoomAlias(room_alias) => {
                        Some((room_alias.clone().into(), uri.via().to_owned()))
                    }
                    _ => None,
                })
        })
        .or_else(|| {
            RoomOrAliasId::parse(room)
                .ok()
                .map(|room_id| (room_id, vec![]))
        })
}
