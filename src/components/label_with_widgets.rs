use gtk::{glib, pango, prelude::*, subclass::prelude::*};

pub const DEFAULT_PLACEHOLDER: &str = "<widget>";
const OBJECT_REPLACEMENT_CHARACTER: &str = "\u{FFFC}";

mod imp {
    use std::{
        cell::{Cell, RefCell},
        marker::PhantomData,
    };

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::LabelWithWidgets)]
    pub struct LabelWithWidgets {
        pub widgets: RefCell<Vec<gtk::Widget>>,
        pub widgets_sizes: RefCell<Vec<(i32, i32)>>,
        pub child: gtk::Label,
        /// The placeholder that is replaced by the widgets.
        #[property(get, set = Self::set_placeholder, explicit_notify, nullable)]
        pub placeholder: RefCell<Option<String>>,
        /// The text of the label.
        #[property(get, set = Self::set_label, explicit_notify, nullable)]
        pub label: RefCell<Option<String>>,
        /// Whether the label includes Pango markup.
        #[property(get = Self::uses_markup, set = Self::set_use_markup)]
        pub use_markup: PhantomData<bool>,
        /// Whether the label should be ellipsized.
        #[property(get, set = Self::set_ellipsize, explicit_notify)]
        pub ellipsize: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LabelWithWidgets {
        const NAME: &'static str = "LabelWithWidgets";
        type Type = super::LabelWithWidgets;
        type ParentType = gtk::Widget;
        type Interfaces = (gtk::Buildable,);
    }

    #[glib::derived_properties]
    impl ObjectImpl for LabelWithWidgets {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            let child = &self.child;
            child.set_parent(&*obj);
            child.set_wrap(true);
            child.set_wrap_mode(pango::WrapMode::WordChar);
            child.set_xalign(0.0);
            child.set_valign(gtk::Align::Start);
        }

        fn dispose(&self) {
            self.child.unparent();

            for widget in self.widgets.borrow().iter() {
                widget.unparent();
            }
        }
    }

    impl WidgetImpl for LabelWithWidgets {
        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            self.obj().allocate_shapes();
            self.child.measure(orientation, for_size)
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.child.allocate(width, height, baseline, None);
            self.obj().allocate_children();
        }

        fn request_mode(&self) -> gtk::SizeRequestMode {
            self.child.request_mode()
        }
    }

    impl BuildableImpl for LabelWithWidgets {
        fn add_child(&self, builder: &gtk::Builder, child: &glib::Object, type_: Option<&str>) {
            if let Some(child) = child.downcast_ref::<gtk::Widget>() {
                self.obj().append_child(child);
            } else {
                self.parent_add_child(builder, child, type_)
            }
        }
    }

    impl LabelWithWidgets {
        /// Set the text of the label.
        fn set_label(&self, label: Option<String>) {
            if *self.label.borrow() == label {
                return;
            }

            self.label.replace(label);

            let obj = self.obj();
            obj.update_label();
            obj.notify_label();
        }

        /// Set the placeholder that is replaced with widgets.
        fn set_placeholder(&self, placeholder: Option<String>) {
            if *self.placeholder.borrow() == placeholder {
                return;
            }

            self.placeholder.replace(placeholder);

            let obj = self.obj();
            obj.update_label();
            obj.notify_placeholder();
        }

        /// Whether the label includes Pango markup.
        fn uses_markup(&self) -> bool {
            self.child.uses_markup()
        }

        /// Set whether the label includes Pango markup.
        fn set_use_markup(&self, use_markup: bool) {
            self.child.set_use_markup(use_markup);
        }

        /// Sets whether the text of the label should be ellipsized.
        fn set_ellipsize(&self, ellipsize: bool) {
            if self.ellipsize.get() == ellipsize {
                return;
            }

            self.ellipsize.set(true);

            let obj = self.obj();
            obj.update_label();
            obj.notify_ellipsize();
        }
    }
}

glib::wrapper! {
    /// A Label that can have multiple widgets placed inside the text.
    ///
    /// By default the string "<widget>" will be used as location to place the
    /// child widgets. You can set your own placeholder if you need.
    pub struct LabelWithWidgets(ObjectSubclass<imp::LabelWithWidgets>)
        @extends gtk::Widget, @implements gtk::Accessible, gtk::Buildable;
}

impl LabelWithWidgets {
    /// Create an empty `LabelWithWidget`.
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Create a `LabelWithWidget` with the given label and widgets.
    pub fn with_label_and_widgets<P: IsA<gtk::Widget>>(label: &str, widgets: Vec<P>) -> Self {
        let obj: Self = glib::Object::builder().property("label", label).build();
        // FIXME: use a property for widgets
        obj.set_widgets(widgets);
        obj
    }

    pub fn append_child<P: IsA<gtk::Widget>>(&self, child: &P) {
        self.imp().widgets.borrow_mut().push(child.clone().upcast());
        child.set_parent(self);
        self.invalidate_child_widgets();
    }

    pub fn set_widgets<P: IsA<gtk::Widget>>(&self, widgets: Vec<P>) {
        let imp = self.imp();

        for widget in imp.widgets.take() {
            widget.unparent();
        }

        imp.widgets
            .borrow_mut()
            .append(&mut widgets.into_iter().map(|w| w.upcast()).collect());

        for child in imp.widgets.borrow().iter() {
            child.set_parent(self);
        }
        self.invalidate_child_widgets();
    }

    pub fn widgets(&self) -> Vec<gtk::Widget> {
        self.imp().widgets.borrow().clone()
    }

    fn invalidate_child_widgets(&self) {
        self.imp().widgets_sizes.borrow_mut().clear();
        self.allocate_shapes();
        self.queue_resize();
    }

    fn allocate_shapes(&self) {
        let imp = self.imp();

        if !imp.label.borrow().as_ref().is_some_and(|s| !s.is_empty()) {
            // No need to compute shapes if the label is empty.
            return;
        }

        if imp.widgets.borrow().is_empty() {
            // There should be no attributes if there are no widgets.
            imp.child.set_attributes(None);
            return;
        }

        let mut widgets_sizes = imp.widgets_sizes.borrow_mut();

        let mut child_size_changed = false;
        for (i, child) in imp.widgets.borrow().iter().enumerate() {
            let (_, natural_size) = child.preferred_size();
            let width = natural_size.width();
            let height = natural_size.height();
            if let Some((old_width, old_height)) = widgets_sizes.get(i) {
                if old_width != &width || old_height != &height {
                    let _ = std::mem::replace(&mut widgets_sizes[i], (width, height));
                    child_size_changed = true;
                }
            } else {
                widgets_sizes.insert(i, (width, height));
                child_size_changed = true;
            }
        }

        if !child_size_changed {
            return;
        }

        let attrs = pango::AttrList::new();
        for (i, (start_index, _)) in imp
            .child
            .text()
            .as_str()
            .match_indices(OBJECT_REPLACEMENT_CHARACTER)
            .enumerate()
        {
            if let Some((width, height)) = widgets_sizes.get(i) {
                let logical_rect = pango::Rectangle::new(
                    0,
                    -(height - (height / 4)) * pango::SCALE,
                    width * pango::SCALE,
                    height * pango::SCALE,
                );

                let mut shape = pango::AttrShape::new(&logical_rect, &logical_rect);
                shape.set_start_index(start_index as u32);
                shape.set_end_index((start_index + OBJECT_REPLACEMENT_CHARACTER.len()) as u32);
                attrs.insert(shape);
            } else {
                break;
            }
        }
        imp.child.set_attributes(Some(&attrs));
    }

    fn allocate_children(&self) {
        let imp = self.imp();
        let widgets = imp.widgets.borrow();
        let widgets_sizes = imp.widgets_sizes.borrow();

        let mut run_iter = imp.child.layout().iter();
        let mut i = 0;
        loop {
            if let Some(run) = run_iter.run_readonly() {
                if run
                    .item()
                    .analysis()
                    .extra_attrs()
                    .iter()
                    .any(|attr| attr.type_() == pango::AttrType::Shape)
                {
                    if let Some(widget) = widgets.get(i) {
                        let (width, height) = widgets_sizes[i];
                        let (_, mut extents) = run_iter.run_extents();
                        pango::extents_to_pixels(Some(&mut extents), None);

                        let (offset_x, offset_y) = imp.child.layout_offsets();
                        let allocation = gtk::Allocation::new(
                            extents.x() + offset_x,
                            extents.y() + offset_y,
                            width,
                            height,
                        );
                        widget.size_allocate(&allocation, -1);
                        i += 1;
                    } else {
                        break;
                    }
                }
            }
            if !run_iter.next_run() {
                break;
            }
        }
    }

    fn update_label(&self) {
        let imp = self.imp();
        let old_label = imp.child.text();
        let old_ellipsize = imp.child.ellipsize() == pango::EllipsizeMode::End;
        let new_ellipsize = self.ellipsize();

        let new_label = if let Some(label) = imp.label.borrow().as_ref() {
            let placeholder = imp.placeholder.borrow();
            let placeholder = placeholder.as_deref().unwrap_or(DEFAULT_PLACEHOLDER);
            let label = label.replace(placeholder, OBJECT_REPLACEMENT_CHARACTER);

            if new_ellipsize {
                if let Some(pos) = label.find('\n') {
                    format!("{}…", &label[0..pos])
                } else {
                    label
                }
            } else {
                label
            }
        } else {
            String::new()
        };

        if old_ellipsize != new_ellipsize || old_label != new_label {
            if new_ellipsize {
                // Workaround: if both wrap and ellipsize are set, and there are
                // widgets inserted, GtkLabel reports an erroneous minimum width.
                imp.child.set_wrap(false);
                imp.child.set_ellipsize(pango::EllipsizeMode::End);
            } else {
                imp.child.set_wrap(true);
                imp.child.set_ellipsize(pango::EllipsizeMode::None);
            }

            imp.child.set_label(&new_label);
            self.invalidate_child_widgets();
        }
    }
}
