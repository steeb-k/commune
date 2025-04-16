#![allow(clippy::too_many_lines)] // We do not care if tests are too long.

use std::{cell::RefCell, rc::Rc};

use assert_matches2::assert_matches;
use gtk::{gio, glib, glib::clone, prelude::*};
use sourceview::prelude::ListModelExt;

use super::{GroupingListGroup, GroupingListModel};
use crate::utils::PlaceholderObject;

#[test]
fn only_singletons_single_item_changes() {
    let list_store = gio::ListStore::new::<PlaceholderObject>();
    let model = GroupingListModel::new(|_lhs, _rhs| false);

    assert_eq!(model.n_items(), 0);
    assert_matches!(model.item(0), None);

    let items_changed = Rc::new(RefCell::new(None));

    model.connect_items_changed(clone!(
        #[strong]
        items_changed,
        move |_, position, removed, added| {
            println!(
                "connect_items_changed: position {position}, removed {removed}, added {added}"
            );
            assert_matches!(
                items_changed.replace(Some((position, removed, added))),
                None
            );
        }
    ));

    model.set_model(Some(list_store.clone()));

    assert_matches!(items_changed.take(), None);
    assert_eq!(model.n_items(), 0);
    assert_matches!(model.item(0), None);

    // Add some initial items.
    list_store.splice(
        0,
        0,
        &[
            PlaceholderObject::new("item 0.0"),
            PlaceholderObject::new("item 1.0"),
            PlaceholderObject::new("item 2.0"),
            PlaceholderObject::new("item 3.0"),
            PlaceholderObject::new("item 4.0"),
        ],
    );

    assert_eq!(items_changed.take(), Some((0, 0, 5)));

    assert_eq!(model.n_items(), 5);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 0.0");
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 1.0");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 2.0");
    assert_matches!(model.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 3.0");
    assert_matches!(model.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 4.0");
    assert_matches!(model.item(5), None);

    // Remove the first item.
    list_store.remove(0);

    assert_eq!(items_changed.take(), Some((0, 1, 0)));

    assert_eq!(model.n_items(), 4);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 1.0");
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 2.0");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 3.0");
    assert_matches!(model.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 4.0");
    assert_matches!(model.item(4), None);

    // Remove the last item.
    list_store.remove(3);

    assert_eq!(items_changed.take(), Some((3, 1, 0)));

    assert_eq!(model.n_items(), 3);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 1.0");
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 2.0");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 3.0");
    assert_matches!(model.item(3), None);

    // Remove the item in the middle.
    list_store.remove(1);

    assert_eq!(items_changed.take(), Some((1, 1, 0)));

    assert_eq!(model.n_items(), 2);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 1.0");
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 3.0");
    assert_matches!(model.item(2), None);

    // Re-add first item.
    list_store.insert(0, &PlaceholderObject::new("item 0.1"));

    assert_eq!(items_changed.take(), Some((0, 0, 1)));

    assert_eq!(model.n_items(), 3);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 0.1");
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 1.0");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 3.0");
    assert_matches!(model.item(3), None);

    // Re-add middle item.
    list_store.insert(2, &PlaceholderObject::new("item 2.1"));

    assert_eq!(items_changed.take(), Some((2, 0, 1)));

    assert_eq!(model.n_items(), 4);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 0.1");
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 1.0");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 2.1");
    assert_matches!(model.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 3.0");
    assert_matches!(model.item(4), None);

    // Re-add end item.
    list_store.insert(4, &PlaceholderObject::new("item 4.1"));

    assert_eq!(items_changed.take(), Some((4, 0, 1)));

    assert_eq!(model.n_items(), 5);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 0.1");
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 1.0");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 2.1");
    assert_matches!(model.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 3.0");
    assert_matches!(model.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 4.1");
    assert_matches!(model.item(5), None);

    // Replace first item.
    list_store.splice(0, 1, &[PlaceholderObject::new("item 0.2")]);

    assert_eq!(items_changed.take(), Some((0, 1, 1)));

    assert_eq!(model.n_items(), 5);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 0.2");
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 1.0");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 2.1");
    assert_matches!(model.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 3.0");
    assert_matches!(model.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 4.1");
    assert_matches!(model.item(5), None);

    // Replace second item.
    list_store.splice(1, 1, &[PlaceholderObject::new("item 1.1")]);

    assert_eq!(items_changed.take(), Some((1, 1, 1)));

    assert_eq!(model.n_items(), 5);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 0.2");
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 1.1");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 2.1");
    assert_matches!(model.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 3.0");
    assert_matches!(model.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 4.1");
    assert_matches!(model.item(5), None);

    // Replace last item.
    list_store.splice(4, 1, &[PlaceholderObject::new("item 4.2")]);

    assert_matches!(items_changed.take(), Some((4, 1, 1)));

    assert_eq!(model.n_items(), 5);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 0.2");
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 1.1");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 2.1");
    assert_matches!(model.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 3.0");
    assert_matches!(model.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 4.2");
    assert_matches!(model.item(5), None);
}

#[test]
fn only_singletons_multiple_item_changes() {
    let list_store = gio::ListStore::new::<PlaceholderObject>();
    let model = GroupingListModel::new(|_lhs, _rhs| false);

    assert_eq!(model.n_items(), 0);
    assert_matches!(model.item(0), None);

    let items_changed = Rc::new(RefCell::new(None));

    model.connect_items_changed(clone!(
        #[strong]
        items_changed,
        move |_, position, removed, added| {
            println!(
                "connect_items_changed: position {position}, removed {removed}, added {added}"
            );
            assert_matches!(
                items_changed.replace(Some((position, removed, added))),
                None
            );
        }
    ));

    model.set_model(Some(list_store.clone()));

    assert_matches!(items_changed.take(), None);
    assert_eq!(model.n_items(), 0);
    assert_matches!(model.item(0), None);

    // Add some initial items.
    list_store.splice(
        0,
        0,
        &[
            PlaceholderObject::new("item 0.0"),
            PlaceholderObject::new("item 1.0"),
            PlaceholderObject::new("item 2.0"),
            PlaceholderObject::new("item 3.0"),
            PlaceholderObject::new("item 4.0"),
        ],
    );

    assert_matches!(items_changed.take(), Some((0, 0, 5)));

    assert_eq!(model.n_items(), 5);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 0.0");
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 1.0");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 2.0");
    assert_matches!(model.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 3.0");
    assert_matches!(model.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 4.0");
    assert_matches!(model.item(5), None);

    // Remove the 2 first items.
    list_store.splice(0, 2, &[] as &[glib::Object]);

    assert_matches!(items_changed.take(), Some((0, 2, 0)));

    assert_eq!(model.n_items(), 3);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 2.0");
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 3.0");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 4.0");
    assert_matches!(model.item(3), None);

    // Remove the 2 last items.
    list_store.splice(1, 2, &[] as &[glib::Object]);

    assert_eq!(items_changed.take(), Some((1, 2, 0)));

    assert_eq!(model.n_items(), 1);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 2.0");
    assert_matches!(model.item(1), None);

    // Re-add the 2 first items.
    list_store.splice(
        0,
        0,
        &[
            PlaceholderObject::new("item 0.1"),
            PlaceholderObject::new("item 1.1"),
        ],
    );

    assert_matches!(items_changed.take(), Some((0, 0, 2)));

    assert_eq!(model.n_items(), 3);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 0.1");
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 1.1");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 2.0");
    assert_matches!(model.item(3), None);

    // Re-add the 2 last items.
    list_store.splice(
        3,
        0,
        &[
            PlaceholderObject::new("item 3.1"),
            PlaceholderObject::new("item 4.1"),
        ],
    );

    assert_matches!(items_changed.take(), Some((3, 0, 2)));

    assert_eq!(model.n_items(), 5);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 0.1");
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 1.1");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 2.0");
    assert_matches!(model.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 3.1");
    assert_matches!(model.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 4.1");
    assert_matches!(model.item(5), None);

    // Remove the 3 middle items.
    list_store.splice(1, 3, &[] as &[glib::Object]);

    assert_eq!(items_changed.take(), Some((1, 3, 0)));

    assert_eq!(model.n_items(), 2);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 0.1");
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 4.1");
    assert_matches!(model.item(2), None);

    // Re-add the 3 middle items.
    list_store.splice(
        1,
        0,
        &[
            PlaceholderObject::new("item 1.2"),
            PlaceholderObject::new("item 2.1"),
            PlaceholderObject::new("item 3.2"),
        ],
    );

    assert_eq!(items_changed.take(), Some((1, 0, 3)));

    assert_eq!(model.n_items(), 5);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 0.1");
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 1.2");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 2.1");
    assert_matches!(model.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 3.2");
    assert_matches!(model.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 4.1");
    assert_matches!(model.item(5), None);

    // Replace the 2 first items.
    list_store.splice(
        0,
        2,
        &[
            PlaceholderObject::new("item 0.2"),
            PlaceholderObject::new("item 1.3"),
        ],
    );

    assert_matches!(items_changed.take(), Some((0, 2, 2)));

    assert_eq!(model.n_items(), 5);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 0.2");
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 1.3");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 2.1");
    assert_matches!(model.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 3.2");
    assert_matches!(model.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 4.1");
    assert_matches!(model.item(5), None);

    // Replace 2 middle items.
    list_store.splice(
        2,
        2,
        &[
            PlaceholderObject::new("item 2.2"),
            PlaceholderObject::new("item 3.3"),
        ],
    );

    assert_matches!(items_changed.take(), Some((2, 2, 2)));

    assert_eq!(model.n_items(), 5);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 0.2");
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 1.3");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 2.2");
    assert_matches!(model.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 3.3");
    assert_matches!(model.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 4.1");
    assert_matches!(model.item(5), None);

    // Replace the 2 last items.
    list_store.splice(
        3,
        2,
        &[
            PlaceholderObject::new("item 3.4"),
            PlaceholderObject::new("item 4.2"),
        ],
    );

    assert_eq!(items_changed.take(), Some((3, 2, 2)));

    assert_eq!(model.n_items(), 5);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 0.2");
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 1.3");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 2.2");
    assert_matches!(model.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 3.4");
    assert_matches!(model.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 4.2");
    assert_matches!(model.item(5), None);

    // Remove all the items.
    list_store.remove_all();

    assert_eq!(items_changed.take(), Some((0, 5, 0)));

    assert_eq!(model.n_items(), 0);
    assert_matches!(model.item(0), None);
}

#[test]
fn only_group_single_item_changes() {
    let list_store = gio::ListStore::new::<PlaceholderObject>();
    let model = GroupingListModel::new(|_lhs, _rhs| true);

    assert_eq!(model.n_items(), 0);
    assert_matches!(model.item(0), None);

    let model_items_changed = Rc::new(RefCell::new(None));

    model.connect_items_changed(clone!(
        #[strong]
        model_items_changed,
        move |_, position, removed, added| {
            println!(
                "model.connect_items_changed: position {position}, removed {removed}, added {added}"
            );
            assert_matches!(
                model_items_changed.replace(Some((position, removed, added))),
                None
            );
        }
    ));

    model.set_model(Some(list_store.clone()));

    assert_matches!(model_items_changed.take(), None);
    assert_eq!(model.n_items(), 0);
    assert_matches!(model.item(0), None);

    // Add some initial items.
    list_store.splice(
        0,
        0,
        &[
            PlaceholderObject::new("item 0.0"),
            PlaceholderObject::new("item 1.0"),
            PlaceholderObject::new("item 2.0"),
            PlaceholderObject::new("item 3.0"),
            PlaceholderObject::new("item 4.0"),
        ],
    );

    assert_eq!(model_items_changed.take(), Some((0, 0, 1)));

    assert_eq!(model.n_items(), 1);
    assert_matches!(
        model.item(0).and_downcast::<GroupingListGroup>(),
        Some(group)
    );
    assert_matches!(model.item(1), None);

    assert_eq!(group.n_items(), 5);
    assert_matches!(group.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 0.0");
    assert_matches!(group.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 1.0");
    assert_matches!(group.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 2.0");
    assert_matches!(group.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 3.0");
    assert_matches!(group.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 4.0");
    assert_matches!(group.item(5), None);

    let group_items_changed = Rc::new(RefCell::new(None));

    group.connect_items_changed(clone!(
            #[strong]
            group_items_changed,
            move |_, position, removed, added| {
                println!(
                    "group.connect_items_changed: position {position}, removed {removed}, added {added}"
                );
                assert_matches!(
                    group_items_changed.replace(Some((position, removed, added))),
                    None
                );
            }
        ));

    // Remove the first item.
    list_store.remove(0);

    assert_eq!(model_items_changed.take(), None);
    assert_eq!(model.n_items(), 1);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group
    );
    assert_matches!(model.item(1), None);

    assert_eq!(group_items_changed.take(), Some((0, 1, 0)));

    assert_eq!(group.n_items(), 4);
    assert_matches!(group.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 1.0");
    assert_matches!(group.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 2.0");
    assert_matches!(group.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 3.0");
    assert_matches!(group.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 4.0");
    assert_matches!(group.item(4), None);

    // Remove the last item.
    list_store.remove(3);

    assert_eq!(model_items_changed.take(), None);
    assert_eq!(model.n_items(), 1);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group
    );
    assert_matches!(model.item(1), None);

    assert_eq!(group_items_changed.take(), Some((3, 1, 0)));

    assert_eq!(group.n_items(), 3);
    assert_matches!(group.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 1.0");
    assert_matches!(group.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 2.0");
    assert_matches!(group.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 3.0");
    assert_matches!(group.item(3), None);

    // Remove the item in the middle.
    list_store.remove(1);

    assert_eq!(model_items_changed.take(), None);
    assert_eq!(model.n_items(), 1);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group
    );
    assert_matches!(model.item(1), None);

    assert_eq!(group_items_changed.take(), Some((1, 1, 0)));

    assert_eq!(group.n_items(), 2);
    assert_matches!(group.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 1.0");
    assert_matches!(group.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 3.0");
    assert_matches!(group.item(2), None);

    // Re-add first item.
    list_store.insert(0, &PlaceholderObject::new("item 0.1"));

    assert_eq!(model_items_changed.take(), None);
    assert_eq!(model.n_items(), 1);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group
    );
    assert_matches!(model.item(1), None);

    assert_eq!(group_items_changed.take(), Some((0, 0, 1)));

    assert_eq!(group.n_items(), 3);
    assert_matches!(group.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 0.1");
    assert_matches!(group.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 1.0");
    assert_matches!(group.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 3.0");
    assert_matches!(group.item(3), None);

    // Re-add middle item.
    list_store.insert(2, &PlaceholderObject::new("item 2.1"));

    assert_eq!(model_items_changed.take(), None);
    assert_eq!(model.n_items(), 1);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group
    );
    assert_matches!(model.item(1), None);

    assert_eq!(group_items_changed.take(), Some((2, 0, 1)));

    assert_eq!(group.n_items(), 4);
    assert_matches!(group.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 0.1");
    assert_matches!(group.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 1.0");
    assert_matches!(group.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 2.1");
    assert_matches!(group.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 3.0");
    assert_matches!(group.item(4), None);

    // Re-add end item.
    list_store.insert(4, &PlaceholderObject::new("item 4.1"));

    assert_matches!(model_items_changed.take(), None);
    assert_eq!(model.n_items(), 1);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group
    );
    assert_matches!(model.item(1), None);

    assert_eq!(group_items_changed.take(), Some((4, 0, 1)));

    assert_eq!(group.n_items(), 5);
    assert_matches!(group.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 0.1");
    assert_matches!(group.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 1.0");
    assert_matches!(group.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 2.1");
    assert_matches!(group.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 3.0");
    assert_matches!(group.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 4.1");
    assert_matches!(group.item(5), None);

    // Replace first item.
    list_store.splice(0, 1, &[PlaceholderObject::new("item 0.2")]);

    assert_matches!(model_items_changed.take(), None);
    assert_eq!(model.n_items(), 1);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group
    );
    assert_matches!(model.item(1), None);

    assert_eq!(group_items_changed.take(), Some((0, 1, 1)));

    assert_eq!(group.n_items(), 5);
    assert_matches!(group.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 0.2");
    assert_matches!(group.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 1.0");
    assert_matches!(group.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 2.1");
    assert_matches!(group.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 3.0");
    assert_matches!(group.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 4.1");
    assert_matches!(group.item(5), None);

    // Replace second item.
    list_store.splice(1, 1, &[PlaceholderObject::new("item 1.1")]);

    assert_eq!(model_items_changed.take(), None);
    assert_eq!(model.n_items(), 1);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group
    );
    assert_matches!(model.item(1), None);

    assert_eq!(group_items_changed.take(), Some((1, 1, 1)));

    assert_eq!(group.n_items(), 5);
    assert_matches!(group.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 0.2");
    assert_matches!(group.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 1.1");
    assert_matches!(group.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 2.1");
    assert_matches!(group.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 3.0");
    assert_matches!(group.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 4.1");
    assert_matches!(group.item(5), None);

    // Replace last item.
    list_store.splice(4, 1, &[PlaceholderObject::new("item 4.2")]);

    assert_eq!(model_items_changed.take(), None);
    assert_eq!(model.n_items(), 1);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group
    );
    assert_matches!(model.item(1), None);

    assert_eq!(group_items_changed.take(), Some((4, 1, 1)));

    assert_eq!(group.n_items(), 5);
    assert_matches!(group.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 0.2");
    assert_matches!(group.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 1.1");
    assert_matches!(group.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 2.1");
    assert_matches!(group.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 3.0");
    assert_matches!(group.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 4.2");
    assert_matches!(group.item(5), None);
}

#[test]
fn only_group_multiple_item_changes() {
    let list_store = gio::ListStore::new::<PlaceholderObject>();
    let model = GroupingListModel::new(|_lhs, _rhs| true);

    assert_eq!(model.n_items(), 0);
    assert_matches!(model.item(0), None);

    let model_items_changed = Rc::new(RefCell::new(None));

    model.connect_items_changed(clone!(
        #[strong]
        model_items_changed,
        move |_, position, removed, added| {
            println!(
                "model.connect_items_changed: position {position}, removed {removed}, added {added}"
            );
            assert_eq!(
                model_items_changed.replace(Some((position, removed, added))),
                None
            );
        }
    ));

    model.set_model(Some(list_store.clone()));

    assert_eq!(model_items_changed.take(), None);
    assert_eq!(model.n_items(), 0);
    assert_matches!(model.item(0), None);

    // Add some initial items.
    list_store.splice(
        0,
        0,
        &[
            PlaceholderObject::new("item 0.0"),
            PlaceholderObject::new("item 1.0"),
            PlaceholderObject::new("item 2.0"),
            PlaceholderObject::new("item 3.0"),
            PlaceholderObject::new("item 4.0"),
        ],
    );

    assert_eq!(model_items_changed.take(), Some((0, 0, 1)));

    assert_eq!(model.n_items(), 1);
    assert_matches!(
        model.item(0).and_downcast::<GroupingListGroup>(),
        Some(group)
    );
    assert_matches!(model.item(1), None);

    assert_eq!(group.n_items(), 5);
    assert_matches!(group.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 0.0");
    assert_matches!(group.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 1.0");
    assert_matches!(group.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 2.0");
    assert_matches!(group.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 3.0");
    assert_matches!(group.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 4.0");
    assert_matches!(group.item(5), None);

    let group_items_changed = Rc::new(RefCell::new(None));

    group.connect_items_changed(clone!(
            #[strong]
            group_items_changed,
            move |_, position, removed, added| {
                println!(
                    "group.connect_items_changed: position {position}, removed {removed}, added {added}"
                );
                assert_eq!(
                    group_items_changed.replace(Some((position, removed, added))),
                    None
                );
            }
        ));

    // Remove the 2 first items.
    list_store.splice(0, 2, &[] as &[glib::Object]);

    assert_eq!(model_items_changed.take(), None);
    assert_eq!(model.n_items(), 1);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group
    );
    assert_matches!(model.item(1), None);

    assert_eq!(group_items_changed.take(), Some((0, 2, 0)));

    assert_eq!(group.n_items(), 3);
    assert_matches!(group.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 2.0");
    assert_matches!(group.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 3.0");
    assert_matches!(group.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 4.0");
    assert_matches!(group.item(3), None);

    // Re-add the 2 first items.
    list_store.splice(
        0,
        0,
        &[
            PlaceholderObject::new("item 0.1"),
            PlaceholderObject::new("item 1.1"),
        ],
    );

    assert_eq!(model_items_changed.take(), None);
    assert_eq!(model.n_items(), 1);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group
    );
    assert_matches!(model.item(1), None);

    assert_eq!(group_items_changed.take(), Some((0, 0, 2)));

    assert_eq!(group.n_items(), 5);
    assert_matches!(group.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 0.1");
    assert_matches!(group.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 1.1");
    assert_matches!(group.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 2.0");
    assert_matches!(group.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 3.0");
    assert_matches!(group.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 4.0");
    assert_matches!(group.item(5), None);

    // Remove the 2 last items.
    list_store.splice(3, 2, &[] as &[glib::Object]);

    assert_eq!(model_items_changed.take(), None);
    assert_eq!(model.n_items(), 1);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group
    );
    assert_matches!(model.item(1), None);

    assert_eq!(group_items_changed.take(), Some((3, 2, 0)));

    assert_eq!(group.n_items(), 3);
    assert_matches!(group.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 0.1");
    assert_matches!(group.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 1.1");
    assert_matches!(group.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 2.0");
    assert_matches!(group.item(3), None);

    // Re-add the 2 last items.
    list_store.splice(
        3,
        0,
        &[
            PlaceholderObject::new("item 3.1"),
            PlaceholderObject::new("item 4.1"),
        ],
    );

    assert_eq!(model_items_changed.take(), None);
    assert_eq!(model.n_items(), 1);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group
    );
    assert_matches!(model.item(1), None);

    assert_eq!(group_items_changed.take(), Some((3, 0, 2)));

    assert_eq!(group.n_items(), 5);
    assert_matches!(group.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 0.1");
    assert_matches!(group.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 1.1");
    assert_matches!(group.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 2.0");
    assert_matches!(group.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 3.1");
    assert_matches!(group.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 4.1");
    assert_matches!(group.item(5), None);

    // Remove the 3 middle items.
    list_store.splice(1, 3, &[] as &[glib::Object]);

    assert_eq!(model_items_changed.take(), None);
    assert_eq!(model.n_items(), 1);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group
    );
    assert_matches!(model.item(1), None);

    assert_eq!(group_items_changed.take(), Some((1, 3, 0)));

    assert_eq!(group.n_items(), 2);
    assert_matches!(group.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 0.1");
    assert_matches!(group.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 4.1");
    assert_matches!(group.item(2), None);

    // Re-add the 3 middle items.
    list_store.splice(
        1,
        0,
        &[
            PlaceholderObject::new("item 1.2"),
            PlaceholderObject::new("item 2.1"),
            PlaceholderObject::new("item 3.2"),
        ],
    );

    assert_eq!(model_items_changed.take(), None);
    assert_eq!(model.n_items(), 1);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group
    );
    assert_matches!(model.item(1), None);

    assert_eq!(group_items_changed.take(), Some((1, 0, 3)));

    assert_eq!(group.n_items(), 5);
    assert_matches!(group.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 0.1");
    assert_matches!(group.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 1.2");
    assert_matches!(group.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 2.1");
    assert_matches!(group.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 3.2");
    assert_matches!(group.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 4.1");
    assert_matches!(group.item(5), None);

    // Replace the 2 first items.
    list_store.splice(
        0,
        2,
        &[
            PlaceholderObject::new("item 0.2"),
            PlaceholderObject::new("item 1.3"),
        ],
    );

    assert_eq!(model_items_changed.take(), None);
    assert_eq!(model.n_items(), 1);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group
    );
    assert_matches!(model.item(1), None);

    assert_eq!(group_items_changed.take(), Some((0, 2, 2)));

    assert_eq!(group.n_items(), 5);
    assert_matches!(group.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 0.2");
    assert_matches!(group.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 1.3");
    assert_matches!(group.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 2.1");
    assert_matches!(group.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 3.2");
    assert_matches!(group.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 4.1");
    assert_matches!(group.item(5), None);

    // Replace 2 middle items.
    list_store.splice(
        2,
        2,
        &[
            PlaceholderObject::new("item 2.2"),
            PlaceholderObject::new("item 3.3"),
        ],
    );

    assert_eq!(model_items_changed.take(), None);
    assert_eq!(model.n_items(), 1);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group
    );
    assert_matches!(model.item(1), None);

    assert_eq!(group_items_changed.take(), Some((2, 2, 2)));

    assert_eq!(group.n_items(), 5);
    assert_matches!(group.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 0.2");
    assert_matches!(group.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 1.3");
    assert_matches!(group.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 2.2");
    assert_matches!(group.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 3.3");
    assert_matches!(group.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 4.1");
    assert_matches!(group.item(5), None);

    // Replace the 2 last items.
    list_store.splice(
        3,
        2,
        &[
            PlaceholderObject::new("item 3.4"),
            PlaceholderObject::new("item 4.2"),
        ],
    );

    assert_eq!(model_items_changed.take(), None);
    assert_eq!(model.n_items(), 1);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group
    );
    assert_matches!(model.item(1), None);

    assert_eq!(group_items_changed.take(), Some((3, 2, 2)));

    assert_eq!(group.n_items(), 5);
    assert_matches!(group.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 0.2");
    assert_matches!(group.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 1.3");
    assert_matches!(group.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 2.2");
    assert_matches!(group.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 3.4");
    assert_matches!(group.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "item 4.2");
    assert_matches!(group.item(5), None);

    // Remove all the items.
    list_store.remove_all();

    assert_eq!(model_items_changed.take(), Some((0, 1, 0)));

    assert_eq!(model.n_items(), 0);
    assert_matches!(model.item(0), None);

    assert_eq!(group_items_changed.take(), None);
}

#[test]
fn mixed_with_group_at_start() {
    let list_store = gio::ListStore::new::<PlaceholderObject>();
    // Group objects that start with "group".
    let model = GroupingListModel::new(|lhs, rhs| {
        lhs.downcast_ref::<PlaceholderObject>()
            .is_some_and(|obj| obj.id().starts_with("group"))
            && rhs
                .downcast_ref::<PlaceholderObject>()
                .is_some_and(|obj| obj.id().starts_with("group"))
    });

    assert_eq!(model.n_items(), 0);
    assert_matches!(model.item(0), None);

    let model_items_changed = Rc::new(RefCell::new(None));

    model.connect_items_changed(clone!(
        #[strong]
        model_items_changed,
        move |_, position, removed, added| {
            println!(
                "model.connect_items_changed: position {position}, removed {removed}, added {added}"
            );
            assert_eq!(
                model_items_changed.replace(Some((position, removed, added))),
                None
            );
        }
    ));

    model.set_model(Some(list_store.clone()));

    assert_eq!(model_items_changed.take(), None);
    assert_eq!(model.n_items(), 0);
    assert_matches!(model.item(0), None);

    // Add some initial items.
    list_store.splice(
        0,
        0,
        &[
            PlaceholderObject::new("group 1 - item 0.0"),
            PlaceholderObject::new("group 1 - item 1.0"),
            PlaceholderObject::new("group 1 - item 2.0"),
            PlaceholderObject::new("singleton 1.0"),
            PlaceholderObject::new("singleton 2.0"),
        ],
    );

    assert_eq!(model_items_changed.take(), Some((0, 0, 3)));

    assert_eq!(model.n_items(), 3);
    assert_matches!(
        model.item(0).and_downcast::<GroupingListGroup>(),
        Some(group1)
    );
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 1.0");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.0");
    assert_matches!(model.item(3), None);

    assert_eq!(group1.n_items(), 3);
    assert_matches!(
        group1.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 0.0");
    assert_matches!(
        group1.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 1.0");
    assert_matches!(
        group1.item(2).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 2.0");
    assert_matches!(group1.item(3), None);

    let group1_items_changed = Rc::new(RefCell::new(None));

    group1.connect_items_changed(clone!(
            #[strong]
            group1_items_changed,
            move |_, position, removed, added| {
                println!(
                    "group1.connect_items_changed: position {position}, removed {removed}, added {added}"
                );
                assert_eq!(
                    group1_items_changed.replace(Some((position, removed, added))),
                    None
                );
            }
        ));

    // Remove the first item in the group.
    list_store.remove(0);

    assert_eq!(model_items_changed.take(), None);

    assert_eq!(model.n_items(), 3);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );

    assert_eq!(group1_items_changed.take(), Some((0, 1, 0)));

    assert_eq!(group1.n_items(), 2);
    assert_matches!(
        group1.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 1.0");
    assert_matches!(
        group1.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 2.0");
    assert_matches!(group1.item(2), None);

    // Re-add the first item in the group.
    list_store.insert(0, &PlaceholderObject::new("group 1 - item 0.1"));

    assert_eq!(model_items_changed.take(), None);
    assert_eq!(model.n_items(), 3);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );

    assert_eq!(group1_items_changed.take(), Some((0, 0, 1)));

    assert_eq!(group1.n_items(), 3);
    assert_matches!(
        group1.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 0.1");
    assert_matches!(
        group1.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 1.0");
    assert_matches!(
        group1.item(2).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 2.0");
    assert_matches!(group1.item(3), None);

    // Remove the middle item in the group.
    list_store.remove(1);

    assert_eq!(model_items_changed.take(), None);

    assert_eq!(model.n_items(), 3);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );

    assert_eq!(group1_items_changed.take(), Some((1, 1, 0)));

    assert_eq!(group1.n_items(), 2);
    assert_matches!(
        group1.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 0.1");
    assert_matches!(
        group1.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 2.0");
    assert_matches!(group1.item(2), None);

    // Re-add the middle item in the group.
    list_store.insert(1, &PlaceholderObject::new("group 1 - item 1.1"));

    assert_eq!(model_items_changed.take(), None);

    assert_eq!(model.n_items(), 3);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );

    assert_eq!(group1_items_changed.take(), Some((1, 0, 1)));

    assert_eq!(group1.n_items(), 3);
    assert_matches!(
        group1.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 0.1");
    assert_matches!(
        group1.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 1.1");
    assert_matches!(
        group1.item(2).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 2.0");
    assert_matches!(group1.item(3), None);

    // Remove the last item in the group.
    list_store.remove(2);

    assert_eq!(model_items_changed.take(), None);
    assert_eq!(model.n_items(), 3);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );

    assert_eq!(group1_items_changed.take(), Some((2, 1, 0)));

    assert_eq!(group1.n_items(), 2);
    assert_matches!(
        group1.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 0.1");
    assert_matches!(
        group1.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 1.1");
    assert_matches!(group1.item(2), None);

    // Re-add the last item in the group.
    list_store.insert(2, &PlaceholderObject::new("group 1 - item 2.1"));

    assert_eq!(model_items_changed.take(), None);

    assert_eq!(model.n_items(), 3);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );

    assert_eq!(group1_items_changed.take(), Some((2, 0, 1)));

    assert_eq!(group1.n_items(), 3);
    assert_matches!(
        group1.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 0.1");
    assert_matches!(
        group1.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 1.1");
    assert_matches!(
        group1.item(2).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 2.1");
    assert_matches!(group1.item(3), None);

    // Remove group and first singleton.
    list_store.splice(0, 4, &[] as &[glib::Object]);

    assert_eq!(model_items_changed.take(), Some((0, 2, 0)));

    assert_eq!(model.n_items(), 1);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.0");
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), None);

    assert_eq!(group1_items_changed.take(), None);
}

#[test]
fn mixed_with_group_in_middle() {
    let list_store = gio::ListStore::new::<PlaceholderObject>();
    // Group objects that start with "group".
    let model = GroupingListModel::new(|lhs, rhs| {
        lhs.downcast_ref::<PlaceholderObject>()
            .is_some_and(|obj| obj.id().starts_with("group"))
            && rhs
                .downcast_ref::<PlaceholderObject>()
                .is_some_and(|obj| obj.id().starts_with("group"))
    });

    assert_eq!(model.n_items(), 0);
    assert_matches!(model.item(0), None);

    let model_items_changed = Rc::new(RefCell::new(None));

    model.connect_items_changed(clone!(
        #[strong]
        model_items_changed,
        move |_, position, removed, added| {
            println!(
                "model.connect_items_changed: position {position}, removed {removed}, added {added}"
            );
            assert_matches!(
                model_items_changed.replace(Some((position, removed, added))),
                None
            );
        }
    ));

    model.set_model(Some(list_store.clone()));

    assert_matches!(model_items_changed.take(), None);
    assert_eq!(model.n_items(), 0);
    assert_matches!(model.item(0), None);

    // Add some initial items.
    list_store.splice(
        0,
        0,
        &[
            PlaceholderObject::new("singleton 0.0"),
            PlaceholderObject::new("group 1 - item 0.0"),
            PlaceholderObject::new("group 1 - item 1.0"),
            PlaceholderObject::new("group 1 - item 2.0"),
            PlaceholderObject::new("singleton 2.0"),
        ],
    );

    assert_eq!(model_items_changed.take(), Some((0, 0, 3)));

    assert_eq!(model.n_items(), 3);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 0.0");
    assert_matches!(
        model.item(1).and_downcast::<GroupingListGroup>(),
        Some(group1)
    );
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.0");
    assert_matches!(model.item(3), None);

    assert_eq!(group1.n_items(), 3);
    assert_matches!(
        group1.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 0.0");
    assert_matches!(
        group1.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 1.0");
    assert_matches!(
        group1.item(2).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 2.0");
    assert_matches!(group1.item(3), None);

    let group1_items_changed = Rc::new(RefCell::new(None));

    group1.connect_items_changed(clone!(
            #[strong]
            group1_items_changed,
            move |_, position, removed, added| {
                println!(
                    "group1.connect_items_changed: position {position}, removed {removed}, added {added}"
                );
                assert_matches!(
                    group1_items_changed.replace(Some((position, removed, added))),
                    None
                );
            }
        ));

    // Remove the first item in the group.
    list_store.remove(1);

    assert_eq!(model_items_changed.take(), None);

    assert_eq!(model.n_items(), 3);
    assert_eq!(
        model.item(1).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );

    assert_eq!(group1_items_changed.take(), Some((0, 1, 0)));

    assert_eq!(group1.n_items(), 2);
    assert_matches!(
        group1.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 1.0");
    assert_matches!(
        group1.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 2.0");
    assert_matches!(group1.item(2), None);

    // Re-add the first item in the group.
    list_store.insert(1, &PlaceholderObject::new("group 1 - item 0.1"));

    assert_eq!(model_items_changed.take(), None);

    assert_eq!(model.n_items(), 3);
    assert_eq!(
        model.item(1).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );

    assert_eq!(group1_items_changed.take(), Some((0, 0, 1)));

    assert_eq!(group1.n_items(), 3);
    assert_matches!(
        group1.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 0.1");
    assert_matches!(
        group1.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 1.0");
    assert_matches!(
        group1.item(2).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 2.0");
    assert_matches!(group1.item(3), None);

    // Remove the middle item in the group.
    list_store.remove(2);

    assert_eq!(model_items_changed.take(), None);

    assert_eq!(model.n_items(), 3);
    assert_eq!(
        model.item(1).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );

    assert_eq!(group1_items_changed.take(), Some((1, 1, 0)));

    assert_eq!(group1.n_items(), 2);
    assert_matches!(
        group1.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 0.1");
    assert_matches!(
        group1.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 2.0");
    assert_matches!(group1.item(2), None);

    // Re-add the middle item in the group.
    list_store.insert(2, &PlaceholderObject::new("group 1 - item 1.1"));

    assert_eq!(model_items_changed.take(), None);

    assert_eq!(model.n_items(), 3);
    assert_eq!(
        model.item(1).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );

    assert_eq!(group1_items_changed.take(), Some((1, 0, 1)));

    assert_eq!(group1.n_items(), 3);
    assert_matches!(
        group1.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 0.1");
    assert_matches!(
        group1.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 1.1");
    assert_matches!(
        group1.item(2).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 2.0");
    assert_matches!(group1.item(3), None);

    // Remove the last item in the group.
    list_store.remove(3);

    assert_eq!(model_items_changed.take(), None);

    assert_eq!(model.n_items(), 3);
    assert_eq!(
        model.item(1).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );

    assert_eq!(group1_items_changed.take(), Some((2, 1, 0)));

    assert_eq!(group1.n_items(), 2);
    assert_matches!(
        group1.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 0.1");
    assert_matches!(
        group1.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 1.1");
    assert_matches!(group1.item(2), None);

    // Re-add the last item in the group.
    list_store.insert(3, &PlaceholderObject::new("group 1 - item 2.1"));

    assert_eq!(model_items_changed.take(), None);

    assert_eq!(model.n_items(), 3);
    assert_eq!(
        model.item(1).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );

    assert_eq!(group1_items_changed.take(), Some((2, 0, 1)));

    assert_eq!(group1.n_items(), 3);
    assert_matches!(
        group1.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 0.1");
    assert_matches!(
        group1.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 1.1");
    assert_matches!(
        group1.item(2).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 2.1");
    assert_matches!(group1.item(3), None);

    // Remove group.
    list_store.splice(1, 3, &[] as &[glib::Object]);

    assert_eq!(model_items_changed.take(), Some((1, 1, 0)));

    assert_eq!(model.n_items(), 2);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 0.0");
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.0");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), None);

    assert_eq!(group1_items_changed.take(), None);
}

#[test]
fn mixed_with_group_at_end() {
    let list_store = gio::ListStore::new::<PlaceholderObject>();
    // Group objects that start with "group".
    let model = GroupingListModel::new(|lhs, rhs| {
        lhs.downcast_ref::<PlaceholderObject>()
            .is_some_and(|obj| obj.id().starts_with("group"))
            && rhs
                .downcast_ref::<PlaceholderObject>()
                .is_some_and(|obj| obj.id().starts_with("group"))
    });

    assert_eq!(model.n_items(), 0);
    assert_matches!(model.item(0), None);

    let model_items_changed = Rc::new(RefCell::new(None));

    model.connect_items_changed(clone!(
        #[strong]
        model_items_changed,
        move |_, position, removed, added| {
            println!(
                "model.connect_items_changed: position {position}, removed {removed}, added {added}"
            );
            assert_matches!(
                model_items_changed.replace(Some((position, removed, added))),
                None
            );
        }
    ));

    model.set_model(Some(list_store.clone()));

    assert_matches!(model_items_changed.take(), None);
    assert_eq!(model.n_items(), 0);
    assert_matches!(model.item(0), None);

    // Add some initial items.
    list_store.splice(
        0,
        0,
        &[
            PlaceholderObject::new("singleton 0.0"),
            PlaceholderObject::new("singleton 1.0"),
            PlaceholderObject::new("group 1 - item 0.0"),
            PlaceholderObject::new("group 1 - item 1.0"),
            PlaceholderObject::new("group 1 - item 2.0"),
        ],
    );

    assert_eq!(model_items_changed.take(), Some((0, 0, 3)));

    assert_eq!(model.n_items(), 3);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 0.0");
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 1.0");
    assert_matches!(
        model.item(2).and_downcast::<GroupingListGroup>(),
        Some(group1)
    );
    assert_matches!(model.item(3), None);

    assert_eq!(group1.n_items(), 3);
    assert_matches!(
        group1.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 0.0");
    assert_matches!(
        group1.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 1.0");
    assert_matches!(
        group1.item(2).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 2.0");
    assert_matches!(group1.item(3), None);

    let group1_items_changed = Rc::new(RefCell::new(None));

    group1.connect_items_changed(clone!(
            #[strong]
            group1_items_changed,
            move |_, position, removed, added| {
                println!(
                    "group1.connect_items_changed: position {position}, removed {removed}, added {added}"
                );
                assert_matches!(
                    group1_items_changed.replace(Some((position, removed, added))),
                    None
                );
            }
        ));

    // Remove the first item in the group.
    list_store.remove(2);

    assert_matches!(model_items_changed.take(), None);

    assert_eq!(model.n_items(), 3);
    assert_eq!(
        model.item(2).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );

    assert_eq!(group1_items_changed.take(), Some((0, 1, 0)));

    assert_eq!(group1.n_items(), 2);
    assert_matches!(
        group1.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 1.0");
    assert_matches!(
        group1.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 2.0");
    assert_matches!(group1.item(2), None);

    // Re-add the first item in the group.
    list_store.insert(2, &PlaceholderObject::new("group 1 - item 0.1"));

    assert_eq!(model_items_changed.take(), None);

    assert_eq!(model.n_items(), 3);
    assert_eq!(
        model.item(2).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );

    assert_eq!(group1_items_changed.take(), Some((0, 0, 1)));

    assert_eq!(group1.n_items(), 3);
    assert_matches!(
        group1.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 0.1");
    assert_matches!(
        group1.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 1.0");
    assert_matches!(
        group1.item(2).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 2.0");
    assert_matches!(group1.item(3), None);

    // Remove the middle item in the group.
    list_store.remove(3);

    assert_eq!(model_items_changed.take(), None);

    assert_eq!(model.n_items(), 3);
    assert_eq!(
        model.item(2).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );

    assert_eq!(group1_items_changed.take(), Some((1, 1, 0)));

    assert_eq!(group1.n_items(), 2);
    assert_matches!(
        group1.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 0.1");
    assert_matches!(
        group1.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 2.0");
    assert_matches!(group1.item(2), None);

    // Re-add the middle item in the group.
    list_store.insert(3, &PlaceholderObject::new("group 1 - item 1.1"));

    assert_eq!(model_items_changed.take(), None);

    assert_eq!(model.n_items(), 3);
    assert_eq!(
        model.item(2).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );

    assert_eq!(group1_items_changed.take(), Some((1, 0, 1)));

    assert_eq!(group1.n_items(), 3);
    assert_matches!(
        group1.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 0.1");
    assert_matches!(
        group1.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 1.1");
    assert_matches!(
        group1.item(2).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 2.0");
    assert_matches!(group1.item(3), None);

    // Remove the last item in the group.
    list_store.remove(4);

    assert_eq!(model_items_changed.take(), None);
    assert_eq!(model.n_items(), 3);
    assert_eq!(
        model.item(2).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );

    assert_eq!(group1_items_changed.take(), Some((2, 1, 0)));

    assert_eq!(group1.n_items(), 2);
    assert_matches!(
        group1.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 0.1");
    assert_matches!(
        group1.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 1.1");
    assert_matches!(group1.item(2), None);

    // Re-add the last item in the group.
    list_store.insert(4, &PlaceholderObject::new("group 1 - item 2.1"));

    assert_eq!(model_items_changed.take(), None);
    assert_eq!(model.n_items(), 3);
    assert_eq!(
        model.item(2).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );

    assert_eq!(group1_items_changed.take(), Some((2, 0, 1)));

    assert_eq!(group1.n_items(), 3);
    assert_matches!(
        group1.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 0.1");
    assert_matches!(
        group1.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 1.1");
    assert_matches!(
        group1.item(2).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 2.1");
    assert_matches!(group1.item(3), None);

    // Remove group and last singleton.
    list_store.splice(1, 4, &[] as &[glib::Object]);

    assert_eq!(model_items_changed.take(), Some((1, 2, 0)));

    assert_eq!(model.n_items(), 1);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 0.0");
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), None);

    assert_eq!(group1_items_changed.take(), None);
}

#[test]
fn mixed_with_singletons_at_start() {
    let list_store = gio::ListStore::new::<PlaceholderObject>();
    // Group objects that start with "group".
    let model = GroupingListModel::new(|lhs, rhs| {
        lhs.downcast_ref::<PlaceholderObject>()
            .is_some_and(|obj| obj.id().starts_with("group"))
            && rhs
                .downcast_ref::<PlaceholderObject>()
                .is_some_and(|obj| obj.id().starts_with("group"))
    });

    assert_eq!(model.n_items(), 0);
    assert_matches!(model.item(0), None);

    let model_items_changed = Rc::new(RefCell::new(None));

    model.connect_items_changed(clone!(
        #[strong]
        model_items_changed,
        move |_, position, removed, added| {
            println!(
                "model.connect_items_changed: position {position}, removed {removed}, added {added}"
            );
            assert_eq!(
                model_items_changed.replace(Some((position, removed, added))),
                None
            );
        }
    ));

    model.set_model(Some(list_store.clone()));

    assert_eq!(model_items_changed.take(), None);
    assert_eq!(model.n_items(), 0);
    assert_matches!(model.item(0), None);

    // Add some initial items.
    list_store.splice(
        0,
        0,
        &[
            PlaceholderObject::new("singleton 0.0"),
            PlaceholderObject::new("singleton 1.0"),
            PlaceholderObject::new("singleton 2.0"),
            PlaceholderObject::new("group 1 - item 0.0"),
            PlaceholderObject::new("group 1 - item 1.0"),
        ],
    );

    assert_eq!(model_items_changed.take(), Some((0, 0, 4)));

    assert_eq!(model.n_items(), 4);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 0.0");
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 1.0");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.0");
    assert_matches!(
        model.item(3).and_downcast::<GroupingListGroup>(),
        Some(group1)
    );
    assert_matches!(model.item(4), None);

    assert_eq!(group1.n_items(), 2);
    assert_matches!(
        group1.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 0.0");
    assert_matches!(
        group1.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 1.0");
    assert_matches!(group1.item(2), None);

    let group1_items_changed = Rc::new(RefCell::new(None));

    group1.connect_items_changed(clone!(
            #[strong]
            group1_items_changed,
            move |_, position, removed, added| {
                println!(
                    "group1.connect_items_changed: position {position}, removed {removed}, added {added}"
                );
                assert_eq!(
                    group1_items_changed.replace(Some((position, removed, added))),
                    None
                );
            }
        ));

    // Remove the first singleton.
    list_store.remove(0);

    assert_eq!(model_items_changed.take(), Some((0, 1, 0)));

    assert_eq!(model.n_items(), 3);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 1.0");
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.0");
    assert_eq!(
        model.item(2).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );
    assert_matches!(model.item(3), None);

    assert_eq!(group1_items_changed.take(), None);

    // Re-add the first singleton.
    list_store.insert(0, &PlaceholderObject::new("singleton 0.1"));

    assert_eq!(model_items_changed.take(), Some((0, 0, 1)));

    assert_eq!(model.n_items(), 4);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 0.1");
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 1.0");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.0");
    assert_eq!(
        model.item(3).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );
    assert_matches!(model.item(4), None);

    assert_eq!(group1_items_changed.take(), None);

    // Remove the middle singleton.
    list_store.remove(1);

    assert_eq!(model_items_changed.take(), Some((1, 1, 0)));

    assert_eq!(model.n_items(), 3);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 0.1");
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.0");
    assert_eq!(
        model.item(2).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );
    assert_matches!(model.item(3), None);

    assert_eq!(group1_items_changed.take(), None);

    // Re-add the middle singleton.
    list_store.insert(1, &PlaceholderObject::new("singleton 1.1"));

    assert_eq!(model_items_changed.take(), Some((1, 0, 1)));

    assert_eq!(model.n_items(), 4);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 0.1");
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 1.1");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.0");
    assert_eq!(
        model.item(3).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );
    assert_matches!(model.item(4), None);

    assert_eq!(group1_items_changed.take(), None);

    // Remove the last singleton.
    list_store.remove(2);

    assert_eq!(model_items_changed.take(), Some((2, 1, 0)));

    assert_eq!(model.n_items(), 3);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 0.1");
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 1.1");
    assert_eq!(
        model.item(2).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );
    assert_matches!(model.item(3), None);

    assert_eq!(group1_items_changed.take(), None);

    // Re-add the last singleton.
    list_store.insert(2, &PlaceholderObject::new("singleton 2.1"));

    assert_eq!(model_items_changed.take(), Some((2, 0, 1)));

    assert_eq!(model.n_items(), 4);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 0.1");
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 1.1");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.1");
    assert_eq!(
        model.item(3).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );
    assert_matches!(model.item(4), None);

    assert_eq!(group1_items_changed.take(), None);
}

#[test]
fn mixed_with_singletons_in_middle() {
    let list_store = gio::ListStore::new::<PlaceholderObject>();
    // Group objects that start with "group".
    let model = GroupingListModel::new(|lhs, rhs| {
        lhs.downcast_ref::<PlaceholderObject>()
            .is_some_and(|obj| obj.id().starts_with("group"))
            && rhs
                .downcast_ref::<PlaceholderObject>()
                .is_some_and(|obj| obj.id().starts_with("group"))
    });

    assert_eq!(model.n_items(), 0);
    assert_matches!(model.item(0), None);

    let model_items_changed = Rc::new(RefCell::new(None));

    model.connect_items_changed(clone!(
        #[strong]
        model_items_changed,
        move |_, position, removed, added| {
            println!(
                "model.connect_items_changed: position {position}, removed {removed}, added {added}"
            );
            assert_eq!(
                model_items_changed.replace(Some((position, removed, added))),
                None
            );
        }
    ));

    model.set_model(Some(list_store.clone()));

    assert_eq!(model_items_changed.take(), None);
    assert_eq!(model.n_items(), 0);
    assert_matches!(model.item(0), None);

    // Add some initial items.
    list_store.splice(
        0,
        0,
        &[
            PlaceholderObject::new("group 1 - item 0.0"),
            PlaceholderObject::new("group 1 - item 1.0"),
            PlaceholderObject::new("singleton 1.0"),
            PlaceholderObject::new("singleton 2.0"),
            PlaceholderObject::new("singleton 3.0"),
            PlaceholderObject::new("group 2 - item 0.0"),
            PlaceholderObject::new("group 2 - item 1.0"),
        ],
    );

    assert_eq!(model_items_changed.take(), Some((0, 0, 5)));

    assert_eq!(model.n_items(), 5);
    assert_matches!(
        model.item(0).and_downcast::<GroupingListGroup>(),
        Some(group1)
    );
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 1.0");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.0");
    assert_matches!(model.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 3.0");
    assert_matches!(
        model.item(4).and_downcast::<GroupingListGroup>(),
        Some(group2)
    );
    assert_matches!(model.item(5), None);

    assert_eq!(group1.n_items(), 2);
    assert_matches!(
        group1.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 0.0");
    assert_matches!(
        group1.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 1.0");
    assert_matches!(group1.item(2), None);

    assert_eq!(group2.n_items(), 2);
    assert_matches!(
        group2.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 2 - item 0.0");
    assert_matches!(
        group2.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 2 - item 1.0");
    assert_matches!(group2.item(2), None);

    let group1_items_changed = Rc::new(RefCell::new(None));
    let group2_items_changed = Rc::new(RefCell::new(None));

    group1.connect_items_changed(clone!(
        #[strong]
        group1_items_changed,
        move |_, position, removed, added| {
            println!(
                "group1.connect_items_changed: position {position}, removed {removed}, added {added}"
            );
            assert_eq!(
                group1_items_changed.replace(Some((position, removed, added))),
                None
            );
        }
    ));
    group2.connect_items_changed(clone!(
        #[strong]
        group2_items_changed,
        move |_, position, removed, added| {
            println!(
                "group2.connect_items_changed: position {position}, removed {removed}, added {added}"
            );
            assert_matches!(
                group2_items_changed.replace(Some((position, removed, added))),
                None
            );
        }
    ));

    // Remove the first singleton.
    list_store.remove(2);

    assert_eq!(model_items_changed.take(), Some((1, 1, 0)));

    assert_eq!(model.n_items(), 4);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.0");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 3.0");
    assert_eq!(
        model.item(3).and_downcast::<GroupingListGroup>().unwrap(),
        group2
    );
    assert_matches!(model.item(4), None);

    assert_matches!(group1_items_changed.take(), None);
    assert_matches!(group2_items_changed.take(), None);

    // Re-add the first singleton.
    list_store.insert(2, &PlaceholderObject::new("singleton 1.1"));

    assert_eq!(model_items_changed.take(), Some((1, 0, 1)));

    assert_eq!(model.n_items(), 5);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 1.1");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.0");
    assert_matches!(model.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 3.0");
    assert_eq!(
        model.item(4).and_downcast::<GroupingListGroup>().unwrap(),
        group2
    );
    assert_matches!(model.item(5), None);

    assert_matches!(group1_items_changed.take(), None);
    assert_matches!(group2_items_changed.take(), None);

    // Remove the middle singleton.
    list_store.remove(3);

    assert_eq!(model_items_changed.take(), Some((2, 1, 0)));

    assert_eq!(model.n_items(), 4);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 1.1");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 3.0");
    assert_eq!(
        model.item(3).and_downcast::<GroupingListGroup>().unwrap(),
        group2
    );
    assert_matches!(model.item(4), None);

    assert_matches!(group1_items_changed.take(), None);
    assert_matches!(group2_items_changed.take(), None);

    // Re-add the middle singleton.
    list_store.insert(3, &PlaceholderObject::new("singleton 2.1"));

    assert_eq!(model_items_changed.take(), Some((2, 0, 1)));

    assert_eq!(model.n_items(), 5);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 1.1");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.1");
    assert_matches!(model.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 3.0");
    assert_eq!(
        model.item(4).and_downcast::<GroupingListGroup>().unwrap(),
        group2
    );
    assert_matches!(model.item(5), None);

    assert_matches!(group1_items_changed.take(), None);
    assert_matches!(group2_items_changed.take(), None);

    // Remove the last singleton.
    list_store.remove(4);

    assert_eq!(model_items_changed.take(), Some((3, 1, 0)));

    assert_eq!(model.n_items(), 4);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 1.1");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.1");
    assert_eq!(
        model.item(3).and_downcast::<GroupingListGroup>().unwrap(),
        group2
    );
    assert_matches!(model.item(4), None);

    assert_matches!(group1_items_changed.take(), None);
    assert_matches!(group2_items_changed.take(), None);

    // Re-add the last singleton.
    list_store.insert(4, &PlaceholderObject::new("singleton 3.1"));

    assert_eq!(model_items_changed.take(), Some((3, 0, 1)));

    assert_eq!(model.n_items(), 5);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 1.1");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.1");
    assert_matches!(model.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 3.1");
    assert_eq!(
        model.item(4).and_downcast::<GroupingListGroup>().unwrap(),
        group2
    );
    assert_matches!(model.item(5), None);

    assert_matches!(group1_items_changed.take(), None);
    assert_matches!(group2_items_changed.take(), None);
}

#[test]
fn mixed_with_singletons_at_end() {
    let list_store = gio::ListStore::new::<PlaceholderObject>();
    // Group objects that start with "group".
    let model = GroupingListModel::new(|lhs, rhs| {
        lhs.downcast_ref::<PlaceholderObject>()
            .is_some_and(|obj| obj.id().starts_with("group"))
            && rhs
                .downcast_ref::<PlaceholderObject>()
                .is_some_and(|obj| obj.id().starts_with("group"))
    });

    assert_eq!(model.n_items(), 0);
    assert_matches!(model.item(0), None);

    let model_items_changed = Rc::new(RefCell::new(None));

    model.connect_items_changed(clone!(
        #[strong]
        model_items_changed,
        move |_, position, removed, added| {
            println!(
                "model.connect_items_changed: position {position}, removed {removed}, added {added}"
            );
            assert_matches!(
                model_items_changed.replace(Some((position, removed, added))),
                None
            );
        }
    ));

    model.set_model(Some(list_store.clone()));

    assert_matches!(model_items_changed.take(), None);
    assert_eq!(model.n_items(), 0);
    assert_matches!(model.item(0), None);

    // Add some initial items.
    list_store.splice(
        0,
        0,
        &[
            PlaceholderObject::new("group 1 - item 0.0"),
            PlaceholderObject::new("group 1 - item 1.0"),
            PlaceholderObject::new("singleton 1.0"),
            PlaceholderObject::new("singleton 2.0"),
            PlaceholderObject::new("singleton 3.0"),
        ],
    );

    assert_matches!(model_items_changed.take(), Some((0, 0, 4)));

    assert_eq!(model.n_items(), 4);
    assert_matches!(
        model.item(0).and_downcast::<GroupingListGroup>(),
        Some(group1)
    );
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 1.0");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.0");
    assert_matches!(model.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 3.0");
    assert_matches!(model.item(4), None);

    assert_eq!(group1.n_items(), 2);
    assert_matches!(
        group1.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 0.0");
    assert_matches!(
        group1.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 1.0");
    assert_matches!(group1.item(2), None);

    let group1_items_changed = Rc::new(RefCell::new(None));

    group1.connect_items_changed(clone!(
            #[strong]
            group1_items_changed,
            move |_, position, removed, added| {
                println!(
                    "group1.connect_items_changed: position {position}, removed {removed}, added {added}"
                );
                assert_matches!(
                    group1_items_changed.replace(Some((position, removed, added))),
                    None
                );
            }
        ));

    // Remove the first singleton.
    list_store.remove(2);

    assert_eq!(model_items_changed.take(), Some((1, 1, 0)));

    assert_eq!(model.n_items(), 3);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.0");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 3.0");
    assert_matches!(model.item(3), None);

    assert_matches!(group1_items_changed.take(), None);

    // Re-add the first singleton.
    list_store.insert(2, &PlaceholderObject::new("singleton 1.1"));

    assert_eq!(model_items_changed.take(), Some((1, 0, 1)));

    assert_eq!(model.n_items(), 4);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 1.1");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.0");
    assert_matches!(model.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 3.0");
    assert_matches!(model.item(4), None);

    assert_matches!(group1_items_changed.take(), None);

    // Remove the middle singleton.
    list_store.remove(3);

    assert_eq!(model_items_changed.take(), Some((2, 1, 0)));

    assert_eq!(model.n_items(), 3);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 1.1");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 3.0");
    assert_matches!(model.item(3), None);

    assert_matches!(group1_items_changed.take(), None);

    // Re-add the middle singleton.
    list_store.insert(3, &PlaceholderObject::new("singleton 2.1"));

    assert_eq!(model_items_changed.take(), Some((2, 0, 1)));

    assert_eq!(model.n_items(), 4);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 1.1");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.1");
    assert_matches!(model.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 3.0");
    assert_matches!(model.item(4), None);

    assert_eq!(group1_items_changed.take(), None);

    // Remove the last singleton.
    list_store.remove(4);

    assert_eq!(model_items_changed.take(), Some((3, 1, 0)));

    assert_eq!(model.n_items(), 3);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 1.1");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.1");
    assert_matches!(model.item(3), None);

    assert_eq!(group1_items_changed.take(), None);

    // Re-add the last singleton.
    list_store.insert(4, &PlaceholderObject::new("singleton 3.1"));

    assert_eq!(model_items_changed.take(), Some((3, 0, 1)));

    assert_eq!(model.n_items(), 4);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 1.1");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.1");
    assert_matches!(model.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 3.1");
    assert_matches!(model.item(4), None);

    assert_eq!(group1_items_changed.take(), None);
}

#[test]
fn merge_into_group() {
    let list_store = gio::ListStore::new::<PlaceholderObject>();
    // Group objects that start with "group".
    let model = GroupingListModel::new(|lhs, rhs| {
        lhs.downcast_ref::<PlaceholderObject>()
            .is_some_and(|obj| obj.id().starts_with("group"))
            && rhs
                .downcast_ref::<PlaceholderObject>()
                .is_some_and(|obj| obj.id().starts_with("group"))
    });

    assert_eq!(model.n_items(), 0);
    assert_matches!(model.item(0), None);

    let model_items_changed = Rc::new(RefCell::new(None));

    let model_handler = model.connect_items_changed(clone!(
        #[strong]
        model_items_changed,
        move |_, position, removed, added| {
            println!(
                "model.connect_items_changed: position {position}, removed {removed}, added {added}"
            );
            assert_matches!(
                model_items_changed.replace(Some((position, removed, added))),
                None
            );
        }
    ));

    model.set_model(Some(list_store.clone()));

    assert_eq!(model_items_changed.take(), None);
    assert_eq!(model.n_items(), 0);
    assert_matches!(model.item(0), None);

    // Add some initial items.
    list_store.splice(
        0,
        0,
        &[
            PlaceholderObject::new("singleton 0.0"),
            PlaceholderObject::new("group 1 - item 1.0"),
            PlaceholderObject::new("singleton 2.0"),
        ],
    );

    assert_eq!(model_items_changed.take(), Some((0, 0, 3)));

    assert_eq!(model.n_items(), 3);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 0.0");
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "group 1 - item 1.0");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.0");
    assert_matches!(model.item(3), None);

    // Add one item to group end.
    list_store.insert(2, &PlaceholderObject::new("group 1 - item 4.0"));

    assert_eq!(model_items_changed.take(), Some((1, 1, 1)));

    assert_eq!(model.n_items(), 3);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 0.0");
    assert_matches!(
        model.item(1).and_downcast::<GroupingListGroup>(),
        Some(group1)
    );
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.0");
    assert_matches!(model.item(3), None);

    assert_eq!(group1.n_items(), 2);
    assert_matches!(
        group1.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 1.0");
    assert_matches!(
        group1.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 4.0");
    assert_matches!(group1.item(2), None);

    let group1_items_changed = Rc::new(RefCell::new(None));

    group1.connect_items_changed(clone!(
        #[strong]
        group1_items_changed,
        move |_, position, removed, added| {
            println!(
                "group1.connect_items_changed: position {position}, removed {removed}, added {added}"
            );
            assert_matches!(
                group1_items_changed.replace(Some((position, removed, added))),
                None
            );
        }
    ));

    // Add one item to group start.
    list_store.insert(1, &PlaceholderObject::new("group 1 - item 0.0"));

    assert_eq!(model_items_changed.take(), None);
    assert_eq!(model.n_items(), 3);
    assert_eq!(
        model.item(1).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );

    assert_eq!(group1_items_changed.take(), Some((0, 0, 1)));

    assert_eq!(group1.n_items(), 3);
    assert_matches!(
        group1.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 0.0");
    assert_matches!(
        group1.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 1.0");
    assert_matches!(
        group1.item(2).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 4.0");
    assert_matches!(group1.item(3), None);

    // Add two more items to group.
    list_store.splice(
        3,
        0,
        &[
            PlaceholderObject::new("group 1 - item 2.0"),
            PlaceholderObject::new("group 1 - item 3.0"),
        ],
    );

    assert_eq!(model_items_changed.take(), None);
    assert_eq!(model.n_items(), 3);
    assert_eq!(
        model.item(1).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );

    assert_eq!(group1_items_changed.take(), Some((2, 0, 2)));

    assert_eq!(group1.n_items(), 5);
    assert_matches!(
        group1.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 0.0");
    assert_matches!(
        group1.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 1.0");
    assert_matches!(
        group1.item(2).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 2.0");
    assert_matches!(
        group1.item(3).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 3.0");
    assert_matches!(
        group1.item(4).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 4.0");
    assert_matches!(group1.item(5), None);

    // Add item for group and other singleton at the end.
    list_store.splice(
        7,
        0,
        &[
            PlaceholderObject::new("group 1 - item 5.0"),
            PlaceholderObject::new("singleton 3.0"),
        ],
    );

    assert_eq!(model_items_changed.take(), Some((3, 0, 2)));

    assert_eq!(model.n_items(), 5);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 0.0");
    assert_eq!(
        model.item(1).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.0");
    assert_matches!(model.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "group 1 - item 5.0");
    assert_matches!(model.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 3.0");
    assert_matches!(model.item(5), None);

    assert_matches!(group1_items_changed.take(), None);

    // Remove second singleton.
    list_store.remove(6);

    assert_eq!(model_items_changed.take(), Some((2, 2, 0)));

    assert_eq!(model.n_items(), 3);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 0.0");
    assert_eq!(
        model.item(1).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 3.0");
    assert_matches!(model.item(3), None);

    assert_eq!(group1_items_changed.take(), Some((5, 0, 1)));

    assert_eq!(group1.n_items(), 6);
    assert_matches!(
        group1.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 0.0");
    assert_matches!(
        group1.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 1.0");
    assert_matches!(
        group1.item(2).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 2.0");
    assert_matches!(
        group1.item(3).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 3.0");
    assert_matches!(
        group1.item(4).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 4.0");
    assert_matches!(
        group1.item(5).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 5.0");
    assert_matches!(group1.item(6), None);

    // Add two items for group at the end.
    list_store.splice(
        8,
        0,
        &[
            PlaceholderObject::new("group 1 - item 6.0"),
            PlaceholderObject::new("group 1 - item 7.0"),
        ],
    );

    assert_eq!(model_items_changed.take(), Some((3, 0, 1)));

    assert_eq!(model.n_items(), 4);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 0.0");
    assert_eq!(
        model.item(1).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 3.0");
    assert_matches!(
        model.item(3).and_downcast::<GroupingListGroup>(),
        Some(group2)
    );
    assert_matches!(model.item(4), None);

    assert_eq!(group1_items_changed.take(), None);

    assert_eq!(group2.n_items(), 2);
    assert_matches!(
        group2.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 6.0");
    assert_matches!(
        group2.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 7.0");
    assert_matches!(group2.item(2), None);

    let group2_items_changed = Rc::new(RefCell::new(None));

    group2.connect_items_changed(clone!(
        #[strong]
        group2_items_changed,
        move |_, position, removed, added| {
            println!(
                "group2.connect_items_changed: position {position}, removed {removed}, added {added}"
            );
            assert_matches!(
                group2_items_changed.replace(Some((position, removed, added))),
                None
            );
        }
    ));

    // Remove the last singleton.
    list_store.remove(7);

    assert_eq!(model_items_changed.take(), Some((2, 2, 0)));

    assert_eq!(model.n_items(), 2);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 0.0");
    assert_eq!(
        model.item(1).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );
    assert_matches!(model.item(2), None);

    assert_eq!(group1_items_changed.take(), Some((6, 0, 2)));

    assert_eq!(group1.n_items(), 8);
    assert_matches!(
        group1.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 0.0");
    assert_matches!(
        group1.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 1.0");
    assert_matches!(
        group1.item(2).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 2.0");
    assert_matches!(
        group1.item(3).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 3.0");
    assert_matches!(
        group1.item(4).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 4.0");
    assert_matches!(
        group1.item(5).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 5.0");
    assert_matches!(
        group1.item(6).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 6.0");
    assert_matches!(
        group1.item(7).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 7.0");
    assert_matches!(group1.item(8), None);

    assert_eq!(group2_items_changed.take(), None);

    // Add two items for another group, separated by a singleton.
    list_store.splice(
        1,
        0,
        &[
            PlaceholderObject::new("group 0 - item 0.0"),
            PlaceholderObject::new("singleton 1.1"),
            PlaceholderObject::new("group 0 - item 1.0"),
            PlaceholderObject::new("singleton 2.1"),
        ],
    );
    assert_eq!(model_items_changed.take(), Some((1, 0, 4)));

    assert_eq!(model.n_items(), 6);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 0.0");
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "group 0 - item 0.0");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 1.1");
    assert_matches!(model.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "group 0 - item 1.0");
    assert_matches!(model.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.1");
    assert_eq!(
        model.item(5).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );
    assert_matches!(model.item(6), None);

    assert_matches!(group1_items_changed.take(), None);

    // We will receive 2 signals for one action.
    model.disconnect(model_handler);
    let model_items_changed = Rc::new(RefCell::new(Vec::new()));
    model.connect_items_changed(clone!(
        #[strong]
        model_items_changed,
        move |_, position, removed, added| {
            println!(
                "model.connect_items_changed: position {position}, removed {removed}, added {added}"
            );
            model_items_changed
                .borrow_mut()
                .push((position, removed, added));
        }
    ));

    // Remove the second singleton.
    list_store.remove(2);

    assert_eq!(model_items_changed.take(), &[(2, 2, 0), (1, 1, 1)]);

    assert_eq!(model.n_items(), 4);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 0.0");
    assert_matches!(
        model.item(1).and_downcast::<GroupingListGroup>(),
        Some(group0)
    );
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.1");
    assert_eq!(
        model.item(3).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );
    assert_matches!(model.item(4), None);

    assert_eq!(group1_items_changed.take(), None);

    assert_eq!(group0.n_items(), 2);
    assert_matches!(
        group0.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 0 - item 0.0");
    assert_matches!(
        group0.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 0 - item 1.0");
    assert_matches!(group0.item(2), None);
}

#[test]
fn split_group() {
    let list_store = gio::ListStore::new::<PlaceholderObject>();
    // Group objects that start with "group".
    let model = GroupingListModel::new(|lhs, rhs| {
        lhs.downcast_ref::<PlaceholderObject>()
            .is_some_and(|obj| obj.id().starts_with("group"))
            && rhs
                .downcast_ref::<PlaceholderObject>()
                .is_some_and(|obj| obj.id().starts_with("group"))
    });

    assert_eq!(model.n_items(), 0);
    assert_matches!(model.item(0), None);

    let model_items_changed = Rc::new(RefCell::new(Vec::new()));

    model.connect_items_changed(clone!(
        #[strong]
        model_items_changed,
        move |_, position, removed, added| {
            println!(
                "model.connect_items_changed: position {position}, removed {removed}, added {added}"
            );
            model_items_changed
                .borrow_mut()
                .push((position, removed, added));
        }
    ));

    model.set_model(Some(list_store.clone()));

    assert_eq!(model_items_changed.take(), &[]);
    assert_eq!(model.n_items(), 0);
    assert_matches!(model.item(0), None);

    // Add some initial items.
    list_store.splice(
        0,
        0,
        &[
            PlaceholderObject::new("singleton 0.0"),
            PlaceholderObject::new("group 1 - item 2.0"),
            PlaceholderObject::new("group 2 - item 0.0"),
            PlaceholderObject::new("singleton 2.0"),
        ],
    );

    assert_eq!(model_items_changed.take(), &[(0, 0, 3)]);

    assert_eq!(model.n_items(), 3);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 0.0");
    assert_matches!(
        model.item(1).and_downcast::<GroupingListGroup>(),
        Some(group1)
    );
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.0");
    assert_matches!(model.item(3), None);

    assert_eq!(group1.n_items(), 2);
    assert_matches!(
        group1.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 2.0");
    assert_matches!(
        group1.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 2 - item 0.0");
    assert_matches!(group1.item(2), None);

    // Add singleton to split group.
    list_store.insert(2, &PlaceholderObject::new("singleton 1.0"));

    assert_eq!(model_items_changed.take(), &[(2, 0, 2), (1, 1, 1)]);

    assert_eq!(model.n_items(), 5);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 0.0");
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "group 1 - item 2.0");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 1.0");
    assert_matches!(model.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "group 2 - item 0.0");
    assert_matches!(model.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.0");
    assert_matches!(model.item(5), None);

    // Add two items to first group.
    list_store.splice(
        1,
        0,
        &[
            PlaceholderObject::new("group 1 - item 0.0"),
            PlaceholderObject::new("group 1 - item 1.0"),
        ],
    );

    assert_eq!(model_items_changed.take(), &[(1, 1, 1)]);

    assert_eq!(model.n_items(), 5);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 0.0");
    assert_matches!(
        model.item(1).and_downcast::<GroupingListGroup>(),
        Some(group1)
    );
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 1.0");
    assert_matches!(model.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "group 2 - item 0.0");
    assert_matches!(model.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.0");
    assert_matches!(model.item(5), None);

    assert_eq!(group1.n_items(), 3);
    assert_matches!(
        group1.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 0.0");
    assert_matches!(
        group1.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 1.0");
    assert_matches!(
        group1.item(2).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 2.0");
    assert_matches!(group1.item(3), None);

    let group1_items_changed = Rc::new(RefCell::new(None));
    group1.connect_items_changed(clone!(
        #[strong]
        group1_items_changed,
        move |_, position, removed, added| {
            println!(
                "group1.connect_items_changed: position {position}, removed {removed}, added {added}"
            );
            assert_eq!(group1_items_changed.replace(Some((position, removed, added))),None);
        }
    ));

    // Add singleton to split first group.
    list_store.insert(3, &PlaceholderObject::new("singleton 0.9"));

    assert_eq!(model_items_changed.take(), &[(2, 0, 2)]);

    assert_eq!(model.n_items(), 7);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 0.0");
    assert_eq!(
        model.item(1).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 0.9");
    assert_matches!(model.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "group 1 - item 2.0");
    assert_matches!(model.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 1.0");
    assert_matches!(model.item(5).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "group 2 - item 0.0");
    assert_matches!(model.item(6).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.0");
    assert_matches!(model.item(7), None);

    assert_eq!(group1_items_changed.take(), Some((2, 1, 0)));

    assert_eq!(group1.n_items(), 2);
    assert_matches!(
        group1.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 0.0");
    assert_matches!(
        group1.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 1.0");
    assert_matches!(group1.item(2), None);

    // Add two items to second group.
    list_store.splice(
        7,
        0,
        &[
            PlaceholderObject::new("group 2 - item 1.0"),
            PlaceholderObject::new("group 2 - item 2.0"),
        ],
    );

    assert_eq!(model_items_changed.take(), &[(5, 1, 1)]);

    assert_eq!(model.n_items(), 7);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 0.0");
    assert_eq!(
        model.item(1).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 0.9");
    assert_matches!(model.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "group 1 - item 2.0");
    assert_matches!(model.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 1.0");
    assert_matches!(
        model.item(5).and_downcast::<GroupingListGroup>(),
        Some(group2)
    );
    assert_matches!(model.item(6).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.0");
    assert_matches!(model.item(7), None);

    assert_eq!(group1_items_changed.take(), None);

    assert_eq!(group2.n_items(), 3);
    assert_matches!(
        group2.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 2 - item 0.0");
    assert_matches!(
        group2.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 2 - item 1.0");
    assert_matches!(
        group2.item(2).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 2 - item 2.0");
    assert_matches!(group2.item(3), None);

    // Add singleton to split second group.
    list_store.insert(7, &PlaceholderObject::new("singleton 1.9"));

    assert_eq!(model_items_changed.take(), &[(6, 0, 2), (5, 1, 1)]);

    assert_eq!(model.n_items(), 9);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 0.0");
    assert_eq!(
        model.item(1).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 0.9");
    assert_matches!(model.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "group 1 - item 2.0");
    assert_matches!(model.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 1.0");
    assert_matches!(model.item(5).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "group 2 - item 0.0");
    assert_matches!(model.item(6).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 1.9");
    assert_matches!(
        model.item(7).and_downcast::<GroupingListGroup>(),
        Some(group2)
    );
    assert_matches!(model.item(8).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.0");
    assert_matches!(model.item(9), None);

    assert_eq!(group1_items_changed.take(), None);

    assert_eq!(group2.n_items(), 2);
    assert_matches!(
        group2.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 2 - item 1.0");
    assert_matches!(
        group2.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 2 - item 2.0");
    assert_matches!(group2.item(2), None);

    // Remove singletons to merge groups.
    list_store.splice(3, 5, &[] as &[glib::Object]);

    assert_eq!(model_items_changed.take(), &[(2, 6, 0)]);

    assert_eq!(model.n_items(), 3);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 0.0");
    assert_eq!(
        model.item(1).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.0");
    assert_matches!(model.item(3), None);

    assert_eq!(group1_items_changed.take(), Some((2, 0, 2)));

    assert_eq!(group1.n_items(), 4);
    assert_matches!(
        group1.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 0.0");
    assert_matches!(
        group1.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 1.0");
    assert_matches!(
        group1.item(2).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 2 - item 1.0");
    assert_matches!(
        group1.item(3).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 2 - item 2.0");
    assert_matches!(group1.item(4), None);

    // Add singleton to re-split group.
    list_store.insert(3, &PlaceholderObject::new("singleton 1.1"));

    assert_eq!(model_items_changed.take(), &[(2, 0, 2)]);

    assert_eq!(model.n_items(), 5);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 0.0");
    assert_eq!(
        model.item(1).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 1.1");
    assert_matches!(
        model.item(3).and_downcast::<GroupingListGroup>(),
        Some(group2)
    );
    assert_matches!(model.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.0");
    assert_matches!(model.item(5), None);

    assert_eq!(group1_items_changed.take(), Some((2, 2, 0)));

    assert_eq!(group1.n_items(), 2);
    assert_matches!(
        group1.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 0.0");
    assert_matches!(
        group1.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 1.0");
    assert_matches!(group1.item(2), None);

    assert_eq!(group2.n_items(), 2);
    assert_matches!(
        group2.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 2 - item 1.0");
    assert_matches!(
        group2.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 2 - item 2.0");
    assert_matches!(group2.item(2), None);
}

#[test]
fn replace() {
    let list_store = gio::ListStore::new::<PlaceholderObject>();
    // Group objects that start with "group".
    let model = GroupingListModel::new(|lhs, rhs| {
        lhs.downcast_ref::<PlaceholderObject>()
            .is_some_and(|obj| obj.id().starts_with("group"))
            && rhs
                .downcast_ref::<PlaceholderObject>()
                .is_some_and(|obj| obj.id().starts_with("group"))
    });

    assert_eq!(model.n_items(), 0);
    assert_matches!(model.item(0), None);

    let model_items_changed = Rc::new(RefCell::new(Vec::new()));
    model.connect_items_changed(clone!(
        #[strong]
        model_items_changed,
        move |_, position, removed, added| {
            println!(
                "model.connect_items_changed: position {position}, removed {removed}, added {added}"
            );
            model_items_changed
                .borrow_mut()
                .push((position, removed, added));
        }
    ));

    model.set_model(Some(list_store.clone()));

    assert_eq!(model_items_changed.take(), &[]);
    assert_eq!(model.n_items(), 0);
    assert_matches!(model.item(0), None);

    // Add some initial items.
    list_store.splice(
        0,
        0,
        &[
            PlaceholderObject::new("singleton 0.0"),
            PlaceholderObject::new("singleton 1.0"),
            PlaceholderObject::new("group 1 - item 0.0"),
            PlaceholderObject::new("group 1 - item 1.0"),
            PlaceholderObject::new("singleton 2.0"),
            PlaceholderObject::new("singleton 3.0"),
        ],
    );

    assert_eq!(model_items_changed.take(), &[(0, 0, 5)]);

    assert_eq!(model.n_items(), 5);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 0.0");
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 1.0");
    assert_matches!(
        model.item(2).and_downcast::<GroupingListGroup>(),
        Some(group1)
    );
    assert_matches!(model.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.0");
    assert_matches!(model.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 3.0");
    assert_matches!(model.item(5), None);

    assert_eq!(group1.n_items(), 2);
    assert_matches!(
        group1.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 0.0");
    assert_matches!(
        group1.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 1 - item 1.0");
    assert_matches!(group1.item(2), None);

    // Replace first singleton by other singleton.
    list_store.splice(0, 1, &[PlaceholderObject::new("singleton 0.1")]);

    assert_eq!(model_items_changed.take(), &[(0, 1, 1)]);

    assert_eq!(model.n_items(), 5);
    assert_matches!(model.item(0).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 0.1");
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 1.0");
    assert_eq!(
        model.item(2).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );
    assert_matches!(model.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.0");
    assert_matches!(model.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 3.0");
    assert_matches!(model.item(5), None);

    assert_eq!(group1.n_items(), 2);

    // Replace first singleton by group.
    list_store.splice(
        0,
        1,
        &[
            PlaceholderObject::new("group 0 - item 0.0"),
            PlaceholderObject::new("group 0 - item 1.0"),
            PlaceholderObject::new("group 0 - item 2.0"),
            PlaceholderObject::new("group 0 - item 3.0"),
        ],
    );

    assert_eq!(model_items_changed.take(), &[(0, 1, 1)]);

    assert_eq!(model.n_items(), 5);
    assert_matches!(
        model.item(0).and_downcast::<GroupingListGroup>(),
        Some(group0)
    );
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 1.0");
    assert_eq!(
        model.item(2).and_downcast::<GroupingListGroup>().unwrap(),
        group1
    );
    assert_matches!(model.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.0");
    assert_matches!(model.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 3.0");
    assert_matches!(model.item(5), None);

    assert_eq!(group0.n_items(), 4);
    assert_matches!(
        group0.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 0 - item 0.0");
    assert_matches!(
        group0.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 0 - item 1.0");
    assert_matches!(
        group0.item(2).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 0 - item 2.0");
    assert_matches!(
        group0.item(3).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 0 - item 3.0");
    assert_matches!(group0.item(4), None);

    assert_eq!(group1.n_items(), 2);

    let group0_items_changed = Rc::new(RefCell::new(None));
    group0.connect_items_changed(clone!(
        #[strong]
        group0_items_changed,
        move |_, position, removed, added| {
            println!(
                "group0.connect_items_changed: position {position}, removed {removed}, added {added}"
            );
            assert_eq!(group0_items_changed.replace(Some((position, removed, added))), None);
        }
    ));

    // Replace second group by other group.
    list_store.splice(
        5,
        2,
        &[
            PlaceholderObject::new("group 2 - item 0.0"),
            PlaceholderObject::new("group 2 - item 1.0"),
        ],
    );

    assert_eq!(model_items_changed.take(), &[(2, 1, 1)]);

    assert_eq!(model.n_items(), 5);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group0
    );
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 1.0");
    assert_matches!(
        model.item(2).and_downcast::<GroupingListGroup>(),
        Some(group2)
    );
    assert_ne!(group1, group2);
    assert_matches!(model.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.0");
    assert_matches!(model.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 3.0");
    assert_matches!(model.item(5), None);

    assert_eq!(group0_items_changed.take(), None);
    assert_eq!(group0.n_items(), 4);

    assert_eq!(group2.n_items(), 2);
    assert_matches!(
        group2.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 2 - item 0.0");
    assert_matches!(
        group2.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 2 - item 1.0");
    assert_matches!(group2.item(2), None);

    // Replace second group by singleton.
    list_store.splice(5, 2, &[PlaceholderObject::new("singleton 1.9")]);

    assert_eq!(model_items_changed.take(), &[(2, 1, 1)]);

    assert_eq!(model.n_items(), 5);
    assert_eq!(
        model.item(0).and_downcast::<GroupingListGroup>().unwrap(),
        group0
    );
    assert_matches!(model.item(1).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 1.0");
    assert_matches!(model.item(2).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 1.9");
    assert_matches!(model.item(3).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 2.0");
    assert_matches!(model.item(4).and_downcast::<PlaceholderObject>(), Some(obj));
    assert_eq!(obj.id(), "singleton 3.0");
    assert_matches!(model.item(5), None);

    assert_eq!(group0_items_changed.take(), None);
    assert_eq!(group0.n_items(), 4);

    // Replace items in group.
    list_store.splice(
        2,
        2,
        &[
            PlaceholderObject::new("group 0 - item 2.1"),
            PlaceholderObject::new("group 0 - item 3.1"),
            PlaceholderObject::new("group 0 - item 4.1"),
        ],
    );

    assert_eq!(model_items_changed.take(), &[]);
    assert_eq!(model.n_items(), 5);

    assert_eq!(group0_items_changed.take(), Some((2, 2, 3)));

    assert_eq!(group0.n_items(), 5);
    assert_matches!(
        group0.item(0).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 0 - item 0.0");
    assert_matches!(
        group0.item(1).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 0 - item 1.0");
    assert_matches!(
        group0.item(2).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 0 - item 2.1");
    assert_matches!(
        group0.item(3).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 0 - item 3.1");
    assert_matches!(
        group0.item(4).and_downcast::<PlaceholderObject>(),
        Some(obj)
    );
    assert_eq!(obj.id(), "group 0 - item 4.1");
    assert_matches!(group0.item(5), None);
}
