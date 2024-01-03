use std::time::Duration;

use adw::subclass::prelude::*;
use gettextrs::gettext;
use gtk::{
    gdk, gio, glib,
    glib::{clone, closure, closure_local},
    prelude::*,
    CompositeTemplate,
};
use tracing::{debug, error};

use super::{ActionButton, ActionState, ImagePaintable};
use crate::{
    session::model::{AvatarData, AvatarImage},
    spawn, toast,
    utils::expression,
};

/// The state of the editable avatar.
#[derive(Debug, Default, Hash, Eq, PartialEq, Clone, Copy, glib::Enum)]
#[repr(u32)]
#[enum_type(name = "EditableAvatarState")]
pub enum EditableAvatarState {
    /// Nothing is currently happening.
    #[default]
    Default = 0,
    /// An edit is in progress.
    EditInProgress = 1,
    /// An edit was successful.
    EditSuccessful = 2,
    // A removal is in progress.
    RemovalInProgress = 3,
}

mod imp {
    use std::cell::{Cell, RefCell};

    use glib::subclass::{InitializingObject, Signal};
    use once_cell::sync::Lazy;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/editable_avatar.ui")]
    #[properties(wrapper_type = super::EditableAvatar)]
    pub struct EditableAvatar {
        /// The [`AvatarData`] to display.
        #[property(get, set = Self::set_data, explicit_notify)]
        pub data: RefCell<Option<AvatarData>>,
        /// Whether this avatar is changeable.
        #[property(get, set = Self::set_editable, explicit_notify)]
        pub editable: Cell<bool>,
        /// The current state of the edit.
        #[property(get, set = Self::set_state, explicit_notify, builder(EditableAvatarState::default()))]
        pub state: Cell<EditableAvatarState>,
        /// The state of the avatar edit.
        pub edit_state: Cell<ActionState>,
        /// Whether the edit button is sensitive.
        pub edit_sensitive: Cell<bool>,
        /// Whether this avatar is removable.
        pub removable: Cell<bool>,
        /// The state of the avatar removal.
        pub remove_state: Cell<ActionState>,
        /// Whether the remove button is sensitive.
        pub remove_sensitive: Cell<bool>,
        /// A temporary image to show instead of the avatar.
        #[property(get)]
        pub temp_image: RefCell<Option<gdk::Paintable>>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub button_remove: TemplateChild<ActionButton>,
        #[template_child]
        pub button_edit: TemplateChild<ActionButton>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for EditableAvatar {
        const NAME: &'static str = "ComponentsEditableAvatar";
        type Type = super::EditableAvatar;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            klass.set_css_name("editable-avatar");

            klass.install_action("editable-avatar.edit-avatar", None, |obj, _, _| {
                spawn!(clone!(@weak obj => async move {
                    obj.choose_avatar().await;
                }));
            });
            klass.install_action("editable-avatar.remove-avatar", None, |obj, _, _| {
                obj.emit_by_name::<()>("remove-avatar", &[]);
            });
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for EditableAvatar {
        fn signals() -> &'static [Signal] {
            static SIGNALS: Lazy<Vec<Signal>> = Lazy::new(|| {
                vec![
                    Signal::builder("edit-avatar")
                        .param_types([gio::File::static_type()])
                        .build(),
                    Signal::builder("remove-avatar").build(),
                ]
            });
            SIGNALS.as_ref()
        }

        fn constructed(&self) {
            self.parent_constructed();

            self.button_remove.set_extra_classes(&["error"]);

            let obj = self.obj();
            let image_present_expr = obj
                .property_expression("data")
                .chain_property::<AvatarData>("image")
                .chain_property::<AvatarImage>("paintable")
                .chain_closure::<bool>(closure!(
                    |_: Option<glib::Object>, image: Option<gdk::Paintable>| { image.is_some() }
                ));
            let editable_expr = obj.property_expression("editable");
            let button_remove_visible = expression::and(editable_expr, image_present_expr);
            button_remove_visible.bind(&*self.button_remove, "visible", glib::Object::NONE);
        }
    }

    impl WidgetImpl for EditableAvatar {}
    impl BinImpl for EditableAvatar {}

    impl EditableAvatar {
        /// Set the [`AvatarData`] to display.
        fn set_data(&self, data: Option<AvatarData>) {
            if *self.data.borrow() == data {
                return;
            }

            self.data.replace(data);
            self.obj().notify_data();
        }

        /// Set whether this avatar is editable.
        fn set_editable(&self, editable: bool) {
            if self.editable.get() == editable {
                return;
            }

            self.editable.set(editable);
            self.obj().notify_editable();
        }

        /// Set the state of the edit.
        fn set_state(&self, state: EditableAvatarState) {
            if self.state.get() == state {
                return;
            }
            let obj = self.obj();

            match state {
                EditableAvatarState::Default => {
                    obj.show_temp_image(false);
                    obj.set_edit_state(ActionState::Default);
                    obj.set_edit_sensitive(true);
                    obj.set_remove_state(ActionState::Default);
                    obj.set_remove_sensitive(true);

                    obj.set_temp_image_from_file(None);
                }
                EditableAvatarState::EditInProgress => {
                    obj.show_temp_image(true);
                    obj.set_edit_state(ActionState::Loading);
                    obj.set_edit_sensitive(true);
                    obj.set_remove_state(ActionState::Default);
                    obj.set_remove_sensitive(false);
                }
                EditableAvatarState::EditSuccessful => {
                    obj.show_temp_image(false);
                    obj.set_edit_sensitive(true);
                    obj.set_remove_state(ActionState::Default);
                    obj.set_remove_sensitive(true);

                    obj.set_temp_image_from_file(None);

                    // Animation for success.
                    obj.set_edit_state(ActionState::Success);
                    glib::timeout_add_local_once(
                        Duration::from_secs(2),
                        clone!(@weak obj => move || {
                            obj.set_state(EditableAvatarState::Default);
                        }),
                    );
                }
                EditableAvatarState::RemovalInProgress => {
                    obj.show_temp_image(true);
                    obj.set_edit_state(ActionState::Default);
                    obj.set_edit_sensitive(false);
                    obj.set_remove_state(ActionState::Loading);
                    obj.set_remove_sensitive(true);
                }
            }

            self.state.set(state);
            obj.notify_state();
        }
    }
}

glib::wrapper! {
    /// An `Avatar` that can be edited.
    pub struct EditableAvatar(ObjectSubclass<imp::EditableAvatar>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl EditableAvatar {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Reset the state of the avatar.
    pub fn reset(&self) {
        self.set_state(EditableAvatarState::Default);
    }

    /// Show that an edit is in progress.
    pub fn edit_in_progress(&self) {
        self.set_state(EditableAvatarState::EditInProgress);
    }

    /// Show that a removal is in progress.
    pub fn removal_in_progress(&self) {
        self.set_state(EditableAvatarState::RemovalInProgress);
    }

    /// Show that the current ongoing action was successful.
    ///
    /// This is has no effect if no action is ongoing.
    pub fn success(&self) {
        if self.edit_state() == ActionState::Loading {
            self.set_state(EditableAvatarState::EditSuccessful);
        } else if self.remove_state() == ActionState::Loading {
            // The remove button is hidden as soon as the avatar is gone so we
            // don't need a state when it succeeds.
            self.set_state(EditableAvatarState::Default);
        }
    }

    /// The state of the avatar edit.
    fn edit_state(&self) -> ActionState {
        self.imp().edit_state.get()
    }

    /// Set the state of the avatar edit.
    fn set_edit_state(&self, state: ActionState) {
        if self.edit_state() == state {
            return;
        }

        self.imp().edit_state.set(state);
    }

    /// Whether the edit button is sensitive.
    fn edit_sensitive(&self) -> bool {
        self.imp().edit_sensitive.get()
    }

    /// Set whether the edit button is sensitive.
    fn set_edit_sensitive(&self, sensitive: bool) {
        if self.edit_sensitive() == sensitive {
            return;
        }

        self.imp().edit_sensitive.set(sensitive);
    }

    /// The state of the avatar removal.
    fn remove_state(&self) -> ActionState {
        self.imp().remove_state.get()
    }

    /// Set the state of the avatar removal.
    fn set_remove_state(&self, state: ActionState) {
        if self.remove_state() == state {
            return;
        }

        self.imp().remove_state.set(state);
    }

    /// Whether the remove button is sensitive.
    fn remove_sensitive(&self) -> bool {
        self.imp().remove_sensitive.get()
    }

    /// Set whether the remove button is sensitive.
    fn set_remove_sensitive(&self, sensitive: bool) {
        if self.remove_sensitive() == sensitive {
            return;
        }

        self.imp().remove_sensitive.set(sensitive);
    }

    fn set_temp_image_from_file(&self, file: Option<&gio::File>) {
        self.imp().temp_image.replace(
            file.and_then(|file| ImagePaintable::from_file(file).ok())
                .map(|texture| texture.upcast()),
        );
        self.notify_temp_image();
    }

    /// Show an avatar with `temp_image` instead of `avatar`.
    fn show_temp_image(&self, show_temp: bool) {
        let stack = &self.imp().stack;
        if show_temp {
            stack.set_visible_child_name("temp");
        } else {
            stack.set_visible_child_name("default");
        }
    }

    async fn choose_avatar(&self) {
        let filters = gio::ListStore::new::<gtk::FileFilter>();

        let image_filter = gtk::FileFilter::new();
        image_filter.set_name(Some(&gettext("Images")));
        image_filter.add_mime_type("image/*");
        filters.append(&image_filter);

        let dialog = gtk::FileDialog::builder()
            .title(gettext("Choose Avatar"))
            .modal(true)
            .accept_label(gettext("Choose"))
            .filters(&filters)
            .build();

        let file = match dialog
            .open_future(self.root().and_downcast_ref::<gtk::Window>())
            .await
        {
            Ok(file) => file,
            Err(error) => {
                if error.matches(gtk::DialogError::Dismissed) {
                    debug!("File dialog dismissed by user");
                } else {
                    error!("Could not open avatar file: {error:?}");
                    toast!(self, gettext("Could not open avatar file"));
                }
                return;
            }
        };

        if let Some(content_type) = file
            .query_info_future(
                gio::FILE_ATTRIBUTE_STANDARD_CONTENT_TYPE,
                gio::FileQueryInfoFlags::NONE,
                glib::Priority::LOW,
            )
            .await
            .ok()
            .and_then(|info| info.content_type())
        {
            if gio::content_type_is_a(&content_type, "image/*") {
                self.set_temp_image_from_file(Some(&file));
                self.emit_by_name::<()>("edit-avatar", &[&file]);
            } else {
                error!("The chosen file is not an image");
                toast!(self, gettext("The chosen file is not an image"));
            }
        } else {
            error!("Could not get the content type of the file");
            toast!(
                self,
                gettext("Could not determine the type of the chosen file")
            );
        }
    }

    pub fn connect_edit_avatar<F: Fn(&Self, gio::File) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "edit-avatar",
            true,
            closure_local!(|obj: Self, file: gio::File| {
                f(&obj, file);
            }),
        )
    }

    pub fn connect_remove_avatar<F: Fn(&Self) + 'static>(&self, f: F) -> glib::SignalHandlerId {
        self.connect_closure(
            "remove-avatar",
            true,
            closure_local!(|obj: Self| {
                f(&obj);
            }),
        )
    }
}
