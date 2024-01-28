use gtk::{glib, prelude::*, subclass::prelude::*, CompositeTemplate};

use super::{Avatar, PillSource};
use crate::session::model::{Member, Room};

mod imp {
    use std::{cell::RefCell, marker::PhantomData};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/pill_source_row.ui")]
    #[properties(wrapper_type = super::PillSourceRow)]
    pub struct PillSourceRow {
        #[template_child]
        pub avatar: TemplateChild<Avatar>,
        #[template_child]
        pub display_name: TemplateChild<gtk::Label>,
        #[template_child]
        pub id: TemplateChild<gtk::Label>,
        /// The source of the data displayed by this row.
        pub source: RefCell<Option<PillSource>>,
        /// The room member presented by this row.
        #[property(get = Self::member, set = Self::set_member, explicit_notify, nullable)]
        pub member: PhantomData<Option<Member>>,
        /// The room presented by this row.
        #[property(get = Self::room, set = Self::set_room, explicit_notify, nullable)]
        pub room: PhantomData<Option<Room>>,
        bindings: RefCell<Option<[glib::Binding; 2]>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PillSourceRow {
        const NAME: &'static str = "PillSourceRow";
        type Type = super::PillSourceRow;
        type ParentType = gtk::ListBoxRow;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for PillSourceRow {
        fn dispose(&self) {
            for binding in self.bindings.take().iter().flatten() {
                binding.unbind();
            }
        }
    }

    impl WidgetImpl for PillSourceRow {}
    impl ListBoxRowImpl for PillSourceRow {}

    impl PillSourceRow {
        /// Set the source of the data displayed by this row.
        pub(super) fn set_source(&self, source: Option<PillSource>) {
            for binding in self.bindings.take().iter().flatten() {
                binding.unbind();
            }

            if let Some(source) = &source {
                let display_name_binding = source.bind_display_name(&*self.display_name, "label");
                let id_binding = source.bind_identifier(&*self.id, "label");
                self.bindings
                    .replace(Some([display_name_binding, id_binding]));
            }

            self.avatar
                .set_data(source.as_ref().map(|s| s.avatar_data()));
            self.source.replace(source);

            let obj = self.obj();
            obj.notify_member();
            obj.notify_room();
        }

        /// The room member displayed by this row.
        fn member(&self) -> Option<Member> {
            match self.source.borrow().as_ref()? {
                PillSource::User(user) => user.clone().downcast().ok(),
                _ => None,
            }
        }

        /// Set the room member displayed by this row.
        fn set_member(&self, member: Option<Member>) {
            self.set_source(member.and_upcast().map(PillSource::User))
        }

        /// The room displayed by this row.
        fn room(&self) -> Option<Room> {
            match self.source.borrow().as_ref()? {
                PillSource::Room(room) => Some(room.clone()),
                _ => None,
            }
        }

        /// Set the room displayed by this row.
        fn set_room(&self, room: Option<Room>) {
            self.set_source(room.map(PillSource::Room))
        }
    }
}

glib::wrapper! {
    /// A list row to display a [`PillSource`].
    pub struct PillSourceRow(ObjectSubclass<imp::PillSourceRow>)
        @extends gtk::Widget, gtk::ListBoxRow;
}

impl PillSourceRow {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// The source of the data displayed by this row
    pub fn source(&self) -> Option<PillSource> {
        self.imp().source.borrow().clone()
    }

    /// Set the source of the data displayed by this row.
    pub fn set_source(&self, source: Option<PillSource>) {
        self.imp().set_source(source);
    }
}

impl Default for PillSourceRow {
    fn default() -> Self {
        Self::new()
    }
}
