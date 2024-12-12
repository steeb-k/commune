use adw::{prelude::*, subclass::prelude::*};
use gtk::{glib, glib::clone, CompositeTemplate};
use ruma::OwnedUserId;

use super::ToastableDialog;
use crate::{
    components::UserPage,
    prelude::*,
    session::model::{Member, RemoteUser, Session, User},
    spawn,
};

mod imp {
    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/Fractal/ui/components/dialogs/user_profile.ui")]
    pub struct UserProfileDialog {
        #[template_child]
        stack: TemplateChild<gtk::Stack>,
        #[template_child]
        user_page: TemplateChild<UserPage>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for UserProfileDialog {
        const NAME: &'static str = "UserProfileDialog";
        type Type = super::UserProfileDialog;
        type ParentType = ToastableDialog;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for UserProfileDialog {}
    impl WidgetImpl for UserProfileDialog {}
    impl AdwDialogImpl for UserProfileDialog {}
    impl ToastableDialogImpl for UserProfileDialog {}

    impl UserProfileDialog {
        /// Load the user with the given session and user ID.
        pub(super) fn load_user(&self, session: &Session, user_id: OwnedUserId) {
            let user = RemoteUser::new(session, user_id);
            self.user_page.set_user(Some(user.clone()));

            spawn!(clone!(
                #[weak(rename_to = imp)]
                self,
                async move {
                    user.load_profile().await;
                    imp.stack.set_visible_child_name("details");
                }
            ));
        }

        /// Set the member to present.
        pub(super) fn set_room_member(&self, member: Member) {
            self.user_page.set_user(Some(member.upcast::<User>()));
            self.stack.set_visible_child_name("details");
        }
    }
}

glib::wrapper! {
    /// Dialog to view a user's profile.
    pub struct UserProfileDialog(ObjectSubclass<imp::UserProfileDialog>)
        @extends gtk::Widget, adw::Dialog, ToastableDialog, @implements gtk::Accessible;
}

impl UserProfileDialog {
    /// Create a new `UserProfileDialog`.
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Load the user with the given session and user ID.
    pub(crate) fn load_user(&self, session: &Session, user_id: OwnedUserId) {
        self.imp().load_user(session, user_id);
    }

    /// Set the member to present.
    pub(crate) fn set_room_member(&self, member: Member) {
        self.imp().set_room_member(member);
    }
}
