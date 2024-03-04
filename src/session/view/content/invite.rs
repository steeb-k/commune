use adw::subclass::prelude::*;
use gettextrs::gettext;
use gtk::{glib, glib::clone, prelude::*, CompositeTemplate};

use crate::{
    components::{Avatar, LabelWithWidgets, Pill, SpinnerButton},
    gettext_f,
    session::model::{MemberList, Room, RoomType},
    toast,
    utils::message_dialog,
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
        pub room_topic: TemplateChild<gtk::Label>,
        #[template_child]
        pub inviter: TemplateChild<LabelWithWidgets>,
        #[template_child]
        pub accept_button: TemplateChild<SpinnerButton>,
        #[template_child]
        pub decline_button: TemplateChild<SpinnerButton>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Invite {
        const NAME: &'static str = "ContentInvite";
        type Type = super::Invite;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Pill::static_type();
            Avatar::static_type();
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

            self.room_topic.connect_label_notify(|room_topic| {
                room_topic.set_visible(!room_topic.label().is_empty());
            });

            self.room_topic
                .set_visible(!self.room_topic.label().is_empty());

            // Translators: Do NOT translate the content between '{' and '}', this is a
            // variable name.
            self.inviter.set_label(Some(gettext_f(
                "{user} invited you",
                &[("user", "<widget>")],
            )));
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
                    self.accept_button.set_loading(true);
                }
                Some(room) if self.decline_requests.borrow().contains(room) => {
                    obj.action_set_enabled("invite.accept", false);
                    obj.action_set_enabled("invite.decline", false);
                    self.decline_button.set_loading(true);
                }
                _ => obj.reset(),
            }

            if let Some(room) = self.room.take() {
                if let Some(handler) = self.category_handler.take() {
                    room.disconnect(handler);
                }
            }

            if let Some(room) = &room {
                let category_handler = room.connect_category_notify(
                    clone!(@weak obj => move |room| {
                        let category = room.category();

                        if category == RoomType::Left {
                            // We declined the invite or the invite was retracted, we should close the room
                            // if it is opened.
                            let Some(session) = room.session() else {
                                return;
                            };
                            let selection = session.sidebar_list_model().selection_model();
                            if let Some(selected_room) = selection.selected_item().and_downcast::<Room>() {
                                if selected_room == *room {
                                    selection.set_selected_item(None::<glib::Object>);
                                }
                            }
                        }

                        if category != RoomType::Invited {
                            let imp = obj.imp();
                            imp.decline_requests.borrow_mut().remove(room);
                            imp.accept_requests.borrow_mut().remove(room);
                            obj.reset();
                            if let Some(category_handler) = imp.category_handler.take() {
                                room.disconnect(category_handler);
                            }
                        }
                    }),
                );
                self.category_handler.replace(Some(category_handler));
            }

            // Keep a strong reference to the members list.
            self.room_members
                .replace(room.as_ref().map(|r| r.get_or_create_members()));
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

    fn reset(&self) {
        let imp = self.imp();
        imp.accept_button.set_loading(false);
        imp.decline_button.set_loading(false);
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
        imp.accept_button.set_loading(true);
        imp.accept_requests.borrow_mut().insert(room.clone());

        let result = room.accept_invite().await;
        if result.is_err() {
            toast!(
                self,
                gettext(
                    // Translators: Do NOT translate the content between '{' and '}', this
                    // is a variable name.
                    "Could not accept invitation for {room}. Try again later.",
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

        if !message_dialog::confirm_leave_room(&room, self).await {
            return;
        }

        self.action_set_enabled("invite.accept", false);
        self.action_set_enabled("invite.decline", false);
        imp.decline_button.set_loading(true);
        imp.decline_requests.borrow_mut().insert(room.clone());

        let result = room.decline_invite().await;
        if result.is_err() {
            toast!(
                self,
                gettext(
                    // Translators: Do NOT translate the content between '{' and '}', this
                    // is a variable name.
                    "Could not decline invitation for {room}. Try again later.",
                ),
                @room,
            );

            imp.decline_requests.borrow_mut().remove(&room);
            self.reset();
        }
    }
}
