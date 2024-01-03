use adw::{prelude::*, subclass::prelude::*};
use gettextrs::npgettext;
use gtk::{glib, glib::clone, CompositeTemplate};

use super::MembershipSubpageItem;
use crate::{session::model::Membership, utils::BoundObject};

mod imp {
    use std::marker::PhantomData;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_details/members_page/members_list_view/membership_subpage_row.ui"
    )]
    #[properties(wrapper_type = super::MembershipSubpageRow)]
    pub struct MembershipSubpageRow {
        /// The item presented by this row.
        #[property(get, set = Self::set_item, explicit_notify, nullable)]
        pub item: BoundObject<MembershipSubpageItem>,
        /// The icon of this row.
        #[property(get = Self::icon)]
        pub icon: PhantomData<Option<String>>,
        /// The label of this row.
        #[property(get = Self::label)]
        pub label: PhantomData<Option<String>>,
        pub gesture: gtk::GestureClick,
        #[template_child]
        pub members_count: TemplateChild<gtk::Label>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MembershipSubpageRow {
        const NAME: &'static str = "MembersPageMembershipSubpageRow";
        type Type = super::MembershipSubpageRow;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for MembershipSubpageRow {}

    impl WidgetImpl for MembershipSubpageRow {}
    impl BinImpl for MembershipSubpageRow {}

    impl MembershipSubpageRow {
        /// Set the item presented by this row.
        fn set_item(&self, item: Option<MembershipSubpageItem>) {
            if self.item.obj() == item {
                return;
            }
            let obj = self.obj();

            self.item.disconnect_signals();

            if let Some(item) = item {
                let model = item.model();

                let handler = model.connect_items_changed(
                    clone!(@weak self as imp => move |model, _, _, _| {
                        imp.member_count_changed(model.n_items());
                        imp.obj().notify_label();
                    }),
                );
                self.member_count_changed(model.n_items());

                self.item.set(item, vec![handler])
            }

            obj.notify_item();
            obj.notify_icon();
            obj.notify_label();
        }

        /// The icon of this row.
        fn icon(&self) -> Option<String> {
            match self.item.obj()?.state() {
                Membership::Invite => Some("user-add-symbolic".to_owned()),
                Membership::Ban => Some("blocked-symbolic".to_owned()),
                _ => None,
            }
        }

        /// The label of this row.
        fn label(&self) -> Option<String> {
            let item = self.item.obj()?;
            let count = item.model().n_items();

            match item.state() {
                // Translators: As in 'Invited Room Member(s)'.
                Membership::Invite => Some(npgettext("members", "Invited", "Invited", count)),
                // Translators: As in 'Banned Room Member(s)'.
                Membership::Ban => Some(npgettext("members", "Banned", "Banned", count)),
                _ => None,
            }
        }

        fn member_count_changed(&self, n: u32) {
            self.members_count.set_text(&format!("{n}"));
        }
    }
}

glib::wrapper! {
    /// A row presenting a `MembershipSubpageItem`.
    pub struct MembershipSubpageRow(ObjectSubclass<imp::MembershipSubpageRow>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl MembershipSubpageRow {
    pub fn new() -> Self {
        glib::Object::new()
    }
}
