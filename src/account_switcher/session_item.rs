use gtk::{self, glib, prelude::*, subclass::prelude::*, CompositeTemplate};

use super::avatar_with_selection::AvatarWithSelection;
use crate::{
    prelude::*,
    session::model::{AvatarData, Session},
    session_list::{FailedSession, SessionInfo},
};

mod imp {
    use std::{cell::RefCell, marker::PhantomData};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/account_switcher/session_item.ui")]
    #[properties(wrapper_type = super::SessionItemRow)]
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
        #[property(get, set = Self::set_session, explicit_notify)]
        pub session: glib::WeakRef<SessionInfo>,
        pub user_bindings: RefCell<Vec<glib::Binding>>,
        /// Whether this session is selected.
        #[property(get = Self::is_selected, set = Self::set_selected, explicit_notify)]
        pub selected: PhantomData<bool>,
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

    #[glib::derived_properties]
    impl ObjectImpl for SessionItemRow {
        fn dispose(&self) {
            for binding in self.user_bindings.take() {
                binding.unbind();
            }
        }
    }

    impl WidgetImpl for SessionItemRow {}
    impl ListBoxRowImpl for SessionItemRow {}

    impl SessionItemRow {
        /// Whether this session is selected.
        fn is_selected(&self) -> bool {
            self.avatar.selected()
        }

        /// Set whether this session is selected.
        fn set_selected(&self, selected: bool) {
            if self.is_selected() == selected {
                return;
            }

            self.avatar.set_selected(selected);

            if selected {
                self.display_name.add_css_class("bold");
            } else {
                self.display_name.remove_css_class("bold");
            }

            self.obj().notify_selected();
        }

        /// Set the session this item represents.
        fn set_session(&self, session: Option<&SessionInfo>) {
            if self.session.upgrade().as_ref() == session {
                return;
            }

            for binding in self.user_bindings.take() {
                binding.unbind();
            }

            if let Some(session) = session {
                if let Some(session) = session.downcast_ref::<Session>() {
                    let user = session.user();

                    let avatar_data_handler = user
                        .bind_property("avatar-data", &*self.avatar, "data")
                        .sync_create()
                        .build();
                    let display_name_handler = user
                        .bind_property("display-name", &*self.display_name, "label")
                        .sync_create()
                        .build();
                    self.user_bindings
                        .borrow_mut()
                        .extend([avatar_data_handler, display_name_handler]);

                    self.user_id.set_label(session.user_id().as_str());
                    self.user_id.set_visible(true);

                    self.state_stack.set_visible_child_name("settings");
                } else {
                    let user_id = session.user_id().to_string();

                    let avatar_data = AvatarData::new();
                    avatar_data.set_display_name(Some(user_id.clone()));
                    self.avatar.set_data(Some(avatar_data));

                    self.display_name.set_label(&user_id);
                    self.user_id.set_visible(false);

                    if let Some(failed) = session.downcast_ref::<FailedSession>() {
                        self.error_image
                            .set_tooltip_text(Some(&failed.error().to_user_facing()));
                        self.state_stack.set_visible_child_name("error");
                    } else {
                        self.state_stack.set_visible_child_name("loading");
                    }
                }
            }

            self.session.set(session);
            self.obj().notify_session();
        }
    }
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
}
