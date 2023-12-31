use adw::{prelude::*, subclass::prelude::*};
use futures_channel::oneshot;
use gtk::{gdk, glib, CompositeTemplate};
use tracing::error;

mod account_row;

use self::account_row::AccountRow;
use crate::session_list::{SessionInfo, SessionList};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/account_chooser_dialog/mod.ui")]
    #[properties(wrapper_type = super::AccountChooserDialog)]
    pub struct AccountChooserDialog {
        #[template_child]
        pub accounts: TemplateChild<gtk::ListBox>,
        /// The list of logged-in sessions.
        #[property(get, set = Self::set_session_list, construct)]
        pub session_list: glib::WeakRef<SessionList>,
        pub sender: RefCell<Option<oneshot::Sender<Option<String>>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AccountChooserDialog {
        const NAME: &'static str = "AccountChooserDialog";
        type Type = super::AccountChooserDialog;
        type ParentType = adw::Window;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);

            klass.add_binding_action(
                gdk::Key::Escape,
                gdk::ModifierType::empty(),
                "window.close",
                None,
            );
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for AccountChooserDialog {}

    impl WidgetImpl for AccountChooserDialog {}

    impl WindowImpl for AccountChooserDialog {
        fn close_request(&self) -> glib::Propagation {
            if let Some(sender) = self.sender.take() {
                if sender.send(None).is_err() {
                    error!("Failed to send selected session");
                }
            }

            glib::Propagation::Proceed
        }
    }

    impl AdwWindowImpl for AccountChooserDialog {}

    impl AccountChooserDialog {
        /// Set the list of logged-in sessions.
        fn set_session_list(&self, session_list: SessionList) {
            self.accounts.bind_model(Some(&session_list), |session| {
                let row = AccountRow::new(session.downcast_ref().unwrap());
                row.upcast()
            });

            self.session_list.set(Some(&session_list));
        }
    }
}

glib::wrapper! {
    /// A dialog to choose an account among the ones that are connected.
    ///
    /// Should be used by calling [`Self::choose_account()`].
    pub struct AccountChooserDialog(ObjectSubclass<imp::AccountChooserDialog>)
        @extends gtk::Widget, gtk::Root, gtk::Window, adw::Window, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl AccountChooserDialog {
    pub fn new(parent_window: Option<&impl IsA<gtk::Window>>, session_list: &SessionList) -> Self {
        glib::Object::builder()
            .property("transient-for", parent_window)
            .property("session-list", session_list)
            .build()
    }

    /// Open this dialog to choose an account.
    pub async fn choose_account(&self) -> Option<String> {
        let (sender, receiver) = oneshot::channel();
        self.imp().sender.replace(Some(sender));

        self.present();

        receiver.await.ok().flatten()
    }

    /// Select the given row in the session list.
    #[template_callback]
    fn select_row(&self, row: gtk::ListBoxRow) {
        if let Some(sender) = self.imp().sender.take() {
            // The index is -1 when it is not in a GtkListBox, but we just got it from the
            // GtkListBox so we can safely assume it's a valid u32.
            let index = row.index() as u32;

            let session_id = self
                .session_list()
                .and_then(|l| l.item(index))
                .and_downcast::<SessionInfo>()
                .map(|s| s.session_id());

            if sender.send(session_id).is_err() {
                error!("Failed to send selected session");
            }
        }

        self.close();
    }
}
