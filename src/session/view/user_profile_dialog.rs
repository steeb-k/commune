use adw::{prelude::*, subclass::prelude::*};
use gtk::{gdk, glib, glib::clone, CompositeTemplate};
use ruma::OwnedUserId;

use super::UserPage;
use crate::{
    components::{Spinner, ToastableWindow},
    prelude::*,
    session::model::{Member, RemoteUser, Room, Session, User},
    spawn,
};

mod imp {
    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/Fractal/ui/session/view/user_profile_dialog.ui")]
    pub struct UserProfileDialog {
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub user_page: TemplateChild<UserPage>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for UserProfileDialog {
        const NAME: &'static str = "UserProfileDialog";
        type Type = super::UserProfileDialog;
        type ParentType = ToastableWindow;

        fn class_init(klass: &mut Self::Class) {
            Spinner::static_type();

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

    impl ObjectImpl for UserProfileDialog {}
    impl WidgetImpl for UserProfileDialog {}
    impl WindowImpl for UserProfileDialog {}
    impl AdwWindowImpl for UserProfileDialog {}
    impl ToastableWindowImpl for UserProfileDialog {}
}

glib::wrapper! {
    /// Dialog to join a room.
    pub struct UserProfileDialog(ObjectSubclass<imp::UserProfileDialog>)
        @extends gtk::Widget, gtk::Window, adw::Window, ToastableWindow, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl UserProfileDialog {
    /// Create a new `UserProfileDialog`.
    pub fn new(parent_window: Option<&impl IsA<gtk::Window>>) -> Self {
        glib::Object::builder::<Self>()
            .property("transient-for", parent_window)
            .build()
    }

    /// Load the user with the given session and user ID.
    pub fn load_user(&self, session: &Session, user_id: OwnedUserId) {
        let imp = self.imp();

        let user = RemoteUser::new(session, user_id);
        imp.user_page.set_user(Some(user.clone()));

        spawn!(clone!(@weak imp, @weak user => async move {
            user.load_profile().await;
            imp.stack.set_visible_child_name("details");
        }));
    }

    /// Set the member to present.
    pub fn set_room_member(&self, room: Room, member: Member) {
        let imp = self.imp();

        imp.user_page.set_room(Some(room));
        imp.user_page.set_user(Some(member.upcast::<User>()));
        imp.stack.set_visible_child_name("details");
    }
}
