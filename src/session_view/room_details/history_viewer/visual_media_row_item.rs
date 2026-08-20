use gtk::{glib, prelude::*, subclass::prelude::*};

use super::{HistoryViewerEvent, VisualMediaItem, VisualMediaRow};

/// The space between two media of the history.
const SPACING: i32 = 2;
/// The size below which a media of the history should not be presented.
const ITEM_SIZE_REQUEST: i32 = 150;
/// The minimum number of media presented on a row.
const MIN_COLUMNS: u32 = 2;
/// The maximum number of media presented on a row.
pub(super) const MAX_COLUMNS: u32 = 5;
/// The width of a row presenting the minimum number of media.
const MIN_ROW_WIDTH: i32 = ITEM_SIZE_REQUEST * 2 + SPACING;

const _: () = assert!(MIN_COLUMNS == 2, "MIN_ROW_WIDTH is written for two columns");

/// The number of media that should be presented on a row of the given width.
pub(super) fn n_columns_for_width(width: i32) -> u32 {
    let n_columns = (width + SPACING) / (ITEM_SIZE_REQUEST + SPACING);
    u32::try_from(n_columns)
        .unwrap_or(MIN_COLUMNS)
        .clamp(MIN_COLUMNS, MAX_COLUMNS)
}

/// The size of a media on a row of the given width with the given number of
/// columns.
fn cell_size(width: i32, n_columns: i32) -> i32 {
    ((width - SPACING * (n_columns - 1)) / n_columns).max(0)
}

mod imp {
    use std::cell::RefCell;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::VisualMediaRowItem)]
    pub struct VisualMediaRowItem {
        /// The row that is presented.
        #[property(get, set = Self::set_row, explicit_notify, nullable)]
        row: RefCell<Option<VisualMediaRow>>,
        /// The widgets presenting the media of the row.
        items: RefCell<Vec<VisualMediaItem>>,
        /// The widget presenting the loading item, when this row is the loading
        /// row.
        loading_widget: RefCell<Option<gtk::Widget>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for VisualMediaRowItem {
        const NAME: &'static str = "ContentVisualMediaHistoryViewerRow";
        type Type = super::VisualMediaRowItem;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("visual-media-history-viewer-row");
            klass.set_accessible_role(gtk::AccessibleRole::Presentation);
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for VisualMediaRowItem {
        fn dispose(&self) {
            self.detach_loading_widget();

            for item in self.items.take() {
                item.unparent();
            }
        }
    }

    impl WidgetImpl for VisualMediaRowItem {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            if let Some(loading_widget) = self.loading_widget.borrow().as_ref() {
                return loading_widget.measure(orientation, for_size);
            }

            let n_columns = self.n_columns();

            if orientation == gtk::Orientation::Horizontal {
                // The minimum does not depend on the number of columns: the rows are
                // rebuilt with fewer of them when the view gets too narrow, and until
                // that happens the media are presented smaller rather than clipped.
                let natural = ITEM_SIZE_REQUEST * n_columns + SPACING * (n_columns - 1);

                (MIN_ROW_WIDTH, natural.max(MIN_ROW_WIDTH), -1, -1)
            } else {
                // The media are presented in squares, so the height of a row is the size
                // of one of its cells.
                let height = if for_size < 0 {
                    ITEM_SIZE_REQUEST
                } else {
                    cell_size(for_size, n_columns)
                };

                (height, height, -1, -1)
            }
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            if let Some(loading_widget) = self.loading_widget.borrow().as_ref() {
                loading_widget.size_allocate(&gtk::Allocation::new(0, 0, width, height), baseline);
                return;
            }

            let n_columns = self.n_columns();
            // The cells are placed from the edges of the row rather than from the size
            // of a cell, so that the rounding of the divisions is spread between them
            // and the last cell always ends at the edge of the row.
            let total_size = width + SPACING;

            for (index, item) in self.items.borrow().iter().enumerate() {
                let Ok(index) = i32::try_from(index) else {
                    break;
                };

                let x = index * total_size / n_columns;
                let item_width = (index + 1) * total_size / n_columns - SPACING - x;

                item.size_allocate(
                    &gtk::Allocation::new(x, 0, item_width.max(0), height),
                    baseline,
                );
            }
        }
    }

    impl VisualMediaRowItem {
        /// The number of media presented on this row when it is full.
        fn n_columns(&self) -> i32 {
            self.row
                .borrow()
                .as_ref()
                .map_or(1, VisualMediaRow::n_columns)
                .try_into()
                .unwrap_or(1)
        }

        /// Set the row that is presented.
        fn set_row(&self, row: Option<VisualMediaRow>) {
            if *self.row.borrow() == row {
                return;
            }

            // The loading widget belongs to the timeline and can only be presented in a
            // single place, so it is detached before this row presents anything else.
            self.detach_loading_widget();

            let items = row.as_ref().map(VisualMediaRow::items).unwrap_or_default();

            if row.as_ref().is_some_and(VisualMediaRow::is_loading) {
                self.set_n_item_widgets(0);

                if let Some(loading_widget) = items
                    .first()
                    .and_then(|item| item.downcast_ref::<gtk::Widget>())
                {
                    // The widget can still be parented to the row that presented it before
                    // this one.
                    loading_widget.unparent();
                    loading_widget.set_parent(&*self.obj());

                    self.loading_widget.replace(Some(loading_widget.clone()));
                }
            } else {
                self.set_n_item_widgets(items.len());

                for (widget, item) in self.items.borrow().iter().zip(&items) {
                    widget.set_event(item.downcast_ref::<HistoryViewerEvent>().cloned());
                }
            }

            self.row.replace(row);

            self.obj().queue_resize();
            self.obj().notify_row();
        }

        /// Detach the widget presenting the loading item, if this row presents
        /// it.
        fn detach_loading_widget(&self) {
            let Some(loading_widget) = self.loading_widget.take() else {
                return;
            };

            // The widget is shared, so another row can have taken it over already, in
            // which case it must be left alone.
            let obj = self.obj();
            if loading_widget.parent().as_ref() == Some(obj.upcast_ref::<gtk::Widget>()) {
                loading_widget.unparent();
            }
        }

        /// Set the number of widgets presenting media on this row.
        ///
        /// The widgets that are kept are reused, so the media that do not move
        /// are not loaded again.
        fn set_n_item_widgets(&self, n_widgets: usize) {
            let mut items = self.items.borrow_mut();

            while items.len() > n_widgets {
                let item = items.pop().expect("there is a widget to remove");
                item.unparent();
            }

            let obj = self.obj();
            while items.len() < n_widgets {
                let item = VisualMediaItem::new();
                item.set_parent(&*obj);
                items.push(item);
            }
        }
    }
}

glib::wrapper! {
    /// A widget presenting a row of the visual media history.
    pub struct VisualMediaRowItem(ObjectSubclass<imp::VisualMediaRowItem>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl VisualMediaRowItem {
    /// Construct a new empty `VisualMediaRowItem`.
    pub fn new() -> Self {
        glib::Object::new()
    }
}

impl Default for VisualMediaRowItem {
    fn default() -> Self {
        Self::new()
    }
}
