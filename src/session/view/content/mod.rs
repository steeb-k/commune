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
    identity_verification_view::IdentityVerificationView,
    session::model::{
        IdentityVerification, Room, RoomCategory, Session, SidebarIconItem, SidebarIconItemType,
    },
};

/// A page of the content stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumString, strum::AsRefStr)]
#[strum(serialize_all = "kebab-case")]
pub enum ContentPage {
    /// The placeholder page when no content is presented.
    Empty,
    /// The history of the selected room.
    RoomHistory,
    /// The selected room invite.
    Invite,
    /// The explore page.
    Explore,
    /// The selected identity verification.
    Verification,
}

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
        pub empty_page_header_bar: TemplateChild<adw::HeaderBar>,
        #[template_child]
        pub verification_page: TemplateChild<adw::ToolbarView>,
        #[template_child]
        pub verification_page_header_bar: TemplateChild<adw::HeaderBar>,
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

            self.stack.connect_visible_child_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    if imp.visible_page() != ContentPage::Verification {
                        imp.identity_verification_widget
                            .set_verification(None::<IdentityVerification>);
                    }
                }
            ));

            if let Some(binding) = self.item_binding.take() {
                binding.unbind();
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
        /// The visible page of the content.
        pub(super) fn visible_page(&self) -> ContentPage {
            self.stack
                .visible_child_name()
                .and_then(|s| s.as_str().try_into().ok())
                .unwrap()
        }

        /// Set the current session.
        fn set_session(&self, session: Option<&Session>) {
            if session == self.session.upgrade().as_ref() {
                return;
            }
            let obj = self.obj();

            if let Some(binding) = self.item_binding.take() {
                binding.unbind();
            }

            if let Some(session) = session {
                let item_binding = session
                    .sidebar_list_model()
                    .selection_model()
                    .bind_property("selected-item", &*obj, "item")
                    .sync_create()
                    .bidirectional()
                    .build();

                self.item_binding.replace(Some(item_binding));
            }

            self.session.set(session);
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
                    let handler_id = room.connect_category_notify(clone!(
                        #[weak]
                        obj,
                        move |_| {
                            obj.update_visible_child();
                        }
                    ));

                    self.signal_handler.replace(Some(handler_id));
                } else if let Some(verification) = item.downcast_ref::<IdentityVerification>() {
                    let handler_id = verification.connect_dismiss(clone!(
                        #[weak]
                        obj,
                        move |_| {
                            obj.set_item(None::<glib::Object>);
                        }
                    ));
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

    /// Set the visible page of the content.
    fn set_visible_page(&self, name: ContentPage) {
        self.imp().stack.set_visible_child_name(name.as_ref());
    }

    /// Handle a paste action.
    pub fn handle_paste_action(&self) {
        let imp = self.imp();
        if imp.visible_page() == ContentPage::RoomHistory {
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
                    if room.category() == RoomCategory::Invited {
                        imp.invite.set_room(Some(room));
                        self.set_visible_page(ContentPage::Invite);
                    } else {
                        imp.room_history.set_timeline(Some(room.timeline()));
                        self.set_visible_page(ContentPage::RoomHistory);
                    }
                }
            }
            Some(o)
                if o.downcast_ref::<SidebarIconItem>()
                    .is_some_and(|i| i.item_type() == SidebarIconItemType::Explore) =>
            {
                imp.explore.init();
                self.set_visible_page(ContentPage::Explore);
            }
            Some(o) if o.is::<IdentityVerification>() => {
                if let Ok(verification) = o.downcast::<IdentityVerification>() {
                    imp.identity_verification_widget
                        .set_verification(Some(verification));
                    self.set_visible_page(ContentPage::Verification);
                }
            }
            _ => {}
        }
    }

    /// All the header bars of the children of the content.
    pub fn header_bars(&self) -> [&adw::HeaderBar; 5] {
        let imp = self.imp();
        [
            &imp.empty_page_header_bar,
            imp.room_history.header_bar(),
            imp.invite.header_bar(),
            imp.explore.header_bar(),
            &imp.verification_page_header_bar,
        ]
    }
}
