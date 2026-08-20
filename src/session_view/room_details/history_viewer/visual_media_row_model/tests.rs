#![allow(clippy::too_many_lines)] // We do not care if tests are too long.

use std::{cell::RefCell, rc::Rc};

use gtk::{gio, glib, glib::clone, prelude::*};

use super::{MediaAge, VisualMediaRow, VisualMediaRowModel};
use crate::utils::PlaceholderObject;

/// The identifier of the item standing in for the loading item.
const LOADING_ID: &str = "loading";

/// Construct a model over the given store.
///
/// The items are [`PlaceholderObject`]s identified by the number of seconds
/// since the Unix EPOCH at which they were sent, except the loading item.
fn media_row_model(store: &gio::ListStore, n_columns: u32) -> VisualMediaRowModel {
    let model = VisualMediaRowModel::new(|item: &glib::Object| {
        let id = item.downcast_ref::<PlaceholderObject>()?.id();

        if id == LOADING_ID {
            return None;
        }

        let secs = id.parse().expect("the identifier should be a timestamp");
        glib::DateTime::from_unix_local(secs).ok()
    });

    model.set_n_columns(n_columns);
    model.set_model(Some(store.clone()));

    model
}

/// The current time.
fn now() -> glib::DateTime {
    glib::DateTime::now_local().expect("current time should be available")
}

/// An item sent at the given time.
fn media(timestamp: &glib::DateTime) -> PlaceholderObject {
    PlaceholderObject::new(&timestamp.to_unix().to_string())
}

/// An item sent today.
fn this_week() -> PlaceholderObject {
    media(&now())
}

/// An item sent the given number of months ago.
fn months_ago(months: i32) -> PlaceholderObject {
    media(&now().add_months(-months).expect("date should be valid"))
}

/// The loading item.
fn loading() -> PlaceholderObject {
    PlaceholderObject::new(LOADING_ID)
}

/// The shape of the given model, as the number of items of each row and the
/// age of its section.
fn shape(model: &VisualMediaRowModel) -> Vec<(usize, Option<MediaAge>)> {
    (0..model.n_items())
        .map(|position| {
            let row = model
                .item(position)
                .and_downcast::<VisualMediaRow>()
                .expect("row should be a media row");
            (row.items().len(), row.age())
        })
        .collect()
}

/// The sections of the given model, as a `start..end` range for each row.
fn sections(model: &VisualMediaRowModel) -> Vec<(u32, u32)> {
    (0..model.n_items())
        .map(|position| model.section(position))
        .collect()
}

#[gtk::test]
fn rows_are_filled_up_to_the_number_of_columns() {
    let store = gio::ListStore::new::<PlaceholderObject>();
    let model = media_row_model(&store, 3);

    assert_eq!(model.n_items(), 0);

    let items = (0..7).map(|_| this_week()).collect::<Vec<_>>();
    store.extend_from_slice(&items);

    // The last row of the section is partial.
    assert_eq!(
        shape(&model),
        [
            (3, Some(MediaAge::ThisWeek)),
            (3, Some(MediaAge::ThisWeek)),
            (1, Some(MediaAge::ThisWeek)),
        ]
    );

    // The items are presented in order, and none is lost.
    let presented = (0..model.n_items())
        .filter_map(|position| model.item(position).and_downcast::<VisualMediaRow>())
        .flat_map(|row| row.items())
        .collect::<Vec<_>>();
    assert_eq!(presented.len(), items.len());

    for (presented, item) in presented.iter().zip(&items) {
        assert_eq!(presented, item.upcast_ref::<glib::Object>());
    }

    // The whole model is a single section.
    assert_eq!(sections(&model), [(0, 3), (0, 3), (0, 3)]);
}

#[gtk::test]
fn a_row_never_spans_two_sections() {
    let store = gio::ListStore::new::<PlaceholderObject>();
    let model = media_row_model(&store, 3);

    store.extend_from_slice(&[
        this_week(),
        this_week(),
        months_ago(2),
        months_ago(2),
        months_ago(2),
        months_ago(2),
    ]);

    // The first section ends after two items, so its row is left partial instead of
    // being completed with the items of the next section.
    assert_eq!(
        shape(&model),
        [
            (2, Some(MediaAge::ThisWeek)),
            (3, Some(MediaAge::MonthsAgo(2))),
            (1, Some(MediaAge::MonthsAgo(2))),
        ]
    );
    assert_eq!(sections(&model), [(0, 1), (1, 3), (1, 3)]);
}

#[gtk::test]
fn appending_completes_the_last_partial_row() {
    let store = gio::ListStore::new::<PlaceholderObject>();
    let model = media_row_model(&store, 3);

    store.extend_from_slice(&[this_week(), this_week()]);
    assert_eq!(shape(&model), [(2, Some(MediaAge::ThisWeek))]);

    let items_changed = Rc::new(RefCell::new(Vec::new()));
    model.connect_items_changed(clone!(
        #[strong]
        items_changed,
        move |_, position, removed, added| {
            items_changed.borrow_mut().push((position, removed, added));
        }
    ));

    store.extend_from_slice(&[this_week(), this_week(), this_week()]);

    // Only the partial row was replaced, the rows before it were untouched.
    assert_eq!(items_changed.take(), [(0, 1, 2)]);
    assert_eq!(
        shape(&model),
        [(3, Some(MediaAge::ThisWeek)), (2, Some(MediaAge::ThisWeek))]
    );
}

#[gtk::test]
fn appending_older_media_opens_a_section() {
    let store = gio::ListStore::new::<PlaceholderObject>();
    let model = media_row_model(&store, 2);

    store.extend_from_slice(&[this_week(), this_week(), this_week()]);
    assert_eq!(
        shape(&model),
        [(2, Some(MediaAge::ThisWeek)), (1, Some(MediaAge::ThisWeek))]
    );

    store.extend_from_slice(&[months_ago(13), months_ago(13)]);

    // The partial row is not completed with media of another age.
    assert_eq!(
        shape(&model),
        [
            (2, Some(MediaAge::ThisWeek)),
            (1, Some(MediaAge::ThisWeek)),
            (2, Some(MediaAge::YearsAgo(1))),
        ]
    );
    assert_eq!(sections(&model), [(0, 2), (0, 2), (2, 3)]);
}

#[gtk::test]
fn sections_stay_in_order_when_timestamps_do_not() {
    let store = gio::ListStore::new::<PlaceholderObject>();
    let model = media_row_model(&store, 1);

    // The event in the middle is out of order, which happens when a homeserver
    // sends an event with a timestamp that its neighbours do not agree with.
    store.extend_from_slice(&[months_ago(3), this_week(), months_ago(3)]);

    // The out-of-order event is kept in the section it is presented in, instead of
    // opening a more recent section in the middle of the history.
    assert_eq!(
        shape(&model),
        [
            (1, Some(MediaAge::MonthsAgo(3))),
            (1, Some(MediaAge::MonthsAgo(3))),
            (1, Some(MediaAge::MonthsAgo(3))),
        ]
    );
    assert_eq!(sections(&model), [(0, 3), (0, 3), (0, 3)]);
}

#[gtk::test]
fn the_loading_item_is_alone_on_the_last_row_of_the_last_section() {
    let store = gio::ListStore::new::<PlaceholderObject>();
    let model = media_row_model(&store, 3);

    store.append(&loading());

    // On its own, the loading item has no age, so it gets no heading.
    assert_eq!(shape(&model), [(1, None)]);

    store.splice(0, 0, &[this_week(), this_week()]);

    // The partial row of the section is not completed with the loading item, and
    // the loading item joins the section instead of opening one of its own.
    assert_eq!(
        shape(&model),
        [(2, Some(MediaAge::ThisWeek)), (1, Some(MediaAge::ThisWeek))]
    );
    assert_eq!(sections(&model), [(0, 2), (0, 2)]);

    let loading_row = model
        .item(1)
        .and_downcast::<VisualMediaRow>()
        .expect("row should be a media row");
    assert!(loading_row.is_loading());

    // Removing the loading item leaves the media alone.
    store.remove(2);
    assert_eq!(shape(&model), [(2, Some(MediaAge::ThisWeek))]);
}

#[gtk::test]
fn changing_the_number_of_columns_rebuilds_the_rows() {
    let store = gio::ListStore::new::<PlaceholderObject>();
    let model = media_row_model(&store, 2);

    store.extend_from_slice(&(0..5).map(|_| this_week()).collect::<Vec<_>>());
    assert_eq!(
        shape(&model),
        [
            (2, Some(MediaAge::ThisWeek)),
            (2, Some(MediaAge::ThisWeek)),
            (1, Some(MediaAge::ThisWeek)),
        ]
    );

    let items_changed = Rc::new(RefCell::new(Vec::new()));
    model.connect_items_changed(clone!(
        #[strong]
        items_changed,
        move |_, position, removed, added| {
            items_changed.borrow_mut().push((position, removed, added));
        }
    ));

    model.set_n_columns(5);

    assert_eq!(items_changed.take(), [(0, 3, 1)]);
    assert_eq!(shape(&model), [(5, Some(MediaAge::ThisWeek))]);
}

#[gtk::test]
fn removing_the_model_empties_the_rows() {
    let store = gio::ListStore::new::<PlaceholderObject>();
    let model = media_row_model(&store, 2);

    store.extend_from_slice(&[this_week(), this_week(), this_week()]);
    assert_eq!(model.n_items(), 2);

    model.set_model(None::<gio::ListModel>);

    assert_eq!(model.n_items(), 0);
    assert_eq!(shape(&model), []);
}
