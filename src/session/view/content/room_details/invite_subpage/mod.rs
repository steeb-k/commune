use adw::{prelude::*, subclass::prelude::*};
use gettextrs::ngettext;
use gtk::{gdk, glib, glib::clone, CompositeTemplate};
use tracing::error;

mod item;
mod list;
mod row;

use self::{
    item::InviteItem,
    list::{InviteList, InviteListState},
    row::InviteRow,
};
use crate::{
    components::{LoadingButton, PillSearchEntry, PillSource, Spinner},
    prelude::*,
    session::model::{Room, User},
    toast,
};

mod imp {
    use std::cell::OnceCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_details/invite_subpage/mod.ui"
    )]
    #[properties(wrapper_type = super::InviteSubpage)]
    pub struct InviteSubpage {
        #[template_child]
        pub search_entry: TemplateChild<PillSearchEntry>,
        #[template_child]
        pub list_view: TemplateChild<gtk::ListView>,
        #[template_child]
        pub invite_button: TemplateChild<LoadingButton>,
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
        /// The room users will be invited to.
        #[property(get, set = Self::set_room, construct_only)]
        pub room: glib::WeakRef<Room>,
        /// The list managing the invited users.
        #[property(get)]
        pub invite_list: OnceCell<InviteList>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for InviteSubpage {
        const NAME: &'static str = "RoomDetailsInviteSubpage";
        type Type = super::InviteSubpage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            InviteRow::ensure_type();

            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);

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
    impl ObjectImpl for InviteSubpage {}

    impl WidgetImpl for InviteSubpage {}
    impl NavigationPageImpl for InviteSubpage {}

    impl InviteSubpage {
        /// Set the room users will be invited to.
        fn set_room(&self, room: Room) {
            let obj = self.obj();

            let invite_list = self.invite_list.get_or_init(|| InviteList::new(&room));
            invite_list.connect_invitee_added(clone!(@weak self as imp => move |_, invitee| {
                imp.search_entry.add_pill(&invitee.user());
            }));

            invite_list.connect_invitee_removed(clone!(@weak self as imp => move |_, invitee| {
                imp.search_entry.remove_pill(&invitee.user().identifier());
            }));

            invite_list.connect_state_notify(clone!(@weak self as imp => move |_| {
                imp.update_view();
            }));

            self.search_entry
                .bind_property("text", invite_list, "search-term")
                .sync_create()
                .build();

            invite_list
                .bind_property("has-invitees", &*self.invite_button, "sensitive")
                .sync_create()
                .build();

            self.list_view
                .set_model(Some(&gtk::NoSelection::new(Some(invite_list.clone()))));

            self.room.set(Some(&room));
            obj.notify_room();
        }

        /// Update the view for the current state of the list.
        fn update_view(&self) {
            let state = self
                .invite_list
                .get()
                .expect("Can't update view without an InviteeList")
                .state();

            let page = match state {
                InviteListState::Initial => "no-search",
                InviteListState::Loading => "loading",
                InviteListState::NoMatching => "no-results",
                InviteListState::Matching => "results",
                InviteListState::Error => "error",
            };

            self.stack.set_visible_child_name(page);
        }
    }
}

glib::wrapper! {
    /// Subpage to invite new members to a room.
    pub struct InviteSubpage(ObjectSubclass<imp::InviteSubpage>)
        @extends gtk::Widget, gtk::Window, adw::NavigationPage, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl InviteSubpage {
    /// Construct a new `InviteSubpage` with the given room.
    pub fn new(room: &Room) -> Self {
        glib::Object::builder().property("room", room).build()
    }

    /// Close this subpage.
    #[template_callback]
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

    /// Toggle the invited state of the item at the given index.
    #[template_callback]
    fn toggle_item_is_invitee(&self, index: u32) {
        let Some(item) = self.invite_list().item(index).and_downcast::<InviteItem>() else {
            return;
        };

        item.set_is_invitee(!item.is_invitee());
    }

    /// Uninvite the user from the given pill source.
    #[template_callback]
    fn remove_pill_invitee(&self, source: PillSource) {
        if let Ok(user) = source.downcast::<User>() {
            self.invite_list().remove_invitee(user.user_id());
        }
    }

    /// Invite the selected users to the room.
    #[template_callback]
    async fn invite(&self) {
        let Some(room) = self.room() else {
            return;
        };

        self.imp().invite_button.set_loading(true);

        let invite_list = self.invite_list();
        let invitees = invite_list.invitees_ids();

        match room.invite(&invitees).await {
            Ok(()) => {
                self.close();
            }
            Err(failed_users) => {
                invite_list.retain_invitees(&failed_users);

                let n_failed = failed_users.len();
                let n = invite_list.n_invitees();
                if n != n_failed {
                    // This should not be possible.
                    error!("The number of failed users does not match the number of remaining invitees: expected {n_failed}, got {n}");
                }

                if n == 0 {
                    self.close();
                } else {
                    let first_failed = invite_list.first_invitee().map(|item| item.user()).unwrap();

                    toast!(
                        self,
                        ngettext(
                            // Translators: Do NOT translate the content between '{' and '}', these
                            // are variable names.
                            "Could not invite {user} to {room}. Try again later.",
                            "Could not invite {n} users to {room}. Try again later.",
                            n as u32,
                        ),
                        @user = first_failed,
                        @room,
                        n = n.to_string(),
                    );
                }
            }
        }

        self.imp().invite_button.set_loading(false);
    }
}
