use adw::{prelude::*, subclass::prelude::*};
use gtk::{glib, glib::clone, CompositeTemplate};

mod at_room;
mod search_entry;
mod source;
mod source_row;

pub use self::{
    at_room::AtRoom,
    search_entry::PillSearchEntry,
    source::{PillSource, PillSourceExt, PillSourceImpl},
    source_row::PillSourceRow,
};
use super::{Avatar, JoinRoomDialog, UserProfileDialog};
use crate::{
    prelude::*,
    session::{
        model::{Member, RemoteRoom, Room},
        view::SessionView,
    },
    utils::{key_bindings, BoundObject},
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
        content: TemplateChild<gtk::Box>,
        #[template_child]
        display_name: TemplateChild<gtk::Label>,
        #[template_child]
        avatar: TemplateChild<Avatar>,
        /// The source of the data displayed by this widget.
        #[property(get, set = Self::set_source, explicit_notify, nullable)]
        source: BoundObject<PillSource>,
        /// Whether the pill can be activated.
        #[property(get, set = Self::set_activatable, explicit_notify)]
        activatable: Cell<bool>,
        gesture_click: RefCell<Option<gtk::GestureClick>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Pill {
        const NAME: &'static str = "Pill";
        type Type = super::Pill;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);

            klass.set_layout_manager_type::<gtk::BinLayout>();
            klass.set_css_name("inline-pill");

            klass.install_action("pill.activate", None, |obj, _, _| {
                obj.imp().activate();
            });

            key_bindings::add_activate_bindings(klass, "pill.activate");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for Pill {
        fn constructed(&self) {
            self.parent_constructed();

            self.update_activatable_state();
        }

        fn dispose(&self) {
            self.content.unparent();
        }
    }

    impl WidgetImpl for Pill {}

    impl Pill {
        /// Set the source of the data displayed by this widget.
        fn set_source(&self, source: Option<PillSource>) {
            if self.source.obj() == source {
                return;
            }

            self.source.disconnect_signals();

            if let Some(source) = source {
                let display_name_handler = source.connect_disambiguated_name_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |source| {
                        imp.set_display_name(&source.disambiguated_name());
                    }
                ));
                self.set_display_name(&source.disambiguated_name());

                self.source.set(source, vec![display_name_handler]);
            }

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

                gesture_click.connect_released(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_, _, _, _| {
                        imp.activate();
                    }
                ));

                obj.add_controller(gesture_click.clone());
                self.gesture_click.replace(Some(gesture_click));
            }

            self.update_activatable_state();
            obj.notify_activatable();
        }

        fn update_activatable_state(&self) {
            let obj = self.obj();
            let activatable = self.activatable.get();

            obj.action_set_enabled("pill.activate", activatable);
            obj.set_focusable(activatable);

            let role = if activatable {
                gtk::AccessibleRole::Link
            } else {
                gtk::AccessibleRole::Group
            };
            obj.set_accessible_role(role);

            if activatable {
                obj.add_css_class("activatable");
            } else {
                obj.remove_css_class("activatable");
            }
        }

        /// Set the display name of this pill.
        fn set_display_name(&self, label: &str) {
            // We ellipsize the string manually because GtkTextView uses the minimum width.
            // Show 30 characters max.
            let mut maybe_ellipsized = label.chars().take(30).collect::<String>();

            let is_ellipsized = maybe_ellipsized.len() < label.len();
            if is_ellipsized {
                maybe_ellipsized.append_ellipsis();
            }

            self.display_name.set_label(&maybe_ellipsized);
        }

        /// Activate the pill.
        ///
        /// This opens a known room or opens the profile of a user or unknown
        /// room.
        fn activate(&self) {
            let Some(source) = self.source.obj() else {
                return;
            };
            let obj = self.obj();

            if let Some(member) = source.downcast_ref::<Member>() {
                let dialog = UserProfileDialog::new();
                dialog.set_room_member(member.clone());
                dialog.present(Some(&*obj));
            } else if let Some(room) = source.downcast_ref::<Room>() {
                let Some(session_view) = obj
                    .ancestor(SessionView::static_type())
                    .and_downcast::<SessionView>()
                else {
                    return;
                };

                session_view.select_room(room.clone());
            } else if let Ok(room) = source.downcast::<RemoteRoom>() {
                let Some(session) = room.session() else {
                    return;
                };

                let dialog = JoinRoomDialog::new(&session);
                dialog.set_room(room);
                dialog.present(Some(&*obj));
            }
        }
    }
}

glib::wrapper! {
    /// Inline widget displaying an emphasized `PillSource`.
    pub struct Pill(ObjectSubclass<imp::Pill>)
        @extends gtk::Widget, @implements gtk::Accessible;
}

impl Pill {
    /// Create a pill with the given source.
    pub fn new(source: &impl IsA<PillSource>) -> Self {
        glib::Object::builder().property("source", source).build()
    }
}
