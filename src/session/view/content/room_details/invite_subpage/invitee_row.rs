use adw::subclass::prelude::BinImpl;
use gtk::{glib, prelude::*, subclass::prelude::*, CompositeTemplate};

use super::Invitee;

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;
    use crate::utils::template_callbacks::TemplateCallbacks;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_details/invite_subpage/invitee_row.ui"
    )]
    #[properties(wrapper_type = super::InviteeRow)]
    pub struct InviteeRow {
        /// The user displayed by this row.
        #[property(get, set = Self::set_user, explicit_notify, nullable)]
        pub user: RefCell<Option<Invitee>>,
        pub binding: RefCell<Option<glib::Binding>>,
        #[template_child]
        pub check_button: TemplateChild<gtk::CheckButton>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for InviteeRow {
        const NAME: &'static str = "ContentInviteInviteeRow";
        type Type = super::InviteeRow;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            TemplateCallbacks::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for InviteeRow {}

    impl WidgetImpl for InviteeRow {}
    impl BinImpl for InviteeRow {}

    impl InviteeRow {
        /// Set the user displayed by this row.
        fn set_user(&self, user: Option<Invitee>) {
            if *self.user.borrow() == user {
                return;
            }

            if let Some(binding) = self.binding.take() {
                binding.unbind();
            }

            if let Some(user) = &user {
                // We can't use `gtk::Expression` because we need a bidirectional binding
                let binding = user
                    .bind_property("invited", &*self.check_button, "active")
                    .sync_create()
                    .bidirectional()
                    .build();

                self.binding.replace(Some(binding));
            }

            self.user.replace(user);
            self.obj().notify_user();
        }
    }
}

glib::wrapper! {
    /// A row presenting a possible invitee.
    pub struct InviteeRow(ObjectSubclass<imp::InviteeRow>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl InviteeRow {
    pub fn new(user: &Invitee) -> Self {
        glib::Object::builder().property("user", user).build()
    }
}
