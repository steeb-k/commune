use adw::prelude::*;
use gtk::{gdk, gio, glib, glib::clone, subclass::prelude::*};

use super::{Avatar, AvatarData};
use crate::utils::BoundObject;

/// Compute the overlap according to the child's size.
fn overlap(for_size: i32) -> i32 {
    // Make the overlap a little less than half the size of the avatar.
    (for_size as f64 / 2.5) as i32
}

pub type ExtractAvatarDataFn = dyn Fn(&glib::Object) -> AvatarData + 'static;

mod imp {
    use std::cell::{Cell, RefCell};

    use super::*;

    #[derive(Default, glib::Properties)]
    #[properties(wrapper_type = super::OverlappingAvatars)]
    pub struct OverlappingAvatars {
        /// The child avatars.
        pub avatars: RefCell<Vec<adw::Bin>>,
        /// The size of the avatars.
        #[property(get, set = Self::set_avatar_size, explicit_notify)]
        pub avatar_size: Cell<i32>,
        /// The maximum number of avatars to display.
        ///
        /// `0` means that all avatars are displayed.
        #[property(get, set = Self::set_max_avatars, explicit_notify)]
        pub max_avatars: Cell<u32>,
        /// The list model that is bound, if any.
        pub bound_model: BoundObject<gio::ListModel>,
        /// The method used to extract `AvatarData` from the items of the list
        /// model, if any.
        pub extract_avatar_data_fn: RefCell<Option<Box<ExtractAvatarDataFn>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for OverlappingAvatars {
        const NAME: &'static str = "OverlappingAvatars";
        type Type = super::OverlappingAvatars;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_accessible_role(gtk::AccessibleRole::Img);
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for OverlappingAvatars {
        fn dispose(&self) {
            for avatar in self.avatars.take() {
                avatar.unparent();
            }
        }
    }

    impl WidgetImpl for OverlappingAvatars {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let mut size = 0;
            // child_size = avatar_size + cutout_borders
            let child_size = self.avatar_size.get() + 2;

            if orientation == gtk::Orientation::Vertical {
                if self.avatars.borrow().is_empty() {
                    return (0, 0, -1, 1);
                } else {
                    return (child_size, child_size, -1, -1);
                }
            }

            let overlap = overlap(child_size);

            for avatar in self.avatars.borrow().iter() {
                if !avatar.should_layout() {
                    continue;
                }

                size += child_size - overlap;
            }

            // The last child doesn't have an overlap.
            if size > 0 {
                size += overlap;
            }

            (size, size, -1, -1)
        }

        fn size_allocate(&self, _width: i32, _height: i32, _baseline: i32) {
            let mut pos = 0;
            // child_size = avatar_size + cutout_borders
            let child_size = self.avatar_size.get() + 2;
            let overlap = overlap(child_size);

            for avatar in self.avatars.borrow().iter() {
                if !avatar.should_layout() {
                    continue;
                }

                let x = pos;
                pos += child_size - overlap;

                let allocation = gdk::Rectangle::new(x, 0, child_size, child_size);

                avatar.size_allocate(&allocation, -1);
            }
        }
    }

    impl AccessibleImpl for OverlappingAvatars {
        fn first_accessible_child(&self) -> Option<gtk::Accessible> {
            // Hide the children in the a11y tree.
            None
        }
    }

    impl OverlappingAvatars {
        /// Set the size of the avatars.
        fn set_avatar_size(&self, size: i32) {
            if self.avatar_size.get() == size {
                return;
            }
            let obj = self.obj();

            self.avatar_size.set(size);
            obj.notify_avatar_size();

            // Update the sizes of the avatars.
            for avatar in self
                .avatars
                .borrow()
                .iter()
                .filter_map(|bin| bin.child().and_downcast::<Avatar>())
            {
                avatar.set_size(size);
            }
            obj.queue_resize();
        }

        /// Set the maximum number of avatars to display.
        fn set_max_avatars(&self, max_avatars: u32) {
            let old_max_avatars = self.max_avatars.get();

            if old_max_avatars == max_avatars {
                return;
            }
            let obj = self.obj();

            self.max_avatars.set(max_avatars);
            if max_avatars != 0 && self.avatars.borrow().len() > max_avatars as usize {
                // We have more children than we should, remove them.
                let children = self.avatars.borrow_mut().split_off(max_avatars as usize);
                for widget in children {
                    widget.unparent()
                }
            } else if max_avatars == 0 || (old_max_avatars != 0 && max_avatars > old_max_avatars) {
                let Some(model) = self.bound_model.obj() else {
                    return;
                };

                let diff = model.n_items() - old_max_avatars;
                if diff > 0 {
                    // We could have more children, create them.
                    obj.handle_items_changed(&model, old_max_avatars, 0, diff);
                }
            }

            obj.notify_max_avatars();
        }
    }
}

glib::wrapper! {
    /// A horizontal list of overlapping avatars.
    pub struct OverlappingAvatars(ObjectSubclass<imp::OverlappingAvatars>)
        @extends gtk::Widget, @implements gtk::Accessible;
}

impl OverlappingAvatars {
    /// Create an empty `OverlappingAvatars`.
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Bind a `ListModel` to this list.
    pub fn bind_model<P: Fn(&glib::Object) -> AvatarData + 'static>(
        &self,
        model: Option<impl glib::IsA<gio::ListModel>>,
        extract_avatar_data_fn: P,
    ) {
        let imp = self.imp();

        imp.bound_model.disconnect_signals();
        for avatar in imp.avatars.take() {
            avatar.unparent();
        }
        imp.extract_avatar_data_fn.take();

        let Some(model) = model else {
            return;
        };

        let signal_handler_id = model.connect_items_changed(
            clone!(@weak self as obj => move |model, position, removed, added| {
                obj.handle_items_changed(model, position, removed, added)
            }),
        );

        imp.bound_model
            .set(model.clone().upcast(), vec![signal_handler_id]);

        imp.extract_avatar_data_fn
            .replace(Some(Box::new(extract_avatar_data_fn)));

        self.handle_items_changed(&model, 0, 0, model.n_items())
    }

    fn handle_items_changed(
        &self,
        model: &impl glib::IsA<gio::ListModel>,
        position: u32,
        mut removed: u32,
        added: u32,
    ) {
        let max_avatars = self.max_avatars();
        if max_avatars != 0 && position >= max_avatars {
            // No changes here.
            return;
        }

        let imp = self.imp();
        let mut avatars = imp.avatars.borrow_mut();
        let avatar_size = self.avatar_size();
        let extract_avatar_data_fn_borrow = imp.extract_avatar_data_fn.borrow();
        let extract_avatar_data_fn = extract_avatar_data_fn_borrow.as_ref().unwrap();

        while removed > 0 {
            if position as usize >= avatars.len() {
                break;
            }

            let avatar = avatars.remove(position as usize);
            avatar.unparent();
            removed -= 1;
        }

        for i in position..(position + added) {
            if max_avatars != 0 && i >= max_avatars {
                break;
            }

            let item = model.item(i).unwrap();
            let avatar_data = extract_avatar_data_fn(&item);

            let avatar = Avatar::new();
            avatar.set_data(Some(avatar_data));
            avatar.set_size(avatar_size);

            let cutout = adw::Bin::builder()
                .child(&avatar)
                .css_classes(["cutout"])
                .build();
            cutout.set_parent(self);

            avatars.insert(i as usize, cutout);
        }

        self.queue_resize();
    }
}
