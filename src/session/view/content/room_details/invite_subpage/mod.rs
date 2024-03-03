use adw::{prelude::*, subclass::prelude::*};
use gettextrs::ngettext;
use gtk::{gdk, glib, glib::clone, CompositeTemplate};
use ruma::OwnedUserId;

mod invitee;
use self::invitee::Invitee;
mod invitee_list;
mod invitee_row;
use self::{
    invitee_list::{InviteeList, InviteeListState},
    invitee_row::InviteeRow,
};
use crate::{
    components::{PillSearchEntry, Spinner, SpinnerButton},
    prelude::*,
    session::model::Room,
    spawn, toast,
};

mod imp {
    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_details/invite_subpage/mod.ui"
    )]
    #[properties(wrapper_type = super::InviteSubpage)]
    pub struct InviteSubpage {
        /// The room users will be invited to.
        #[property(get, set = Self::set_room, construct_only)]
        pub room: glib::WeakRef<Room>,
        #[template_child]
        pub search_entry: TemplateChild<PillSearchEntry>,
        #[template_child]
        pub list_view: TemplateChild<gtk::ListView>,
        #[template_child]
        pub invite_button: TemplateChild<SpinnerButton>,
        #[template_child]
        pub cancel_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub matching_page: TemplateChild<gtk::ScrolledWindow>,
        #[template_child]
        pub no_matching_page: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub no_search_page: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub error_page: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub loading_page: TemplateChild<Spinner>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for InviteSubpage {
        const NAME: &'static str = "ContentInviteSubpage";
        type Type = super::InviteSubpage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            InviteeRow::static_type();
            Self::bind_template(klass);

            klass.add_binding(gdk::Key::Escape, gdk::ModifierType::empty(), |obj| {
                obj.close();
                glib::Propagation::Stop
            });
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for InviteSubpage {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            self.cancel_button
                .connect_clicked(clone!(@weak obj => move |_| {
                    obj.close();
                }));

            self.search_entry.connect_pill_removed(|_, source| {
                if let Ok(user) = source.downcast::<Invitee>() {
                    user.set_invited(false);
                }
            });

            self.invite_button
                .connect_clicked(clone!(@weak obj => move |_| {
                    obj.invite();
                }));

            self.list_view.connect_activate(|list_view, index| {
                let Some(invitee) = list_view
                    .model()
                    .and_then(|m| m.item(index))
                    .and_downcast::<Invitee>()
                else {
                    return;
                };

                invitee.set_invited(!invitee.invited());
            });
        }
    }

    impl WidgetImpl for InviteSubpage {}
    impl NavigationPageImpl for InviteSubpage {}

    impl InviteSubpage {
        /// Set the room users will be invited to.
        fn set_room(&self, room: Room) {
            let obj = self.obj();

            let user_list = InviteeList::new(&room);
            user_list.connect_invitee_added(clone!(@weak self as imp => move |_, invitee| {
                imp.search_entry.add_pill(invitee);
            }));

            user_list.connect_invitee_removed(clone!(@weak self as imp => move |_, invitee| {
                imp.search_entry.remove_pill(&invitee.identifier());
            }));

            user_list.connect_state_notify(clone!(@weak obj => move |_| {
                obj.update_view();
            }));

            self.search_entry
                .bind_property("text", &user_list, "search-term")
                .sync_create()
                .build();

            user_list
                .bind_property("has-selected", &*self.invite_button, "sensitive")
                .sync_create()
                .build();

            self.list_view
                .set_model(Some(&gtk::NoSelection::new(Some(user_list))));

            self.room.set(Some(&room));
            obj.notify_room();
        }
    }
}

glib::wrapper! {
    /// Subpage to invite new members to a room.
    pub struct InviteSubpage(ObjectSubclass<imp::InviteSubpage>)
        @extends gtk::Widget, gtk::Window, adw::NavigationPage, @implements gtk::Accessible;
}

impl InviteSubpage {
    pub fn new(room: &Room) -> Self {
        glib::Object::builder().property("room", room).build()
    }

    fn close(&self) {
        let window = self
            .root()
            .and_downcast::<adw::PreferencesWindow>()
            .unwrap();
        if self.can_pop() {
            window.pop_subpage();
        } else {
            window.close();
        }
    }

    fn invitee_list(&self) -> Option<InviteeList> {
        self.imp()
            .list_view
            .model()
            .and_downcast::<gtk::NoSelection>()?
            .model()
            .and_downcast::<InviteeList>()
    }

    /// Invite the selected users to the room.
    fn invite(&self) {
        self.imp().invite_button.set_loading(true);

        spawn!(clone!(@weak self as obj => async move {
            obj.invite_inner().await;
        }));
    }

    async fn invite_inner(&self) {
        let Some(room) = self.room() else {
            return;
        };
        let Some(user_list) = self.invitee_list() else {
            return;
        };

        let invitees: Vec<OwnedUserId> = user_list
            .invitees()
            .into_iter()
            .map(|i| i.user_id().clone())
            .collect();

        match room.invite(&invitees).await {
            Ok(()) => {
                self.close();
            }
            Err(failed_users) => {
                for invitee in &invitees {
                    if !failed_users.contains(&invitee.as_ref()) {
                        user_list.remove_invitee(invitee)
                    }
                }

                let n = failed_users.len();
                let first_failed = failed_users
                    .first()
                    .and_then(|user_id| {
                        user_list
                            .invitees()
                            .into_iter()
                            .find(|i| i.user_id() == *user_id)
                    })
                    .unwrap();

                toast!(
                    self,
                    ngettext(
                        // Translators: Do NOT translate the content between '{' and '}', these
                        // are variable names.
                        "Failed to invite {user} to {room}. Try again later.",
                        "Failed to invite {n} users to {room}. Try again later.",
                        n as u32,
                    ),
                    @user = first_failed,
                    @room,
                    n = n.to_string(),
                );
            }
        }

        self.imp().invite_button.set_loading(false);
    }

    fn update_view(&self) {
        let imp = self.imp();
        match self
            .invitee_list()
            .expect("Can't update view without an InviteeList")
            .state()
        {
            InviteeListState::Initial => imp.stack.set_visible_child(&*imp.no_search_page),
            InviteeListState::Loading => imp.stack.set_visible_child(&*imp.loading_page),
            InviteeListState::NoMatching => imp.stack.set_visible_child(&*imp.no_matching_page),
            InviteeListState::Matching => imp.stack.set_visible_child(&*imp.matching_page),
            InviteeListState::Error => imp.stack.set_visible_child(&*imp.error_page),
        }
    }
}
