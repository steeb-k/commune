use adw::prelude::*;
use gtk::{gdk, gio, glib, glib::clone, subclass::prelude::*};

use super::{crop_circle::CropCircle, Avatar, AvatarData};
use crate::utils::BoundObject;

pub type ExtractAvatarDataFn = dyn Fn(&glib::Object) -> AvatarData + 'static;

mod imp {
    use std::cell::{Cell, RefCell};

    use super::*;

    #[derive(Default, glib::Properties)]
    #[properties(wrapper_type = super::OverlappingAvatars)]
    pub struct OverlappingAvatars {
        /// The children containing the avatars.
        pub children: RefCell<Vec<CropCircle>>,
        /// The size of the avatars.
        #[property(get, set = Self::set_avatar_size, explicit_notify)]
        pub avatar_size: Cell<u32>,
        /// The spacing between the avatars.
        #[property(get, set = Self::set_spacing, explicit_notify)]
        pub spacing: Cell<u32>,
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
            for child in self.children.take() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for OverlappingAvatars {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            if self.children.borrow().is_empty() {
                return (0, 0, -1, 1);
            }

            let avatar_size = self.avatar_size.get();

            if orientation == gtk::Orientation::Vertical {
                let size = avatar_size.try_into().unwrap_or(i32::MAX);
                return (size, size, -1, -1);
            }

            let n_children = u32::try_from(self.children.borrow().len())
                .expect("count of children fits into u32");
            let overlap = self.overlap();
            let spacing = self.spacing.get();

            // The last avatar has no overlap.
            let mut size =
                n_children.saturating_sub(1) * avatar_size.saturating_sub(overlap + spacing);
            size += avatar_size;

            let size = size.try_into().unwrap_or(i32::MAX);
            (size, size, -1, -1)
        }

        fn size_allocate(&self, _width: i32, _height: i32, _baseline: i32) {
            let avatar_size = i32::try_from(self.avatar_size.get()).unwrap_or(i32::MAX);
            let overlap = i32::try_from(self.overlap()).expect("overlap fits into i32");
            let spacing = i32::try_from(self.spacing.get()).expect("spacing fits into i32");

            let distance_between_centers = (avatar_size - overlap).saturating_add(spacing);

            let mut x = 0;
            for child in self.children.borrow().iter() {
                let allocation = gdk::Rectangle::new(x, 0, avatar_size, avatar_size);
                child.size_allocate(&allocation, -1);

                x = x.saturating_add(distance_between_centers);
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
        fn set_avatar_size(&self, size: u32) {
            if self.avatar_size.get() == size {
                return;
            }
            let obj = self.obj();

            self.avatar_size.set(size);

            // Update the sizes of the avatars.
            let size = i32::try_from(size).unwrap_or(i32::MAX);
            let overlap = self.overlap();
            for child in self.children.borrow().iter() {
                child.set_cropped_width(overlap);

                if let Some(avatar) = child.child().and_downcast::<Avatar>() {
                    avatar.set_size(size);
                }
            }
            obj.queue_resize();

            obj.notify_avatar_size();
        }

        /// Compute the avatars overlap according to their size.
        #[allow(clippy::cast_sign_loss)] // The result can only be positive.
        pub(super) fn overlap(&self) -> u32 {
            let avatar_size = self.avatar_size.get();
            // Make the overlap a little less than half the size of the avatar.
            (f64::from(avatar_size) / 2.5) as u32
        }

        /// Set the spacing between the avatars.
        fn set_spacing(&self, spacing: u32) {
            if self.spacing.get() == spacing {
                return;
            }

            self.spacing.set(spacing);

            let obj = self.obj();
            obj.queue_resize();
            obj.notify_avatar_size();
        }

        /// Set the maximum number of avatars to display.
        fn set_max_avatars(&self, max_avatars: u32) {
            let old_max_avatars = self.max_avatars.get();

            if old_max_avatars == max_avatars {
                return;
            }
            let obj = self.obj();

            self.max_avatars.set(max_avatars);
            if max_avatars != 0 && self.children.borrow().len() > max_avatars as usize {
                // We have more children than we should, remove them.
                let children = self.children.borrow_mut().split_off(max_avatars as usize);

                for child in children {
                    child.unparent();
                }

                if let Some(child) = self.children.borrow().last() {
                    child.set_is_cropped(false);
                }

                obj.queue_resize();
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
        model: Option<impl IsA<gio::ListModel>>,
        extract_avatar_data_fn: P,
    ) {
        let imp = self.imp();

        imp.bound_model.disconnect_signals();
        for child in imp.children.take() {
            child.unparent();
        }
        imp.extract_avatar_data_fn.take();

        let Some(model) = model else {
            return;
        };

        let signal_handler_id = model.connect_items_changed(clone!(
            #[weak(rename_to = obj)]
            self,
            move |model, position, removed, added| {
                obj.handle_items_changed(model, position, removed, added);
            }
        ));

        imp.bound_model
            .set(model.clone().upcast(), vec![signal_handler_id]);

        imp.extract_avatar_data_fn
            .replace(Some(Box::new(extract_avatar_data_fn)));

        self.handle_items_changed(&model, 0, 0, model.n_items());
    }

    fn handle_items_changed(
        &self,
        model: &impl IsA<gio::ListModel>,
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
        let mut children = imp.children.borrow_mut();
        let extract_avatar_data_fn_borrow = imp.extract_avatar_data_fn.borrow();
        let extract_avatar_data_fn = extract_avatar_data_fn_borrow.as_ref().unwrap();

        let avatar_size = i32::try_from(self.avatar_size()).unwrap_or(i32::MAX);
        let cropped_width = imp.overlap();

        while removed > 0 {
            if position as usize >= children.len() {
                break;
            }

            let child = children.remove(position as usize);
            child.unparent();
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

            let child = CropCircle::new();
            child.set_child(Some(avatar));
            child.set_cropped_width(cropped_width);
            child.set_parent(self);

            children.insert(i as usize, child);
        }

        // Make sure that only the last avatar is not cropped.
        let last_pos = children.len().saturating_sub(1);
        for (i, child) in children.iter().enumerate() {
            child.set_is_cropped(i != last_pos);
        }

        self.queue_resize();
    }
}
