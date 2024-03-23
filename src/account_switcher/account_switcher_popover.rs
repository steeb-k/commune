use gtk::{
    glib::{self, clone},
    prelude::*,
    subclass::prelude::*,
    CompositeTemplate,
};

use super::session_item::SessionItemRow;
use crate::utils::BoundObjectWeakRef;

mod imp {
    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/account_switcher/account_switcher_popover.ui")]
    #[properties(wrapper_type = super::AccountSwitcherPopover)]
    pub struct AccountSwitcherPopover {
        #[template_child]
        pub sessions: TemplateChild<gtk::ListBox>,
        /// The model containing the logged-in sessions selection.
        #[property(get, set = Self::set_session_selection, explicit_notify, nullable)]
        pub session_selection: BoundObjectWeakRef<gtk::SingleSelection>,
        /// The selected row.
        pub selected_row: glib::WeakRef<SessionItemRow>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AccountSwitcherPopover {
        const NAME: &'static str = "AccountSwitcherPopover";
        type Type = super::AccountSwitcherPopover;
        type ParentType = gtk::Popover;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);

            klass.install_action("account-switcher.close", None, |obj, _, _| {
                obj.popdown();
            });
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for AccountSwitcherPopover {}

    impl WidgetImpl for AccountSwitcherPopover {}
    impl PopoverImpl for AccountSwitcherPopover {}

    impl AccountSwitcherPopover {
        /// Set the model containing the logged-in sessions selection.
        fn set_session_selection(&self, selection: Option<gtk::SingleSelection>) {
            if selection == self.session_selection.obj() {
                return;
            }
            let obj = self.obj();

            self.session_selection.disconnect_signals();

            self.sessions.bind_model(selection.as_ref(), |session| {
                let row = SessionItemRow::new(session.downcast_ref().unwrap());
                row.upcast()
            });

            if let Some(selection) = &selection {
                let selected_handler =
                    selection.connect_selected_item_notify(clone!(@weak obj => move |selection| {
                        obj.update_selected_item(selection.selected());
                    }));
                obj.update_selected_item(selection.selected());

                self.session_selection
                    .set(selection, vec![selected_handler]);
            }

            obj.notify_session_selection();
        }
    }
}

glib::wrapper! {
    pub struct AccountSwitcherPopover(ObjectSubclass<imp::AccountSwitcherPopover>)
        @extends gtk::Widget, gtk::Popover, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl AccountSwitcherPopover {
    pub fn new() -> Self {
        glib::Object::new()
    }

    fn selected_row(&self) -> Option<SessionItemRow> {
        self.imp().selected_row.upgrade()
    }

    /// Select the given row in the session list.
    #[template_callback]
    fn select_row(&self, row: gtk::ListBoxRow) {
        self.popdown();

        let Some(selection) = self.session_selection() else {
            return;
        };

        // The index is -1 when it is not in a GtkListBox, but we just got it from the
        // GtkListBox so we can safely assume it's a valid u32.
        selection.set_selected(row.index() as u32);
    }

    /// Update the selected item in the session list.
    fn update_selected_item(&self, selected: u32) {
        let imp = self.imp();

        let old_selected = self.selected_row();
        let new_selected = if selected == gtk::INVALID_LIST_POSITION {
            None
        } else {
            imp.sessions
                .row_at_index(selected as i32)
                .and_downcast::<SessionItemRow>()
        };

        if old_selected == new_selected {
            return;
        }

        if let Some(row) = &old_selected {
            row.set_selected(false);
        }
        if let Some(row) = &new_selected {
            row.set_selected(true);
        }

        imp.selected_row.set(new_selected.as_ref());
    }
}

impl Default for AccountSwitcherPopover {
    fn default() -> Self {
        Self::new()
    }
}
