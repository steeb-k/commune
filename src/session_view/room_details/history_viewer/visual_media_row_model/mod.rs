use std::fmt;

use gtk::{gio, glib, glib::clone, prelude::*, subclass::prelude::*};

mod age;
mod row;
// These tests need GTK initialized, because the model implements
// `GtkSectionModel` and registering that interface asserts on it.
// `#[gtk::test]` initializes GTK on a `GThreadPool` thread, and on macOS GTK
// insists on being initialized on the process main thread, which no test
// harness we use runs the body of a test on. The model itself has nothing
// platform-specific in it, so the Linux runs cover it.
#[cfg(all(test, not(target_os = "macos")))]
mod tests;

pub(crate) use self::{age::MediaAge, row::VisualMediaRow};
use crate::utils::BoundObject;

/// A function returning the time at which the given item was sent, or `None`
/// if the item is not a media.
pub(crate) type TimestampFn = dyn Fn(&glib::Object) -> Option<glib::DateTime>;

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};

    use super::*;

    #[derive(glib::Properties)]
    #[properties(wrapper_type = super::VisualMediaRowModel)]
    pub struct VisualMediaRowModel {
        /// The underlying model.
        #[property(get, set = Self::set_model, explicit_notify, nullable)]
        model: BoundObject<gio::ListModel>,
        /// The number of items presented on a full row.
        #[property(get, set = Self::set_n_columns, explicit_notify, minimum = 1, default = 1)]
        n_columns: Cell<u32>,
        /// The function returning the time at which an item was sent.
        pub(super) timestamp_fn: OnceCell<Box<TimestampFn>>,
        /// The rows presented by this model.
        rows: RefCell<Vec<VisualMediaRow>>,
    }

    impl Default for VisualMediaRowModel {
        fn default() -> Self {
            Self {
                model: BoundObject::default(),
                n_columns: Cell::new(1),
                timestamp_fn: OnceCell::default(),
                rows: RefCell::default(),
            }
        }
    }

    impl fmt::Debug for VisualMediaRowModel {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("VisualMediaRowModel")
                .field("model", &self.model)
                .field("n_columns", &self.n_columns)
                .field("rows", &self.rows)
                .finish_non_exhaustive()
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for VisualMediaRowModel {
        const NAME: &'static str = "VisualMediaRowModel";
        type Type = super::VisualMediaRowModel;
        type Interfaces = (gio::ListModel, gtk::SectionModel);
    }

    #[glib::derived_properties]
    impl ObjectImpl for VisualMediaRowModel {}

    impl ListModelImpl for VisualMediaRowModel {
        fn item_type(&self) -> glib::Type {
            VisualMediaRow::static_type()
        }

        fn n_items(&self) -> u32 {
            self.rows.borrow().len() as u32
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            self.rows
                .borrow()
                .get(position as usize)
                .map(|row| row.clone().upcast())
        }
    }

    impl SectionModelImpl for VisualMediaRowModel {
        /// The rows sharing the age of the row at the given position, as a
        /// `start..end` range.
        fn section(&self, position: u32) -> (u32, u32) {
            let rows = self.rows.borrow();
            let n_rows = rows.len() as u32;

            let Some(age) = rows.get(position as usize).map(VisualMediaRow::age) else {
                // The position is past the end of the model, which GTK asks about to know
                // where the last section stops.
                return (n_rows, u32::MAX);
            };

            let position = position as usize;
            let start = rows[..position]
                .iter()
                .rposition(|row| row.age() != age)
                .map_or(0, |index| index as u32 + 1);
            let end = rows[position..]
                .iter()
                .position(|row| row.age() != age)
                .map_or(n_rows, |offset| position as u32 + offset as u32);

            (start, end)
        }
    }

    impl VisualMediaRowModel {
        /// The function returning the time at which an item was sent.
        fn timestamp_fn(&self) -> &TimestampFn {
            self.timestamp_fn
                .get()
                .expect("timestamp Fn should be initialized")
        }

        /// Set the underlying model.
        fn set_model(&self, model: Option<gio::ListModel>) {
            if self.model.obj() == model {
                return;
            }

            self.model.disconnect_signals();

            if let Some(model) = model {
                let items_changed_handler = model.connect_items_changed(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_, position, removed, added| {
                        imp.source_items_changed(position, removed, added);
                    }
                ));

                self.model.set(model, vec![items_changed_handler]);
            }

            self.rebuild_from(0);
            self.obj().notify_model();
        }

        /// Set the number of items presented on a full row.
        fn set_n_columns(&self, n_columns: u32) {
            let n_columns = n_columns.max(1);

            if self.n_columns.get() == n_columns {
                return;
            }

            self.n_columns.set(n_columns);

            self.rebuild_from(0);
            self.obj().notify_n_columns();
        }

        /// Rebuild all the rows, to pick up the current time.
        pub(super) fn refresh(&self) {
            self.rebuild_from(0);
        }

        /// Handle when the items changed in the underlying model.
        fn source_items_changed(&self, position: u32, removed: u32, added: u32) {
            if removed == 0 && added == 0 {
                return;
            }

            // The rows cover the underlying model without a gap, so the first row that
            // ends after the change is the one that contains it. When the change is at
            // the very end there is none, and the last row is rebuilt anyway: it can be
            // partial, in which case the added items belong to it.
            let index = {
                let rows = self.rows.borrow();
                rows.iter()
                    .position(|row| row.source_end() > position)
                    .unwrap_or_else(|| rows.len().saturating_sub(1))
            };

            self.rebuild_from(index);
        }

        /// Rebuild the rows from the given index, and notify about the change.
        fn rebuild_from(&self, index: usize) {
            let new_rows = self.build_rows_from(index);

            let removed = self.rows.borrow().len().saturating_sub(index);
            let added = new_rows.len();

            if removed == 0 && added == 0 {
                return;
            }

            let retired = {
                let mut rows = self.rows.borrow_mut();
                let retired = rows.split_off(index);
                rows.extend(new_rows);
                retired
            };

            self.obj()
                .items_changed(index as u32, removed as u32, added as u32);

            // `retired`'s `VisualMediaRow`s are only dropped now, after the borrow above
            // was released and the change was signalled.
            drop(retired);
        }

        /// Build the rows from the given index to the end of the underlying
        /// model.
        fn build_rows_from(&self, index: usize) -> Vec<VisualMediaRow> {
            let Some(model) = self.model.obj() else {
                return Vec::new();
            };

            // The rows before the index are kept, so the new ones start where they stop,
            // and the first age to compare with is the one of the last kept row.
            let (mut position, mut age) = {
                let rows = self.rows.borrow();
                let position = rows.get(index).map_or_else(
                    || rows.last().map_or(0, VisualMediaRow::source_end),
                    VisualMediaRow::source_position,
                );
                let age = index
                    .checked_sub(1)
                    .and_then(|index| rows.get(index))
                    .and_then(VisualMediaRow::age);

                (position, age)
            };

            let timestamp_fn = self.timestamp_fn();
            let now = glib::DateTime::now_local().expect("current time should be available");
            let n_columns = self.n_columns.get();
            let n_source_items = model.n_items();

            let mut rows = Vec::new();
            let mut items = Vec::with_capacity(n_columns as usize);
            let mut items_position = position;

            while position < n_source_items {
                let Some(item) = model.item(position) else {
                    break;
                };

                let Some(timestamp) = timestamp_fn(&item) else {
                    // This is the loading item. It is presented on a row of its own, at the
                    // end of the last section.
                    if !items.is_empty() {
                        rows.push(VisualMediaRow::new(
                            std::mem::take(&mut items),
                            n_columns,
                            age,
                            items_position,
                        ));
                    }

                    rows.push(VisualMediaRow::new_loading(item, age, position));

                    position += 1;
                    items_position = position;
                    continue;
                };

                let item_age = MediaAge::new(&timestamp, &now);
                // The events are paginated backwards but their timestamps are not
                // guaranteed to decrease, while the sections must stay in order, so an
                // item is never presented as more recent than the one before it.
                let item_age = age.map_or(item_age, |age| item_age.max(age));

                if age != Some(item_age) && !items.is_empty() {
                    // The section ends here, so does the row, even if it is not full.
                    rows.push(VisualMediaRow::new(
                        std::mem::take(&mut items),
                        n_columns,
                        age,
                        items_position,
                    ));
                    items_position = position;
                }

                age = Some(item_age);
                items.push(item);

                if items.len() as u32 == n_columns {
                    rows.push(VisualMediaRow::new(
                        std::mem::take(&mut items),
                        n_columns,
                        age,
                        items_position,
                    ));
                    items_position = position + 1;
                }

                position += 1;
            }

            if !items.is_empty() {
                rows.push(VisualMediaRow::new(items, n_columns, age, items_position));
            }

            rows
        }
    }
}

glib::wrapper! {
    /// A list model presenting the items of another model as rows of a fixed
    /// number of columns, split into sections by the age of the media.
    ///
    /// The last row of a section can have fewer items than the number of
    /// columns, because a row never spans two sections.
    pub struct VisualMediaRowModel(ObjectSubclass<imp::VisualMediaRowModel>)
        @implements gio::ListModel, gtk::SectionModel;
}

impl VisualMediaRowModel {
    /// Construct a new `VisualMediaRowModel` with the given function to get the
    /// time at which an item was sent.
    pub(crate) fn new<F>(timestamp_fn: F) -> Self
    where
        F: Fn(&glib::Object) -> Option<glib::DateTime> + 'static,
    {
        let obj = glib::Object::new::<Self>();
        // Ignore the error because we cannot `.expect()` when the value is a function.
        let _ = obj.imp().timestamp_fn.set(Box::new(timestamp_fn));
        obj
    }

    /// Rebuild all the rows, to pick up the current time.
    ///
    /// The headings are relative to the current date, so they go stale when the
    /// viewer is left open across midnight.
    pub(crate) fn refresh(&self) {
        self.imp().refresh();
    }
}
