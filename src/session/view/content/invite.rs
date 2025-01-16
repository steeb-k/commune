use adw::subclass::prelude::*;
use gettextrs::gettext;
use gtk::{glib, glib::clone, prelude::*, CompositeTemplate};

use crate::{
    components::{confirm_leave_room_dialog, Avatar, LabelWithWidgets, LoadingButton, Pill},
    gettext_f,
    prelude::*,
    session::model::{MemberList, Room, RoomCategory, TargetRoomCategory, User},
    toast,
    utils::matrix::MatrixIdUri,
};

mod imp {
    use std::{cell::RefCell, collections::HashSet};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session/view/content/invite.ui")]
    #[properties(wrapper_type = super::Invite)]
    pub struct Invite {
        /// The room currently displayed.
        #[property(get, set = Self::set_room, explicit_notify, nullable)]
        pub room: RefCell<Option<Room>>,
        pub room_members: RefCell<Option<MemberList>>,
        pub accept_requests: RefCell<HashSet<Room>>,
        pub decline_requests: RefCell<HashSet<Room>>,
        pub category_handler: RefCell<Option<glib::SignalHandlerId>>,
        #[template_child]
        pub header_bar: TemplateChild<adw::HeaderBar>,
        #[template_child]
        pub room_alias: TemplateChild<gtk::Label>,
        #[template_child]
        pub room_topic: TemplateChild<gtk::Label>,
        #[template_child]
        pub inviter: TemplateChild<LabelWithWidgets>,
        #[template_child]
        pub accept_button: TemplateChild<LoadingButton>,
        #[template_child]
        pub decline_button: TemplateChild<LoadingButton>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Invite {
        const NAME: &'static str = "ContentInvite";
        type Type = super::Invite;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Pill::ensure_type();
            Avatar::ensure_type();

            Self::bind_template(klass);

            klass.set_accessible_role(gtk::AccessibleRole::Group);

            klass.install_action_async("invite.decline", None, move |widget, _, _| async move {
                widget.decline().await;
            });
            klass.install_action_async("invite.accept", None, move |widget, _, _| async move {
                widget.accept().await;
            });
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for Invite {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            self.room_alias.connect_label_notify(|room_alias| {
                room_alias.set_visible(!room_alias.label().is_empty());
            });
            self.room_alias
                .set_visible(!self.room_alias.label().is_empty());

            self.room_topic.connect_label_notify(|room_topic| {
                room_topic.set_visible(!room_topic.label().is_empty());
            });
            self.room_topic
                .set_visible(!self.room_topic.label().is_empty());
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

        fn dispose(&self) {
            if let Some(room) = self.room.take() {
                if let Some(handler) = self.category_handler.take() {
                    room.disconnect(handler);
                }
            }
        }
    }

    impl WidgetImpl for Invite {}
    impl BinImpl for Invite {}

    impl Invite {
        /// Set the room currently displayed.
        fn set_room(&self, room: Option<Room>) {
            if *self.room.borrow() == room {
                return;
            }
            let obj = self.obj();

            match &room {
                Some(room) if self.accept_requests.borrow().contains(room) => {
                    obj.action_set_enabled("invite.accept", false);
                    obj.action_set_enabled("invite.decline", false);
                    self.accept_button.set_is_loading(true);
                }
                Some(room) if self.decline_requests.borrow().contains(room) => {
                    obj.action_set_enabled("invite.accept", false);
                    obj.action_set_enabled("invite.decline", false);
                    self.decline_button.set_is_loading(true);
                }
                _ => obj.reset(),
            }

            if let Some(room) = self.room.take() {
                if let Some(handler) = self.category_handler.take() {
                    room.disconnect(handler);
                }
            }

            if let Some(room) = &room {
                let category_handler = room.connect_category_notify(clone!(
                    #[weak]
                    obj,
                    move |room| {
                        let category = room.category();

                        if category == RoomCategory::Left {
                            // We declined the invite or the invite was retracted, we should close
                            // the room if it is opened.
                            let Some(session) = room.session() else {
                                return;
                            };
                            let selection = session.sidebar_list_model().selection_model();
                            if let Some(selected_room) =
                                selection.selected_item().and_downcast::<Room>()
                            {
                                if selected_room == *room {
                                    selection.set_selected_item(None::<glib::Object>);
                                }
                            }
                        }

                        if category != RoomCategory::Invited {
                            let imp = obj.imp();
                            imp.decline_requests.borrow_mut().remove(room);
                            imp.accept_requests.borrow_mut().remove(room);
                            obj.reset();
                            if let Some(category_handler) = imp.category_handler.take() {
                                room.disconnect(category_handler);
                            }
                        }
                    }
                ));
                self.category_handler.replace(Some(category_handler));

                if let Some(inviter) = room.inviter() {
                    let pill = inviter.to_pill();
                    let label = gettext_f(
                        // Translators: Do NOT translate the content between '{' and '}', these are
                        // variable names.
                        "{user_name} ({user_id}) invited you",
                        &[
                            ("user_name", LabelWithWidgets::PLACEHOLDER),
                            ("user_id", inviter.user_id().as_str()),
                        ],
                    );
                    self.inviter.set_label_and_widgets(label, vec![pill]);
                }
            }

            // Keep a strong reference to the members list.
            self.room_members
                .replace(room.as_ref().map(Room::get_or_create_members));
            self.room.replace(room);

            obj.notify_room();
        }
    }
}

glib::wrapper! {
    /// A view presenting an invitation to a room.
    pub struct Invite(ObjectSubclass<imp::Invite>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl Invite {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// The header bar of the invite.
    pub fn header_bar(&self) -> &adw::HeaderBar {
        &self.imp().header_bar
    }

    fn reset(&self) {
        let imp = self.imp();
        imp.accept_button.set_is_loading(false);
        imp.decline_button.set_is_loading(false);
        self.action_set_enabled("invite.accept", true);
        self.action_set_enabled("invite.decline", true);
    }

    /// Accept the invite.
    async fn accept(&self) {
        let Some(room) = self.room() else {
            return;
        };
        let imp = self.imp();

        self.action_set_enabled("invite.accept", false);
        self.action_set_enabled("invite.decline", false);
        imp.accept_button.set_is_loading(true);
        imp.accept_requests.borrow_mut().insert(room.clone());

        if room.set_category(TargetRoomCategory::Normal).await.is_err() {
            toast!(
                self,
                gettext(
                    // Translators: Do NOT translate the content between '{' and '}', this
                    // is a variable name.
                    "Could not accept invitation for {room}",
                ),
                @room,
            );

            imp.accept_requests.borrow_mut().remove(&room);
            self.reset();
        }
    }

    /// Decline the invite.
    async fn decline(&self) {
        let Some(room) = self.room() else {
            return;
        };
        let imp = self.imp();

        let Some(response) = confirm_leave_room_dialog(&room, self).await else {
            return;
        };

        self.action_set_enabled("invite.accept", false);
        self.action_set_enabled("invite.decline", false);
        imp.decline_button.set_is_loading(true);
        imp.decline_requests.borrow_mut().insert(room.clone());

        let ignored_inviter = response.ignore_inviter.then(|| room.inviter()).flatten();

        let closed = if room.set_category(TargetRoomCategory::Left).await.is_ok() {
            // A room where we were invited is usually empty so just close it.
            let _ = self.activate_action("session.close-room", None);
            true
        } else {
            toast!(
                self,
                gettext(
                    // Translators: Do NOT translate the content between '{' and '}', this
                    // is a variable name.
                    "Could not decline invitation for {room}",
                ),
                @room,
            );

            imp.decline_requests.borrow_mut().remove(&room);
            self.reset();
            false
        };

        if let Some(inviter) = ignored_inviter {
            if inviter.upcast::<User>().ignore().await.is_err() {
                toast!(self, gettext("Could not ignore user"));
            } else if !closed {
                // Ignoring the user should remove the room from the sidebar so close it.
                let _ = self.activate_action("session.close-room", None);
            }
        }
    }
}
