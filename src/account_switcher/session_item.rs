use gtk::{self, glib, prelude::*, subclass::prelude::*, CompositeTemplate};

use super::avatar_with_selection::AvatarWithSelection;
use crate::{
    prelude::*,
    session::model::{AvatarData, Session},
    session_list::{FailedSession, SessionInfo},
};

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/org/gnome/Fractal/ui/account_switcher/session_item.ui")]
    pub struct SessionItemRow {
        #[template_child]
        pub avatar: TemplateChild<AvatarWithSelection>,
        #[template_child]
        pub display_name: TemplateChild<gtk::Label>,
        #[template_child]
        pub user_id: TemplateChild<gtk::Label>,
        #[template_child]
        pub state_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub error_image: TemplateChild<gtk::Image>,
        /// The session this item represents.
        pub session: glib::WeakRef<SessionInfo>,
        pub user_bindings: RefCell<Vec<glib::Binding>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SessionItemRow {
        const NAME: &'static str = "SessionItemRow";
        type Type = super::SessionItemRow;
        type ParentType = gtk::ListBoxRow;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for SessionItemRow {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
                vec![
                    glib::ParamSpecObject::builder::<SessionInfo>("session")
                        .explicit_notify()
                        .build(),
                    glib::ParamSpecBoolean::builder("selected")
                        .explicit_notify()
                        .build(),
                ]
            });

            PROPERTIES.as_ref()
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            let obj = self.obj();

            match pspec.name() {
                "session" => obj.set_session(value.get().unwrap()),
                "selected" => obj.set_selected(value.get().unwrap()),
                _ => unimplemented!(),
            }
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            let obj = self.obj();

            match pspec.name() {
                "session" => obj.session().to_value(),
                "selected" => obj.is_selected().to_value(),
                _ => unimplemented!(),
            }
        }

        fn dispose(&self) {
            for binding in self.user_bindings.take() {
                binding.unbind();
            }
        }
    }

    impl WidgetImpl for SessionItemRow {}
    impl ListBoxRowImpl for SessionItemRow {}
}

glib::wrapper! {
    /// A `GtkListBoxRow` representing a logged-in session.
    pub struct SessionItemRow(ObjectSubclass<imp::SessionItemRow>)
        @extends gtk::Widget, gtk::ListBoxRow, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl SessionItemRow {
    pub fn new(session: &SessionInfo) -> Self {
        glib::Object::builder().property("session", session).build()
    }

    /// Set whether this session is selected.
    pub fn set_selected(&self, selected: bool) {
        let imp = self.imp();

        if imp.avatar.is_selected() == selected {
            return;
        }

        imp.avatar.set_selected(selected);

        if selected {
            imp.display_name.add_css_class("bold");
        } else {
            imp.display_name.remove_css_class("bold");
        }

        self.notify("selected");
    }

    /// Whether this session is selected.
    pub fn is_selected(&self) -> bool {
        self.imp().avatar.is_selected()
    }

    #[template_callback]
    pub fn show_account_settings(&self) {
        let Some(session) = self.session() else {
            return;
        };

        self.activate_action("account-switcher.close", None)
            .unwrap();
        self.activate_action(
            "win.open-account-settings",
            Some(&session.session_id().to_variant()),
        )
        .unwrap();
    }

    /// The session this item represents.
    pub fn session(&self) -> Option<SessionInfo> {
        self.imp().session.upgrade()
    }

    /// Set the session this item represents.
    pub fn set_session(&self, session: Option<&SessionInfo>) {
        if self.session().as_ref() == session {
            return;
        }

        let imp = self.imp();
        for binding in imp.user_bindings.take() {
            binding.unbind();
        }

        if let Some(session) = session {
            if let Some(session) = session.downcast_ref::<Session>() {
                let user = session.user().unwrap();

                let avatar_data_handler = user
                    .bind_property("avatar-data", &*imp.avatar, "data")
                    .sync_create()
                    .build();
                let display_name_handler = user
                    .bind_property("display-name", &*imp.display_name, "label")
                    .sync_create()
                    .build();
                imp.user_bindings
                    .borrow_mut()
                    .extend([avatar_data_handler, display_name_handler]);

                imp.user_id.set_label(session.user_id().as_str());
                imp.user_id.set_visible(true);

                imp.state_stack.set_visible_child_name("settings");
            } else {
                let user_id = session.user_id().as_str();

                let avatar_data = AvatarData::new();
                avatar_data.set_display_name(Some(user_id.to_owned()));
                imp.avatar.set_data(Some(avatar_data));

                imp.display_name.set_label(user_id);
                imp.user_id.set_visible(false);

                if let Some(failed) = session.downcast_ref::<FailedSession>() {
                    imp.error_image
                        .set_tooltip_text(Some(&failed.error().to_user_facing()));
                    imp.state_stack.set_visible_child_name("error");
                } else {
                    imp.state_stack.set_visible_child_name("loading");
                }
            }
        }

        imp.session.set(session);
        self.notify("session");
    }
}
