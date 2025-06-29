use adw::{prelude::*, subclass::prelude::*};
use gtk::{CompositeTemplate, glib};

mod members_list_view;

use self::members_list_view::MembersListView;
use super::membership_lists::MembershipLists;
use crate::session::model::{Membership, Room};

mod imp {
    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_details/members_page/mod.ui"
    )]
    #[properties(wrapper_type = super::MembersPage)]
    pub struct MembersPage {
        #[template_child]
        navigation_view: TemplateChild<adw::NavigationView>,
        /// The room containing the members.
        #[property(get, construct_only)]
        room: glib::WeakRef<Room>,
        /// The lists of members filtered by membership for the room.
        #[property(get, construct_only)]
        membership_lists: glib::WeakRef<MembershipLists>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MembersPage {
        const NAME: &'static str = "MembersPage";
        type Type = super::MembersPage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);

            klass.install_action(
                "members.show-membership-list",
                Some(&Membership::static_variant_type()),
                |obj, _, param| {
                    let Some(membership) = param.and_then(glib::Variant::get::<Membership>) else {
                        return;
                    };

                    obj.imp().show_membership_list(membership);
                },
            );
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for MembersPage {
        fn constructed(&self) {
            self.parent_constructed();

            // Initialize the first page.
            self.show_membership_list(Membership::Join);
        }
    }

    impl WidgetImpl for MembersPage {}
    impl NavigationPageImpl for MembersPage {}

    impl MembersPage {
        /// Show the subpage for the list with the given membership.
        fn show_membership_list(&self, membership: Membership) {
            let tag = membership_as_tag(membership);

            if self.navigation_view.find_page(tag).is_some() {
                self.navigation_view.push_by_tag(tag);
                return;
            }

            let Some(room) = self.room.upgrade() else {
                return;
            };
            let Some(membership_lists) = self.membership_lists.upgrade() else {
                return;
            };

            let subpage = MembersListView::new(&room, &membership_lists, membership);
            self.navigation_view.push(&subpage);
        }
    }
}

glib::wrapper! {
    /// A page showing the members of a room.
    pub struct MembersPage(ObjectSubclass<imp::MembersPage>)
        @extends gtk::Widget, adw::NavigationPage;
}

impl MembersPage {
    /// Construct a `MembersPage` for the given room and membership lists.
    pub fn new(room: &Room, membership_lists: &MembershipLists) -> Self {
        glib::Object::builder()
            .property("room", room)
            .property("membership-lists", membership_lists)
            .build()
    }
}

/// Get a page tag for the given membership.
fn membership_as_tag(membership: Membership) -> &'static str {
    match membership {
        Membership::Invite => "invited",
        Membership::Ban => "banned",
        _ => "joined",
    }
}
