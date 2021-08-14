use adw::subclass::prelude::*;
use gettextrs::ngettext;
use gtk::glib::{self, clone};
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::CompositeTemplate;

use crate::components::{Avatar, Badge};
use crate::prelude::*;
use crate::session::room::{Member, RoomAction};
use crate::session::Room;

mod imp {
    use super::*;
    use glib::subclass::InitializingObject;
    use once_cell::sync::Lazy;
    use once_cell::unsync::OnceCell;

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
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MemberPage {
        const NAME: &'static str = "ContentMemberPage";
        type Type = super::MemberPage;
        type ParentType = adw::PreferencesPage;

        fn class_init(klass: &mut Self::Class) {
            Avatar::static_type();
            Badge::static_type();
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for MemberPage {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![glib::ParamSpec::new_object(
                    "room",
                    "Room",
                    "The room backing all details of the member page",
                    Room::static_type(),
                    glib::ParamFlags::READWRITE | glib::ParamFlags::CONSTRUCT_ONLY,
                )]
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

        fn property(&self, _obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "room" => self.room.get().to_value(),
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

        fn search_string(member: Member) -> String {
            format!(
                "{} {} {} {}",
                member.display_name(),
                member.user_id(),
                member.role(),
                member.power_level(),
            )
        }

        let member_expr = gtk::ClosureExpression::new(
            |value| {
                value[0]
                    .get::<Member>()
                    .map(search_string)
                    .unwrap_or_default()
            },
            &[],
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

        let filter_model = gtk::FilterListModel::new(Some(members), Some(&filter));
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
    }
}
