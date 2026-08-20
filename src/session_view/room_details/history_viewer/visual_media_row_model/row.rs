use gtk::{glib, subclass::prelude::*};

use super::MediaAge;

mod imp {
    use std::cell::{Cell, RefCell};

    use super::*;

    #[derive(Debug, Default)]
    pub struct VisualMediaRow {
        /// The items presented on this row.
        pub(super) items: RefCell<Vec<glib::Object>>,
        /// The number of items that fit on a full row.
        pub(super) n_columns: Cell<u32>,
        /// The age of the section that this row belongs to.
        pub(super) age: Cell<Option<MediaAge>>,
        /// The position of the first item of this row in the source model.
        pub(super) source_position: Cell<u32>,
        /// Whether this row presents the loading item.
        pub(super) is_loading: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for VisualMediaRow {
        const NAME: &'static str = "VisualMediaRow";
        type Type = super::VisualMediaRow;
    }

    impl ObjectImpl for VisualMediaRow {}
}

glib::wrapper! {
    /// A row of the visual media history viewer.
    ///
    /// A row never spans two sections, so the last row of a section can have
    /// fewer items than the others.
    pub struct VisualMediaRow(ObjectSubclass<imp::VisualMediaRow>);
}

impl VisualMediaRow {
    /// Construct a row presenting the given media items.
    pub(super) fn new(
        items: Vec<glib::Object>,
        n_columns: u32,
        age: Option<MediaAge>,
        source_position: u32,
    ) -> Self {
        let obj = glib::Object::new::<Self>();

        let imp = obj.imp();
        imp.items.replace(items);
        imp.n_columns.set(n_columns);
        imp.age.set(age);
        imp.source_position.set(source_position);

        obj
    }

    /// Construct a row presenting the given loading item.
    ///
    /// The age is the one of the section that the row is appended to, so that
    /// the loading item does not get a heading of its own.
    pub(super) fn new_loading(
        item: glib::Object,
        age: Option<MediaAge>,
        source_position: u32,
    ) -> Self {
        let obj = Self::new(vec![item], 1, age, source_position);
        obj.imp().is_loading.set(true);
        obj
    }

    /// The items presented on this row.
    pub(crate) fn items(&self) -> Vec<glib::Object> {
        self.imp().items.borrow().clone()
    }

    /// The number of items that fit on a full row.
    ///
    /// This is always at least 1, and the number of items of the row is never
    /// larger than it.
    pub(crate) fn n_columns(&self) -> u32 {
        self.imp().n_columns.get().max(1)
    }

    /// The age of the section that this row belongs to.
    ///
    /// This is `None` only for the loading item when there is no media before
    /// it.
    pub(crate) fn age(&self) -> Option<MediaAge> {
        self.imp().age.get()
    }

    /// Whether this row presents the loading item.
    pub(crate) fn is_loading(&self) -> bool {
        self.imp().is_loading.get()
    }

    /// The position of the first item of this row in the source model.
    pub(super) fn source_position(&self) -> u32 {
        self.imp().source_position.get()
    }

    /// The position after the last item of this row in the source model.
    pub(super) fn source_end(&self) -> u32 {
        self.source_position() + self.imp().items.borrow().len() as u32
    }
}
