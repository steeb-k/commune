use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{
    gio,
    glib::{self, clone, closure},
    CompositeTemplate,
};
use ruma::UserId;

mod members_list_view;

use self::members_list_view::{ExtraLists, MembersListView, MembershipSubpageItem};
use crate::{
    components::UserPage,
    session::model::{Member, Membership, Room},
    toast,
};

mod imp {
    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_details/members_page/mod.ui"
    )]
    #[properties(wrapper_type = super::MembersPage)]
    pub struct MembersPage {
        /// The room containing the members.
        #[property(get, set = Self::set_room, construct_only)]
        pub room: glib::WeakRef<Room>,
        #[template_child]
        pub navigation_view: TemplateChild<adw::NavigationView>,
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
                    let Some(membership) = param.and_then(|variant| variant.get::<Membership>())
                    else {
                        return;
                    };

                    let subpage = match membership {
                        Membership::Join => "joined",
                        Membership::Invite => "invited",
                        Membership::Ban => "banned",
                        _ => return,
                    };

                    obj.imp().navigation_view.push_by_tag(subpage);
                },
            );

            klass.install_action(
                "members.show-member",
                Some(&String::static_variant_type()),
                |obj, _, param| {
                    let Some(user_id) = param
                        .and_then(|variant| variant.get::<String>())
                        .and_then(|s| UserId::parse(s).ok())
                    else {
                        return;
                    };
                    let Some(room) = obj.room() else {
                        return;
                    };

                    let member = room.get_or_create_members().get_or_create(user_id);
                    let user_page = UserPage::new(&member);
                    user_page.connect_close(clone!(@weak obj => move |_| {
                        let _ = obj.activate_action("navigation.pop", None);
                        toast!(obj, gettext("The user is not in the room members list anymore"));
                    }));

                    obj.imp().navigation_view.push(&user_page);
                },
            );
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for MembersPage {}

    impl WidgetImpl for MembersPage {}
    impl NavigationPageImpl for MembersPage {}

    impl MembersPage {
        /// Set the room containing the members.
        fn set_room(&self, room: Room) {
            let obj = self.obj();

            obj.init_members_list(&room);

            self.room.set(Some(&room));
            obj.notify_room();
        }
    }
}

glib::wrapper! {
    /// A page showing the members of a room.
    pub struct MembersPage(ObjectSubclass<imp::MembersPage>)
        @extends gtk::Widget, adw::NavigationPage;
}

impl MembersPage {
    pub fn new(room: &Room) -> Self {
        glib::Object::builder().property("room", room).build()
    }

    fn init_members_list(&self, room: &Room) {
        let imp = self.imp();

        // Sort the members list by power level, then display name.
        let sorter = gtk::MultiSorter::new();
        sorter.append(
            gtk::NumericSorter::builder()
                .expression(Member::this_expression("power-level"))
                .sort_order(gtk::SortType::Descending)
                .build(),
        );

        sorter.append(gtk::StringSorter::new(Some(Member::this_expression(
            "display-name",
        ))));

        // We should have a strong reference to the list in the main page so we can use
        // `get_or_create_members()`.
        let members = room.get_or_create_members();
        let sorted_members = gtk::SortListModel::new(Some(members.clone()), Some(sorter));

        let joined_members = self.build_filtered_list(sorted_members.clone(), Membership::Join);
        let invited_members = self.build_filtered_list(sorted_members.clone(), Membership::Invite);
        let banned_members = self.build_filtered_list(sorted_members, Membership::Ban);

        let extra_list = ExtraLists::new(
            &members,
            &MembershipSubpageItem::new(Membership::Invite, &invited_members),
            &MembershipSubpageItem::new(Membership::Ban, &banned_members),
        );
        let model_list = gio::ListStore::builder()
            .item_type(gio::ListModel::static_type())
            .build();
        model_list.append(&extra_list);
        model_list.append(&joined_members);

        let main_list = gtk::FlattenListModel::new(Some(model_list));

        let permissions = room.permissions();
        let joined_view = MembersListView::new(&main_list, Membership::Join);
        permissions
            .bind_property("can-invite", &joined_view, "can-invite")
            .sync_create()
            .build();
        imp.navigation_view.add(&joined_view);
        let invited_view = MembersListView::new(&invited_members, Membership::Invite);
        permissions
            .bind_property("can-invite", &invited_view, "can-invite")
            .sync_create()
            .build();
        imp.navigation_view.add(&invited_view);
        let banned_view = MembersListView::new(&banned_members, Membership::Ban);
        permissions
            .bind_property("can-invite", &banned_view, "can-invite")
            .sync_create()
            .build();
        imp.navigation_view.add(&banned_view);
    }

    fn build_filtered_list(
        &self,
        model: impl IsA<gio::ListModel>,
        state: Membership,
    ) -> gio::ListModel {
        let membership_expression = Member::this_expression("membership").chain_closure::<bool>(
            closure!(|_: Option<glib::Object>, this_state: Membership| this_state == state),
        );

        let membership_filter = gtk::BoolFilter::new(Some(&membership_expression));

        let filter_model = gtk::FilterListModel::new(Some(model), Some(membership_filter));
        filter_model.upcast()
    }
}
