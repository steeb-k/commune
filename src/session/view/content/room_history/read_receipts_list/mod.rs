use adw::subclass::prelude::*;
use gtk::{gio, glib, glib::clone, prelude::*, CompositeTemplate};

mod read_receipts_popover;

use self::read_receipts_popover::ReadReceiptsPopover;
use super::member_timestamp::MemberTimestamp;
use crate::{
    components::{Avatar, OverlappingBox},
    i18n::{gettext_f, ngettext_f},
    prelude::*,
    session::model::{Member, MemberList, UserReadReceipt},
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
        pub toggle_button: TemplateChild<gtk::ToggleButton>,
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
        /// The displayed member if there is only one receipt.
        pub receipt_member: BoundObjectWeakRef<Member>,
    }

    impl Default for ReadReceiptsList {
        fn default() -> Self {
            Self {
                toggle_button: Default::default(),
                label: Default::default(),
                overlapping_box: Default::default(),
                members: Default::default(),
                list: gio::ListStore::new::<MemberTimestamp>(),
                source: Default::default(),
                receipt_member: Default::default(),
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
            Self::Type::bind_template_callbacks(klass);

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

                    if let Some(member) = item.downcast_ref::<MemberTimestamp>().and_then(|r| r.member()) {
                        avatar.set_data(Some(member.avatar_data().clone()));
                    }

                    let cutout = adw::Bin::builder().child(&avatar).css_classes(["cutout"]).build();
                    cutout.upcast()
                }),
            );

            self.list
                .connect_items_changed(clone!(@weak obj => move |_, _,_,_| {
                    obj.update_tooltip();
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

#[gtk::template_callbacks]
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
        let Some(members) = &*self.imp().members.borrow() else {
            return;
        };

        let mut new_receipts = Vec::with_capacity(added as usize);

        for i in pos..pos + added {
            let Some(boxed) = source.item(i).and_downcast::<glib::BoxedAnyObject>() else {
                break;
            };

            let source_receipt = boxed.borrow::<UserReadReceipt>();
            let member = members.get_or_create(source_receipt.user_id.clone());
            let receipt = MemberTimestamp::new(
                &member,
                source_receipt.receipt.ts.map(|ts| ts.as_secs().into()),
            );

            new_receipts.push(receipt);
        }

        self.list().splice(pos, removed, &new_receipts);
    }

    fn update_tooltip(&self) {
        let imp = self.imp();
        imp.receipt_member.disconnect_signals();
        let n_items = self.list().n_items();

        if n_items == 1 {
            if let Some(member) = self
                .list()
                .item(0)
                .and_downcast::<MemberTimestamp>()
                .and_then(|r| r.member())
            {
                // Listen to changes of the display name.
                let handler_id = member.connect_notify_local(
                    Some("display-name"),
                    clone!(@weak self as obj => move |member, _| {
                        obj.update_member_tooltip(member);
                    }),
                );

                imp.receipt_member.set(&member, vec![handler_id]);
                self.update_member_tooltip(&member);
                return;
            }
        }

        let text = (n_items > 0).then(|| {
            ngettext_f(
                // Translators: Do NOT translate the content between '{' and '}', this is a
                // variable name.
                "Seen by 1 member",
                "Seen by {n} members",
                n_items,
                &[("n", &n_items.to_string())],
            )
        });

        self.imp().toggle_button.set_tooltip_text(text.as_deref())
    }

    fn update_member_tooltip(&self, member: &Member) {
        // Translators: Do NOT translate the content between '{' and '}', this is a
        // variable name.
        let text = gettext_f("Seen by {name}", &[("name", &member.display_name())]);

        self.imp().toggle_button.set_tooltip_text(Some(&text));
    }

    fn update_label(&self) {
        let label = &self.imp().label;
        let n_items = self.list().n_items();

        if n_items > MAX_RECEIPTS_SHOWN {
            label.set_text(&format!("{} +", n_items - MAX_RECEIPTS_SHOWN));
            label.set_visible(true);
        } else {
            label.set_visible(false);
        }
    }

    /// Handle a click on the container.
    ///
    /// Shows a popover with the list of receipts if there are any.
    #[template_callback]
    fn show_popover(&self) {
        if self.list().n_items() == 0 {
            // No popover.
            return;
        }

        let toggle_button = &*self.imp().toggle_button;
        let popover = ReadReceiptsPopover::new(self.list());
        popover.set_parent(toggle_button);
        popover.connect_closed(clone!(@weak toggle_button => move |popover| {
            popover.unparent();
            toggle_button.set_active(false);
        }));

        popover.popup();
    }
}
