use adw::subclass::prelude::*;
use gtk::{glib, prelude::*, CompositeTemplate};

mod crop_circle;
mod data;
mod editable;
mod image;
mod overlapping;

pub use self::{
    data::AvatarData,
    editable::EditableAvatar,
    image::{AvatarImage, AvatarUriSource},
    overlapping::OverlappingAvatars,
};

mod imp {
    use std::{cell::RefCell, marker::PhantomData};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/avatar/mod.ui")]
    #[properties(wrapper_type = super::Avatar)]
    pub struct Avatar {
        /// The [`AvatarData`] displayed by this widget.
        #[property(get, set = Self::set_data, explicit_notify, nullable)]
        pub data: RefCell<Option<AvatarData>>,
        /// The size of the Avatar.
        #[property(get = Self::size, set = Self::set_size, explicit_notify, builder().default_value(-1).minimum(-1))]
        pub size: PhantomData<i32>,
        #[template_child]
        pub avatar: TemplateChild<adw::Avatar>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Avatar {
        const NAME: &'static str = "Avatar";
        type Type = super::Avatar;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            AvatarImage::ensure_type();

            Self::bind_template(klass);

            klass.set_accessible_role(gtk::AccessibleRole::Img);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for Avatar {
        fn constructed(&self) {
            self.parent_constructed();

            self.obj().connect_map(|avatar| {
                avatar.request_custom_avatar();
            });
        }
    }

    impl WidgetImpl for Avatar {}
    impl BinImpl for Avatar {}

    impl AccessibleImpl for Avatar {
        fn first_accessible_child(&self) -> Option<gtk::Accessible> {
            // Hide the children in the a11y tree.
            None
        }
    }

    impl Avatar {
        /// The size of the Avatar.
        fn size(&self) -> i32 {
            self.avatar.size()
        }

        /// Set the size of the Avatar.
        fn set_size(&self, size: i32) {
            if self.size() == size {
                return;
            }

            self.avatar.set_size(size);

            let obj = self.obj();
            if obj.is_mapped() {
                obj.request_custom_avatar();
            }

            obj.notify_size();
        }

        /// Set the [`AvatarData`] displayed by this widget.
        fn set_data(&self, data: Option<AvatarData>) {
            if *self.data.borrow() == data {
                return;
            }

            self.data.replace(data);

            let obj = self.obj();
            if obj.is_mapped() {
                obj.request_custom_avatar();
            }

            obj.notify_data();
        }
    }
}

glib::wrapper! {
    /// A widget displaying an `Avatar` for a `Room` or `User`.
    pub struct Avatar(ObjectSubclass<imp::Avatar>)
        @extends gtk::Widget, adw::Bin, @implements gtk::Accessible;
}

impl Avatar {
    pub fn new() -> Self {
        glib::Object::new()
    }

    fn request_custom_avatar(&self) {
        if let Some(image) = self.imp().data.borrow().as_ref().and_then(|d| d.image()) {
            let size = self.size() * self.scale_factor();
            image.set_needed_size(size as u32);
        }
    }
}
