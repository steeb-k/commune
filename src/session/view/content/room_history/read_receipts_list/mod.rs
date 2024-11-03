use adw::subclass::prelude::*;
use gtk::{gdk, gio, glib, glib::clone, prelude::*, CompositeTemplate};

mod read_receipts_popover;

use self::read_receipts_popover::ReadReceiptsPopover;
use super::member_timestamp::MemberTimestamp;
use crate::{
    components::OverlappingAvatars,
    i18n::{gettext_f, ngettext_f},
    prelude::*,
    session::model::{Member, MemberList, UserReadReceipt},
    utils::{add_activate_binding_action, BoundObjectWeakRef},
};

// Keep in sync with the `max-avatars` property of the `avatar_list` in the
// UI file.
const MAX_RECEIPTS_SHOWN: u32 = 5;

mod imp {
    use std::cell::{Cell, RefCell};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_history/read_receipts_list/mod.ui"
    )]
    #[properties(wrapper_type = super::ReadReceiptsList)]
    pub struct ReadReceiptsList {
        #[template_child]
        pub content: TemplateChild<gtk::Box>,
        #[template_child]
        pub label: TemplateChild<gtk::Label>,
        #[template_child]
        pub avatar_list: TemplateChild<OverlappingAvatars>,
        /// Whether this list is active.
        ///
        /// This list is active when the popover is displayed.
        #[property(get)]
        pub active: Cell<bool>,
        /// The list of room members.
        #[property(get, set = Self::set_members, explicit_notify, nullable)]
        pub members: RefCell<Option<MemberList>>,
        /// The list of read receipts.
        #[property(get)]
        pub list: gio::ListStore,
        /// The read receipts used as a source.
        pub source: BoundObjectWeakRef<gio::ListStore>,
        /// The displayed member if there is only one receipt.
        pub receipt_member: BoundObjectWeakRef<Member>,
    }

    impl Default for ReadReceiptsList {
        fn default() -> Self {
            Self {
                content: Default::default(),
                label: Default::default(),
                avatar_list: Default::default(),
                active: Default::default(),
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
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_layout_manager_type::<gtk::BinLayout>();

            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);

            klass.set_css_name("read-receipts-list");
            klass.set_accessible_role(gtk::AccessibleRole::ToggleButton);

            klass.install_action("read-receipts-list.activate", None, |obj, _, _| {
                obj.show_popover(1, 0.0, 0.0);
            });

            add_activate_binding_action(klass, "read-receipts-list.activate");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for ReadReceiptsList {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            self.avatar_list
                .bind_model(Some(self.list.clone()), |item| {
                    item.downcast_ref::<MemberTimestamp>()
                        .and_then(MemberTimestamp::member)
                        .map(|m| m.avatar_data().clone())
                        .unwrap()
                });

            self.list.connect_items_changed(clone!(
                #[weak]
                obj,
                move |_, _, _, _| {
                    obj.update_tooltip();
                    obj.update_label();
                }
            ));

            obj.set_pressed_state(false);
        }

        fn dispose(&self) {
            self.content.unparent();
        }
    }

    impl WidgetImpl for ReadReceiptsList {}

    impl AccessibleImpl for ReadReceiptsList {
        fn first_accessible_child(&self) -> Option<gtk::Accessible> {
            // Hide the children in the a11y tree.
            None
        }
    }

    impl ReadReceiptsList {
        /// Set the list of room members.
        fn set_members(&self, members: Option<MemberList>) {
            if *self.members.borrow() == members {
                return;
            }
            let obj = self.obj();

            self.members.replace(members);
            obj.notify_members();

            if let Some(source) = self.source.obj() {
                obj.items_changed(&source, 0, self.list.n_items(), source.n_items());
            }
        }
    }
}

glib::wrapper! {
    /// A widget displaying the read receipts on a message.
    pub struct ReadReceiptsList(ObjectSubclass<imp::ReadReceiptsList>)
        @extends gtk::Widget, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl ReadReceiptsList {
    pub fn new(members: &MemberList) -> Self {
        glib::Object::builder().property("members", members).build()
    }

    /// Set whether this list is active.
    fn set_active(&self, active: bool) {
        if self.active() == active {
            return;
        }

        self.imp().active.set(active);
        self.notify_active();
        self.set_pressed_state(active);
    }

    /// Set the CSS and a11 states.
    fn set_pressed_state(&self, pressed: bool) {
        if pressed {
            self.set_state_flags(gtk::StateFlags::CHECKED, false);
        } else {
            self.unset_state_flags(gtk::StateFlags::CHECKED);
        }

        let tristate = if pressed {
            gtk::AccessibleTristate::True
        } else {
            gtk::AccessibleTristate::False
        };
        self.update_state(&[gtk::accessible::State::Pressed(tristate)]);
    }

    /// Set the read receipts that are used as a source of data.
    pub fn set_source(&self, source: &gio::ListStore) {
        let imp = self.imp();

        let items_changed_handler_id = source.connect_items_changed(clone!(
            #[weak(rename_to = obj)]
            self,
            move |source, pos, removed, added| {
                obj.items_changed(source, pos, removed, added);
            }
        ));
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
                let handler_id = member.connect_display_name_notify(clone!(
                    #[weak(rename_to = obj)]
                    self,
                    move |member| {
                        obj.update_member_tooltip(member);
                    }
                ));

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

        self.set_tooltip_text(text.as_deref());
    }

    fn update_member_tooltip(&self, member: &Member) {
        // Translators: Do NOT translate the content between '{' and '}', this is a
        // variable name.
        let text = gettext_f("Seen by {name}", &[("name", &member.disambiguated_name())]);

        self.set_tooltip_text(Some(&text));
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
    fn show_popover(&self, _n_press: i32, x: f64, y: f64) {
        let list = self.list();
        if list.n_items() == 0 {
            // No popover.
            return;
        }
        self.set_active(true);

        let popover = ReadReceiptsPopover::new(&list);
        popover.set_parent(self);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 0, 0)));
        popover.connect_closed(clone!(
            #[weak(rename_to = obj)]
            self,
            move |popover| {
                popover.unparent();
                obj.set_active(false);
            }
        ));

        popover.popup();
    }
}
