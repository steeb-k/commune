use adw::{prelude::*, subclass::prelude::*};
use gettextrs::ngettext;
use gtk::{
    glib::{self, clone, closure},
    subclass::prelude::*,
    CompositeTemplate,
};

mod member_menu;
mod member_row;
use log::warn;

use self::{member_menu::MemberMenu, member_row::MemberRow};
use crate::{
    components::{Avatar, Badge},
    prelude::*,
    session::{
        content::RoomDetails,
        room::{Member, RoomAction},
        Room, User, UserActions,
    },
    spawn,
};

mod imp {
    use glib::subclass::InitializingObject;
    use once_cell::{sync::Lazy, unsync::OnceCell};

    use super::*;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/content-member-page.ui")]
    pub struct MemberPage {
        pub room: OnceCell<Room>,
        #[template_child]
        pub member_count: TemplateChild<gtk::Label>,
        #[template_child]
        pub invite_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub members_search_entry: TemplateChild<gtk::SearchEntry>,
        #[template_child]
        pub members_list_view: TemplateChild<gtk::ListView>,
        pub member_menu: OnceCell<MemberMenu>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MemberPage {
        const NAME: &'static str = "ContentMemberPage";
        type Type = super::MemberPage;
        type ParentType = adw::PreferencesPage;

        fn class_init(klass: &mut Self::Class) {
            Avatar::static_type();
            Badge::static_type();
            MemberRow::static_type();
            Self::bind_template(klass);

            klass.install_action("member.verify", None, move |widget, _, _| {
                if let Some(member) = widget.member_menu().member() {
                    widget.verify_member(member);
                } else {
                    warn!("No member was selected to be verified");
                }
            });
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for MemberPage {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpecObject::new(
                        "room",
                        "Room",
                        "The room backing all details of the member page",
                        Room::static_type(),
                        glib::ParamFlags::READWRITE | glib::ParamFlags::CONSTRUCT_ONLY,
                    ),
                    glib::ParamSpecObject::new(
                        "member-menu",
                        "Member Menu",
                        "The object holding information needed for the menu of each MemberRow",
                        MemberMenu::static_type(),
                        glib::ParamFlags::READABLE,
                    ),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(
            &self,
            obj: &Self::Type,
            _id: usize,
            value: &glib::Value,
            pspec: &glib::ParamSpec,
        ) {
            match pspec.name() {
                "room" => obj.set_room(value.get().unwrap()),
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "room" => self.room.get().to_value(),
                "member-menu" => obj.member_menu().to_value(),
                _ => unimplemented!(),
            }
        }

        fn constructed(&self, obj: &Self::Type) {
            self.parent_constructed(obj);

            obj.init_member_search();
            obj.init_member_count();
            obj.init_invite_button();
        }
    }
    impl WidgetImpl for MemberPage {}
    impl PreferencesPageImpl for MemberPage {}
}

glib::wrapper! {
    pub struct MemberPage(ObjectSubclass<imp::MemberPage>)
        @extends gtk::Widget, adw::PreferencesPage;
}

impl MemberPage {
    pub fn new(room: &Room) -> Self {
        glib::Object::new(&[("room", room)]).expect("Failed to create MemberPage")
    }

    pub fn room(&self) -> &Room {
        let priv_ = imp::MemberPage::from_instance(self);
        priv_.room.get().unwrap()
    }

    fn set_room(&self, room: Room) {
        let priv_ = imp::MemberPage::from_instance(self);
        priv_.room.set(room).expect("Room already initialized");
    }

    fn init_member_search(&self) {
        let priv_ = imp::MemberPage::from_instance(self);
        let members = self.room().members();

        // Sort the members list by power level, then display name.
        let sorter = gtk::MultiSorter::new();
        sorter.append(
            &gtk::NumericSorter::builder()
                .expression(&gtk::PropertyExpression::new(
                    Member::static_type(),
                    gtk::Expression::NONE,
                    "power-level",
                ))
                .sort_order(gtk::SortType::Descending)
                .build(),
        );
        sorter.append(&gtk::StringSorter::new(Some(
            &gtk::PropertyExpression::new(
                Member::static_type(),
                gtk::Expression::NONE,
                "display-name",
            ),
        )));
        let sorted_members = gtk::SortListModel::new(Some(members), Some(&sorter));

        fn search_string(member: Member) -> String {
            format!(
                "{} {} {} {}",
                member.display_name(),
                member.user_id(),
                member.role(),
                member.power_level(),
            )
        }

        let member_expr = gtk::ClosureExpression::new::<String, &[gtk::Expression], _>(
            &[],
            closure!(|member: Option<Member>| { member.map(search_string).unwrap_or_default() }),
        );
        let filter = gtk::StringFilter::builder()
            .match_mode(gtk::StringFilterMatchMode::Substring)
            .expression(&member_expr)
            .ignore_case(true)
            .build();
        priv_
            .members_search_entry
            .bind_property("text", &filter, "search")
            .flags(glib::BindingFlags::SYNC_CREATE)
            .build();

        let filter_model = gtk::FilterListModel::new(Some(&sorted_members), Some(&filter));
        let model = gtk::NoSelection::new(Some(&filter_model));
        priv_.members_list_view.set_model(Some(&model));
    }

    fn init_member_count(&self) {
        let priv_ = imp::MemberPage::from_instance(self);
        let members = self.room().members();

        let member_count = priv_.member_count.get();
        fn set_member_count(member_count: &gtk::Label, n: u32) {
            member_count.set_text(&ngettext!("{} Member", "{} Members", n, n));
        }
        set_member_count(&member_count, members.n_items());
        members.connect_items_changed(clone!(@weak member_count => move |members, _, _, _| {
            set_member_count(&member_count, members.n_items());
        }));
    }

    fn init_invite_button(&self) {
        let priv_ = imp::MemberPage::from_instance(self);

        let invite_possible = self.room().new_allowed_expr(RoomAction::Invite);
        const NONE_OBJECT: Option<&glib::Object> = None;
        invite_possible.bind(&*priv_.invite_button, "sensitive", NONE_OBJECT);

        priv_
            .invite_button
            .connect_clicked(clone!(@weak self as obj => move |_| {
                let window = obj
                .root()
                .unwrap()
                .downcast::<RoomDetails>()
                .unwrap();
                window.present_invite_subpage();
            }));
    }

    pub fn member_menu(&self) -> &MemberMenu {
        let priv_ = imp::MemberPage::from_instance(self);
        priv_.member_menu.get_or_init(|| {
            let menu = MemberMenu::new();

            menu.connect_notify_local(
                Some("allowed-actions"),
                clone!(@weak self as obj => move |menu, _| {
                    obj.update_actions(menu.allowed_actions());
                }),
            );
            self.update_actions(menu.allowed_actions());
            menu
        })
    }

    fn update_actions(&self, allowed_actions: UserActions) {
        self.action_set_enabled(
            "member.verify",
            allowed_actions.contains(UserActions::VERIFY),
        );
    }

    fn verify_member(&self, member: Member) {
        // TODO: show the verification immediately when started
        spawn!(clone!(@weak self as obj => async move {
            member.upcast::<User>().verify_identity().await;
        }));
    }
}
