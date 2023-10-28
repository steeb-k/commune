use adw::subclass::prelude::*;
use gtk::{gio, glib, glib::clone, prelude::*, CompositeTemplate};

mod member_read_receipt;

use self::member_read_receipt::MemberReadReceipt;
use crate::{
    components::{Avatar, OverlappingBox},
    prelude::*,
    session::model::{MemberList, UserReadReceipt},
    utils::BoundObjectWeakRef,
};

// Keep in sync with the `max-children` property of the `overlapping_box` in the
// UI file.
const MAX_RECEIPTS_SHOWN: u32 = 10;

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, CompositeTemplate)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_history/read_receipts_list/mod.ui"
    )]
    pub struct ReadReceiptsList {
        #[template_child]
        pub label: TemplateChild<gtk::Label>,
        #[template_child]
        pub overlapping_box: TemplateChild<OverlappingBox>,
        /// The list of room members.
        pub members: RefCell<Option<MemberList>>,
        /// The list of read receipts.
        pub list: gio::ListStore,
        /// The read receipts used as a source.
        pub source: BoundObjectWeakRef<gio::ListStore>,
    }

    impl Default for ReadReceiptsList {
        fn default() -> Self {
            Self {
                label: Default::default(),
                overlapping_box: Default::default(),
                members: Default::default(),
                list: gio::ListStore::new::<MemberReadReceipt>(),
                source: Default::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ReadReceiptsList {
        const NAME: &'static str = "ContentReadReceiptsList";
        type Type = super::ReadReceiptsList;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            klass.set_css_name("read-receipts-list");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for ReadReceiptsList {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpecObject::builder::<MemberList>("members").build(),
                    glib::ParamSpecObject::builder::<gio::ListStore>("list")
                        .read_only()
                        .build(),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            let obj = self.obj();

            match pspec.name() {
                "members" => obj.members().to_value(),
                "list" => obj.list().to_value(),
                _ => unimplemented!(),
            }
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            let obj = self.obj();

            match pspec.name() {
                "members" => obj.set_members(value.get().unwrap()),
                _ => unimplemented!(),
            }
        }

        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            self.overlapping_box.bind_model(
                Some(&self.list),
                clone!(@weak obj => @default-return { Avatar::new().upcast() }, move |item| {
                    let avatar = Avatar::new();
                    avatar.set_size(20);

                    if let Some(member) = item.downcast_ref::<MemberReadReceipt>().and_then(|r| r.member()) {
                        avatar.set_data(Some(member.avatar_data().clone()));
                    }

                    let cutout = adw::Bin::builder().child(&avatar).css_classes(["cutout"]).build();
                    cutout.upcast()
                }),
            );

            self.list
                .connect_items_changed(clone!(@weak obj => move |_, _,_,_| {
                    obj.update_label();
                }));
        }

        fn dispose(&self) {
            self.source.disconnect_signals();
        }
    }

    impl WidgetImpl for ReadReceiptsList {}

    impl BinImpl for ReadReceiptsList {}
}

glib::wrapper! {
    /// A widget displaying the read receipts on a message.
    pub struct ReadReceiptsList(ObjectSubclass<imp::ReadReceiptsList>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl ReadReceiptsList {
    pub fn new(members: &MemberList) -> Self {
        glib::Object::builder().property("members", members).build()
    }

    /// The list of room members.
    pub fn members(&self) -> Option<MemberList> {
        self.imp().members.borrow().clone()
    }

    /// Set the list of room members.
    pub fn set_members(&self, members: Option<MemberList>) {
        let imp = self.imp();

        if imp.members.borrow().as_ref() == members.as_ref() {
            return;
        }

        imp.members.replace(members);
        self.notify("members");

        if let Some(source) = imp.source.obj() {
            self.items_changed(&source, 0, self.list().n_items(), source.n_items());
        }
    }

    /// The list of read receipts to present.
    pub fn list(&self) -> &gio::ListStore {
        &self.imp().list
    }

    /// Set the read receipts that are used as a source of data.
    pub fn set_source(&self, source: &gio::ListStore) {
        let imp = self.imp();

        let items_changed_handler_id = source.connect_items_changed(
            clone!(@weak self as obj => move |source, pos, removed, added| {
                obj.items_changed(source, pos, removed, added);
            }),
        );
        self.items_changed(source, 0, self.list().n_items(), source.n_items());

        imp.source.set(source, vec![items_changed_handler_id]);
    }

    fn items_changed(&self, source: &gio::ListStore, pos: u32, removed: u32, added: u32) {
        let mut new_receipts = Vec::with_capacity(added as usize);

        {
            let Some(members) = &*self.imp().members.borrow() else {
                return;
            };

            for i in pos..pos + added {
                let Some(boxed) = source.item(i).and_downcast::<glib::BoxedAnyObject>() else {
                    break;
                };

                let source_receipt = boxed.borrow::<UserReadReceipt>();
                let member = members.get_or_create(source_receipt.user_id.clone());
                let receipt = MemberReadReceipt::new(
                    &member,
                    source_receipt.receipt.ts.map(|ts| ts.0.into()),
                );

                new_receipts.push(receipt);
            }
        }

        self.list().splice(pos, removed, &new_receipts);
    }

    fn update_label(&self) {
        let label = &self.imp().label;
        let n_items = self.list().n_items();

        if n_items > MAX_RECEIPTS_SHOWN {
            label.set_text(&format!("{} +", n_items - MAX_RECEIPTS_SHOWN));
        } else {
            label.set_text("");
        }
    }
}
