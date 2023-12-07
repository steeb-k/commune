use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{
    gio, glib,
    glib::{clone, closure},
    CompositeTemplate,
};

mod extra_lists;
mod item_row;
mod member_row;
mod membership_subpage_item;
mod membership_subpage_row;

pub use self::{extra_lists::ExtraLists, membership_subpage_item::MembershipSubpageItem};
use self::{
    item_row::ItemRow, member_row::MemberRow, membership_subpage_row::MembershipSubpageRow,
};
use crate::{
    prelude::*,
    session::model::{Member, Membership},
};

mod imp {
    use std::cell::Cell;

    use glib::subclass::InitializingObject;
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_details/members_page/members_list_view/mod.ui"
    )]
    pub struct MembersListView {
        #[template_child]
        pub search_bar: TemplateChild<gtk::SearchBar>,
        #[template_child]
        pub search_entry: TemplateChild<gtk::SearchEntry>,
        #[template_child]
        pub list_view: TemplateChild<gtk::ListView>,
        pub filtered_model: gtk::FilterListModel,
        pub can_invite: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MembersListView {
        const NAME: &'static str = "ContentMembersListView";
        type Type = super::MembersListView;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            ItemRow::static_type();

            Self::bind_template(klass);

            klass.set_css_name("members-list");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for MembersListView {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpecObject::builder::<gio::ListModel>("model")
                        .explicit_notify()
                        .build(),
                    glib::ParamSpecBoolean::builder("can-invite")
                        .explicit_notify()
                        .build(),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            match pspec.name() {
                "model" => self.obj().set_model(value.get::<&gio::ListModel>().ok()),
                "can-invite" => self.obj().set_can_invite(value.get().unwrap()),
                _ => unimplemented!(),
            }
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "model" => self.obj().model().to_value(),
                "can-invite" => self.obj().can_invite().to_value(),
                _ => unimplemented!(),
            }
        }

        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            // Needed because the GtkSearchEntry is not the direct child of the
            // GtkSearchBear.
            self.search_bar.connect_entry(&*self.search_entry);

            fn search_string(member: Member) -> String {
                format!(
                    "{} {} {} {}",
                    member.display_name(),
                    member.user_id(),
                    member.role(),
                    member.power_level(),
                )
            }

            let member_expr = gtk::ClosureExpression::new::<String>(
                &[] as &[gtk::Expression],
                closure!(|item: Option<glib::Object>| {
                    item.and_downcast().map(search_string).unwrap_or_default()
                }),
            );
            let search_filter = gtk::StringFilter::builder()
                .match_mode(gtk::StringFilterMatchMode::Substring)
                .expression(&member_expr)
                .ignore_case(true)
                .build();

            self.search_entry
                .bind_property("text", &search_filter, "search")
                .sync_create()
                .build();

            self.filtered_model.set_filter(Some(&search_filter));

            self.list_view.set_model(Some(&gtk::NoSelection::new(Some(
                self.filtered_model.clone(),
            ))));
            self.list_view
                .connect_activate(clone!(@weak obj => move |_, pos| {
                    let Some(item) = obj.imp().filtered_model.item(pos) else {
                        return;
                    };

                    if let Some(member) = item.downcast_ref::<Member>() {
                        obj.activate_action(
                            "members.show-member",
                            Some(&member.user_id().as_str().to_variant()),
                        )
                        .unwrap();
                    } else if let Some(item) = item.downcast_ref::<MembershipSubpageItem>() {
                        obj.activate_action(
                            "members.show-membership-list",
                            Some(&item.state().to_variant()),
                        )
                        .unwrap();
                    }
                }));
        }
    }

    impl WidgetImpl for MembersListView {}
    impl NavigationPageImpl for MembersListView {}
}

glib::wrapper! {
    pub struct MembersListView(ObjectSubclass<imp::MembersListView>)
        @extends gtk::Widget, adw::NavigationPage;
}

impl MembersListView {
    pub fn new(model: &impl IsA<gio::ListModel>, membership: Membership) -> Self {
        let (tag, title) = match membership {
            Membership::Invite => ("invited", gettext("Invited Room Members")),
            Membership::Ban => ("banned", gettext("Banned Room Members")),
            _ => ("joined", gettext("Room Members")),
        };

        glib::Object::builder()
            .property("model", model)
            .property("tag", tag)
            .property("title", title)
            .build()
    }

    /// The model used for this view.
    pub fn model(&self) -> Option<gio::ListModel> {
        self.imp().filtered_model.model()
    }

    /// Set the model used for this view.
    pub fn set_model(&self, model: Option<&impl IsA<gio::ListModel>>) {
        let model: Option<&gio::ListModel> = model.map(|model| model.upcast_ref());
        if self.model().as_ref() == model {
            return;
        }

        self.imp().filtered_model.set_model(model);
        self.notify("model");
    }

    /// Whether our own user can send an invite in the current room.
    pub fn can_invite(&self) -> bool {
        self.imp().can_invite.get()
    }

    /// Set whether our own user can send an invite in the current room.
    pub fn set_can_invite(&self, can_invite: bool) {
        if self.can_invite() == can_invite {
            return;
        }

        self.imp().can_invite.set(can_invite);
        self.notify("can-invite");
    }
}
