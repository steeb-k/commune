use gtk::{
    glib::{self, clone, closure},
    prelude::*,
    subclass::prelude::*,
    CompositeTemplate,
};

use super::AccountSwitcherPopover;
use crate::{
    components::Avatar,
    session_list::SessionInfo,
    utils::{template_callbacks::TemplateCallbacks, BoundObjectWeakRef},
    Window,
};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/Fractal/ui/account_switcher/account_switcher_button.ui")]
    pub struct AccountSwitcherButton {
        pub popover: BoundObjectWeakRef<AccountSwitcherPopover>,
        pub watch: RefCell<Option<gtk::ExpressionWatch>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AccountSwitcherButton {
        const NAME: &'static str = "AccountSwitcherButton";
        type Type = super::AccountSwitcherButton;
        type ParentType = gtk::ToggleButton;

        fn class_init(klass: &mut Self::Class) {
            Avatar::ensure_type();
            SessionInfo::ensure_type();

            Self::bind_template(klass);
            TemplateCallbacks::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for AccountSwitcherButton {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            obj.connect_toggled(|obj| {
                obj.handle_toggled();
            });

            let watch = obj
                .property_expression("root")
                .chain_property::<Window>("session-selection")
                .chain_property::<gtk::SingleSelection>("n-items")
                .chain_closure::<bool>(closure!(|_: Option<glib::Object>, n_items: u32| {
                    n_items > 0
                }))
                .bind(&*obj, "visible", glib::Object::NONE);
            self.watch.replace(Some(watch));
        }

        fn dispose(&self) {
            if let Some(watch) = self.watch.take() {
                watch.unwatch();
            }
        }
    }

    impl WidgetImpl for AccountSwitcherButton {}
    impl ButtonImpl for AccountSwitcherButton {}
    impl ToggleButtonImpl for AccountSwitcherButton {}
}

glib::wrapper! {
    /// A button showing the currently selected account and opening the account switcher popover.
    pub struct AccountSwitcherButton(ObjectSubclass<imp::AccountSwitcherButton>)
        @extends gtk::Widget, gtk::Button, gtk::ToggleButton, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl AccountSwitcherButton {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn popover(&self) -> Option<AccountSwitcherPopover> {
        self.imp().popover.obj()
    }

    pub fn set_popover(&self, popover: Option<&AccountSwitcherPopover>) {
        let old_popover = self.popover();

        if old_popover.as_ref() == popover {
            return;
        }

        let imp = self.imp();

        // Reset the state.
        if let Some(popover) = old_popover {
            popover.unparent();
        }
        imp.popover.disconnect_signals();
        self.set_active(false);

        if let Some(popover) = popover {
            // We need to remove the popover from the previous button, if any.
            if let Some(parent) = popover.parent().and_downcast::<AccountSwitcherButton>() {
                parent.set_popover(None);
            }

            let closed_handler = popover.connect_closed(clone!(
                #[weak(rename_to = obj)]
                self,
                move |_| {
                    obj.set_active(false);
                }
            ));

            popover.set_parent(self);
            imp.popover.set(popover, vec![closed_handler]);
        }
    }

    fn handle_toggled(&self) {
        if self.is_active() {
            let Some(window) = self.root().and_downcast::<Window>() else {
                return;
            };

            let popover = window.account_switcher();
            self.set_popover(Some(popover));

            popover.popup();
        } else if let Some(popover) = self.popover() {
            popover.popdown();
        }
    }
}

impl Default for AccountSwitcherButton {
    fn default() -> Self {
        Self::new()
    }
}
