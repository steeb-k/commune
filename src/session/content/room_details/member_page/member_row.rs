use gtk::{glib, prelude::*, subclass::prelude::*, CompositeTemplate};

use crate::session::room::Member;
use adw::subclass::prelude::BinImpl;

mod imp {
    use super::*;
    use glib::subclass::InitializingObject;
    use once_cell::sync::Lazy;
    use std::cell::RefCell;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/FractalNext/content-member-row.ui")]
    pub struct MemberRow {
        pub member: RefCell<Option<Member>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MemberRow {
        const NAME: &'static str = "ContentMemberRow";
        type Type = super::MemberRow;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for MemberRow {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![glib::ParamSpec::new_object(
                    "member",
                    "Member",
                    "The member this row is showing",
                    Member::static_type(),
                    glib::ParamFlags::READWRITE | glib::ParamFlags::EXPLICIT_NOTIFY,
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
                "member" => {
                    obj.set_member(value.get().unwrap());
                }
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "member" => obj.member().to_value(),
                _ => unimplemented!(),
            }
        }
    }
    impl WidgetImpl for MemberRow {}
    impl BinImpl for MemberRow {}
}

glib::wrapper! {
    pub struct MemberRow(ObjectSubclass<imp::MemberRow>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl MemberRow {
    pub fn new(member: &Member) -> Self {
        glib::Object::new(&[("member", member)]).expect("Failed to create MemberRow")
    }

    pub fn member(&self) -> Option<Member> {
        let priv_ = imp::MemberRow::from_instance(self);
        priv_.member.borrow().clone()
    }

    pub fn set_member(&self, member: Option<Member>) {
        let priv_ = imp::MemberRow::from_instance(self);

        if self.member() == member {
            return;
        }

        priv_.member.replace(member);
        self.notify("member");
    }
}
