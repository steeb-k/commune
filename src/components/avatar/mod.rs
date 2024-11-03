use adw::subclass::prelude::*;
use gtk::{glib, glib::clone, prelude::*, CompositeTemplate};

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
use crate::{components::AnimatedImagePaintable, utils::CountedRef};

mod imp {
    use std::{cell::RefCell, marker::PhantomData};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/components/avatar/mod.ui")]
    #[properties(wrapper_type = super::Avatar)]
    pub struct Avatar {
        #[template_child]
        pub avatar: TemplateChild<adw::Avatar>,
        /// The [`AvatarData`] displayed by this widget.
        #[property(get, set = Self::set_data, explicit_notify, nullable)]
        pub data: RefCell<Option<AvatarData>>,
        /// The size of the Avatar.
        #[property(get = Self::size, set = Self::set_size, explicit_notify, builder().default_value(-1).minimum(-1))]
        pub size: PhantomData<i32>,
        paintable_animation_ref: RefCell<Option<CountedRef>>,
        image_watches: RefCell<Vec<gtk::ExpressionWatch>>,
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
        fn dispose(&self) {
            for watch in self.image_watches.take() {
                watch.unwatch();
            }
        }
    }

    impl WidgetImpl for Avatar {
        fn map(&self) {
            self.parent_map();
            self.update_image_size();
            self.update_animated_paintable_state();
        }

        fn unmap(&self) {
            self.parent_unmap();
            self.update_animated_paintable_state();
        }
    }

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

            self.update_image_size();
            self.obj().notify_size();
        }

        /// Set the [`AvatarData`] displayed by this widget.
        fn set_data(&self, data: Option<AvatarData>) {
            if *self.data.borrow() == data {
                return;
            }

            for watch in self.image_watches.take() {
                watch.unwatch();
            }

            if let Some(data) = &data {
                let image_watch = data.property_expression("image").watch(
                    None::<&glib::Object>,
                    clone!(
                        #[weak(rename_to = imp)]
                        self,
                        move || {
                            imp.update_image_size();
                        }
                    ),
                );
                let paintable_watch = data
                    .property_expression("image")
                    .chain_property::<AvatarImage>("paintable")
                    .watch(
                        None::<&glib::Object>,
                        clone!(
                            #[weak(rename_to = imp)]
                            self,
                            move || {
                                imp.update_animated_paintable_state();
                            }
                        ),
                    );
                self.image_watches
                    .replace(vec![image_watch, paintable_watch]);
            }

            self.data.replace(data);

            self.update_image_size();
            self.update_animated_paintable_state();
            self.obj().notify_data();
        }

        /// Update the size of the image for this avatar.
        fn update_image_size(&self) {
            let Some(image) = self.data.borrow().as_ref().and_then(AvatarData::image) else {
                return;
            };
            let obj = self.obj();

            if obj.is_mapped() {
                let needed_size = self.size() * obj.scale_factor();
                image.set_needed_size(u32::try_from(needed_size).unwrap_or_default());
            }
        }

        /// Update the state of the animated paintable for this avatar.
        fn update_animated_paintable_state(&self) {
            self.paintable_animation_ref.take();

            let Some(paintable) = self
                .data
                .borrow()
                .as_ref()
                .and_then(AvatarData::image)
                .and_then(|i| i.paintable())
                .and_downcast::<AnimatedImagePaintable>()
            else {
                return;
            };

            if self.obj().is_mapped() {
                self.paintable_animation_ref
                    .replace(Some(paintable.animation_ref()));
            }
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
}
