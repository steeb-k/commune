mod explore;
mod invite;
mod room_details;
mod room_history;

use adw::subclass::prelude::*;
use gtk::{glib, glib::clone, prelude::*, CompositeTemplate};

use self::{
    explore::Explore, invite::Invite, room_details::RoomDetails, room_history::RoomHistory,
};
use crate::{
    session::model::{
        IdentityVerification, Room, RoomType, Session, SidebarIconItem, SidebarIconItemType,
    },
    verification_view::IdentityVerificationView,
};

mod imp {
    use std::cell::{Cell, RefCell};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session/view/content/mod.ui")]
    #[properties(wrapper_type = super::Content)]
    pub struct Content {
        /// The current session.
        #[property(get, set = Self::set_session, explicit_notify, nullable)]
        pub session: glib::WeakRef<Session>,
        /// Whether this is the only visible view, i.e. there is no sidebar.
        #[property(get, set)]
        pub only_view: Cell<bool>,
        pub item_binding: RefCell<Option<glib::Binding>>,
        /// The item currently displayed.
        #[property(get, set = Self::set_item, explicit_notify, nullable)]
        pub item: RefCell<Option<glib::Object>>,
        pub signal_handler: RefCell<Option<glib::SignalHandlerId>>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub room_history: TemplateChild<RoomHistory>,
        #[template_child]
        pub invite: TemplateChild<Invite>,
        #[template_child]
        pub explore: TemplateChild<Explore>,
        #[template_child]
        pub empty_page: TemplateChild<adw::ToolbarView>,
        #[template_child]
        pub verification_page: TemplateChild<adw::ToolbarView>,
        #[template_child]
        pub identity_verification_widget: TemplateChild<IdentityVerificationView>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Content {
        const NAME: &'static str = "Content";
        type Type = super::Content;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            klass.set_accessible_role(gtk::AccessibleRole::Group);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for Content {
        fn constructed(&self) {
            self.parent_constructed();

            self.stack
                .connect_visible_child_notify(clone!(@weak self as imp => move |stack| {
                    if stack.visible_child().as_ref() != Some(imp.verification_page.upcast_ref::<gtk::Widget>()) {
                        imp.identity_verification_widget.set_verification(None::<IdentityVerification>);
                    }
                }));

            if let Some(binding) = self.item_binding.take() {
                binding.unbind()
            }
        }
    }

    impl WidgetImpl for Content {}

    impl NavigationPageImpl for Content {
        fn hidden(&self) {
            self.obj().set_item(None::<glib::Object>);
        }
    }

    impl Content {
        /// Set the current session.
        fn set_session(&self, session: Option<Session>) {
            if session == self.session.upgrade() {
                return;
            }
            let obj = self.obj();

            if let Some(binding) = self.item_binding.take() {
                binding.unbind();
            }

            if let Some(session) = &session {
                let item_binding = session
                    .sidebar_list_model()
                    .selection_model()
                    .bind_property("selected-item", &*obj, "item")
                    .sync_create()
                    .bidirectional()
                    .build();

                self.item_binding.replace(Some(item_binding));
            }

            self.session.set(session.as_ref());
            obj.notify_session();
        }

        /// Set the item currently displayed.
        fn set_item(&self, item: Option<glib::Object>) {
            if *self.item.borrow() == item {
                return;
            }
            let obj = self.obj();

            if let Some(item) = self.item.take() {
                if let Some(signal_handler) = self.signal_handler.take() {
                    item.disconnect(signal_handler);
                }
            }

            if let Some(item) = &item {
                if let Some(room) = item.downcast_ref::<Room>() {
                    let handler_id = room.connect_category_notify(clone!(@weak obj => move |_| {
                        obj.update_visible_child();
                    }));

                    self.signal_handler.replace(Some(handler_id));
                } else if let Some(verification) = item.downcast_ref::<IdentityVerification>() {
                    let handler_id = verification.connect_dismiss(clone!(@weak obj => move |_| {
                        tracing::debug!("Dismiss verification");
                        obj.set_item(None::<glib::Object>);
                    }));
                    self.signal_handler.replace(Some(handler_id));
                }
            }

            self.item.replace(item);
            obj.update_visible_child();
            obj.notify_item();
        }
    }
}

glib::wrapper! {
    /// A view displaying the selected content in the sidebar.
    pub struct Content(ObjectSubclass<imp::Content>)
        @extends gtk::Widget, adw::NavigationPage, @implements gtk::Accessible;
}

impl Content {
    pub fn new(session: &Session) -> Self {
        glib::Object::builder().property("session", session).build()
    }

    pub fn handle_paste_action(&self) {
        let imp = self.imp();
        if imp
            .stack
            .visible_child()
            .as_ref()
            .map(|c| c == imp.room_history.upcast_ref::<gtk::Widget>())
            .unwrap_or_default()
        {
            imp.room_history.handle_paste_action();
        }
    }

    /// Update the visible child according to the current item.
    fn update_visible_child(&self) {
        let imp = self.imp();

        match self.item() {
            None => {
                imp.stack.set_visible_child(&*imp.empty_page);
            }
            Some(o) if o.is::<Room>() => {
                if let Ok(room) = o.downcast::<Room>() {
                    if room.category() == RoomType::Invited {
                        imp.invite.set_room(Some(room));
                        imp.stack.set_visible_child(&*imp.invite);
                    } else {
                        imp.room_history.set_room(Some(room));
                        imp.stack.set_visible_child(&*imp.room_history);
                    }
                }
            }
            Some(o)
                if o.downcast_ref::<SidebarIconItem>()
                    .is_some_and(|i| i.item_type() == SidebarIconItemType::Explore) =>
            {
                imp.explore.init();
                imp.stack.set_visible_child(&*imp.explore);
            }
            Some(o) if o.is::<IdentityVerification>() => {
                if let Ok(verification) = o.downcast::<IdentityVerification>() {
                    imp.identity_verification_widget
                        .set_verification(Some(verification));
                    imp.stack.set_visible_child(&*imp.verification_page);
                }
            }
            _ => {}
        }
    }
}
