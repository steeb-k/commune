use adw::prelude::*;
use gtk::{
    gio,
    glib::{self, clone},
    subclass::prelude::*,
    CompositeTemplate,
};

use crate::{
    components::UserProfileDialog,
    session::view::content::room_history::member_timestamp::{
        row::MemberTimestampRow, MemberTimestamp,
    },
};

mod imp {
    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_history/read_receipts_list/read_receipts_popover.ui"
    )]
    #[properties(wrapper_type = super::ReadReceiptsPopover)]
    pub struct ReadReceiptsPopover {
        #[template_child]
        pub list: TemplateChild<gtk::ListView>,
        /// The receipts to display.
        #[property(get, set = Self::set_receipts, construct_only)]
        pub receipts: glib::WeakRef<gio::ListStore>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ReadReceiptsPopover {
        const NAME: &'static str = "ContentReadReceiptsPopover";
        type Type = super::ReadReceiptsPopover;
        type ParentType = gtk::Popover;

        fn class_init(klass: &mut Self::Class) {
            MemberTimestampRow::ensure_type();

            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for ReadReceiptsPopover {
        fn constructed(&self) {
            self.parent_constructed();
        }
    }

    impl WidgetImpl for ReadReceiptsPopover {}
    impl PopoverImpl for ReadReceiptsPopover {}

    impl ReadReceiptsPopover {
        /// Set the receipts to display.
        fn set_receipts(&self, receipts: gio::ListStore) {
            let obj = self.obj();

            self.receipts.set(Some(&receipts));
            self.list
                .set_model(Some(&gtk::NoSelection::new(Some(receipts))));
            self.list.connect_activate(clone!(
                #[weak]
                obj,
                move |_, pos| {
                    let Some(member) = obj
                        .receipts()
                        .and_then(|list| list.item(pos))
                        .and_downcast::<MemberTimestamp>()
                        .and_then(|ts| ts.member())
                    else {
                        return;
                    };

                    let dialog = UserProfileDialog::new();
                    dialog.set_room_member(member);
                    dialog.present(&obj);
                    obj.popdown();
                }
            ));
        }
    }
}

glib::wrapper! {
    /// A popover to display the read receipts on an event.
    pub struct ReadReceiptsPopover(ObjectSubclass<imp::ReadReceiptsPopover>)
        @extends gtk::Widget, gtk::Popover;
}

impl ReadReceiptsPopover {
    /// Constructs a new `ReadReceiptsPopover` with the given receipts list.
    pub fn new(receipts: &gio::ListStore) -> Self {
        glib::Object::builder()
            .property("receipts", receipts)
            .build()
    }
}
