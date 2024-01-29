use adw::subclass::prelude::*;
use gtk::{glib, glib::clone, prelude::*, CompositeTemplate};

mod source;
mod source_row;

pub use self::{
    source::{PillSource, PillSourceExt, PillSourceImpl},
    source_row::PillSourceRow,
};
use super::{Avatar, JoinRoomDialog, UserProfileDialog};
use crate::{
    session::{
        model::{Member, RemoteRoom, Room},
        view::SessionView,
    },
    utils::add_activate_binding_action,
};

mod imp {
    use std::cell::{Cell, RefCell};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/pill/mod.ui")]
    #[properties(wrapper_type = super::Pill)]
    pub struct Pill {
        #[template_child]
        pub display_name: TemplateChild<gtk::Label>,
        #[template_child]
        pub avatar: TemplateChild<Avatar>,
        /// The source of the data displayed by this widget.
        #[property(get, set = Self::set_source, explicit_notify, nullable)]
        pub source: RefCell<Option<PillSource>>,
        /// Whether the pill can be activated.
        #[property(get, set = Self::set_activatable, explicit_notify)]
        pub activatable: Cell<bool>,
        gesture_click: RefCell<Option<gtk::GestureClick>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Pill {
        const NAME: &'static str = "Pill";
        type Type = super::Pill;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);

            klass.install_action("pill.activate", None, move |widget, _, _| {
                widget.activate();
            });

            add_activate_binding_action(klass, "pill.activate");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for Pill {}

    impl WidgetImpl for Pill {}
    impl BinImpl for Pill {}

    impl Pill {
        /// Set the source of the data displayed by this widget.
        fn set_source(&self, source: Option<PillSource>) {
            if *self.source.borrow() == source {
                return;
            }

            self.source.replace(source);
            self.obj().notify_source();
        }

        /// Set whether this widget can be activated.
        fn set_activatable(&self, activatable: bool) {
            if self.activatable.get() == activatable {
                return;
            }
            let obj = self.obj();

            if let Some(gesture_click) = self.gesture_click.take() {
                obj.remove_controller(&gesture_click);
            }

            self.activatable.set(activatable);

            if activatable {
                let gesture_click = gtk::GestureClick::new();

                gesture_click.connect_released(clone!(@weak obj => move |_, _, _, _| {
                    obj.activate();
                }));

                obj.add_controller(gesture_click.clone());
                self.gesture_click.replace(Some(gesture_click));
            }

            obj.action_set_enabled("pill.activate", activatable);
            obj.set_focusable(activatable);
            obj.notify_activatable();
        }
    }
}

glib::wrapper! {
    /// Inline widget displaying an emphasized `PillSource`.
    pub struct Pill(ObjectSubclass<imp::Pill>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl Pill {
    /// Create a pill with the given source.
    pub fn new(source: &impl IsA<PillSource>) -> Self {
        glib::Object::builder().property("source", source).build()
    }

    /// Activate the pill.
    ///
    /// This opens a known room or opens the profile of a user or unknown room.
    fn activate(&self) {
        let Some(source) = self.source() else {
            return;
        };
        let Some(window) = self.root().and_downcast::<gtk::Window>() else {
            return;
        };

        if let Some(member) = source.downcast_ref::<Member>() {
            let dialog = UserProfileDialog::new(Some(&window));
            dialog.set_room_member(member.clone());
            dialog.present();
        } else if let Some(room) = source.downcast_ref::<Room>() {
            let Some(session_view) = self
                .ancestor(SessionView::static_type())
                .and_downcast::<SessionView>()
            else {
                return;
            };

            session_view.select_room(Some(room.clone()));
        } else if let Ok(room) = source.downcast::<RemoteRoom>() {
            let Some(session) = room.session() else {
                return;
            };

            let dialog = JoinRoomDialog::new(Some(&window), &session);
            dialog.set_room(room);
            dialog.present();
        }
    }
}
