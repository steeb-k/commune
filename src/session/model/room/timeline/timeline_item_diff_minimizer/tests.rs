#![cfg(test)]
#![allow(clippy::too_many_lines)]

use std::cell::RefCell;

use assert_matches2::assert_matches;
use gtk::{glib, prelude::*, subclass::prelude::*};
use matrix_sdk_ui::eyeball_im::Vector;

use super::*;
use crate::session::model::TimelineItemImpl;

/// Timeline item store to test `TimelineItemDiffMinimizer`.
#[derive(Debug, Clone, Default)]
struct TestTimelineItemStore {
    /// The items in the store.
    items: RefCell<Vec<TestTimelineItem>>,
}

impl TestTimelineItemStore {
    /// Set `processed` to false for all items.
    fn reset_processed_items(&self) {
        for item in &*self.items.borrow() {
            item.downcast_ref::<TestTimelineItem>()
                .expect("TestTimelineItemStore only receives TestTimelineItem")
                .set_processed(false);
        }
    }
}

impl TimelineItemStore for TestTimelineItemStore {
    type Item = TestTimelineItem;
    type Data = TestTimelineItemData;

    fn items(&self) -> Vec<TestTimelineItem> {
        self.items.borrow().clone()
    }

    fn create_item(&self, data: &Self::Data) -> TestTimelineItem {
        println!("create_item: {data:?}");
        TestTimelineItem::new(data)
    }

    fn update_item(&self, item: &TestTimelineItem, data: &Self::Data) {
        println!("update_item: {item:?} {data:?}");
        item.set_version(data.version);
    }

    fn apply_item_diff_list(&self, item_diff_list: Vec<TimelineItemDiff<TestTimelineItem>>) {
        for item_diff in item_diff_list {
            match item_diff {
                TimelineItemDiff::Splice(splice_diff) => {
                    let mut items = self.items.borrow_mut();
                    let pos = splice_diff.pos as usize;
                    let n_removals = splice_diff.n_removals as usize;
                    let n_additions = splice_diff.additions.len();

                    items.splice(pos..pos + n_removals, splice_diff.additions);

                    // Set all the new additions and the first one after the current batch as
                    // processed.
                    for item in items.iter().skip(pos).take(n_additions + 1) {
                        item.set_processed(true);
                    }
                }
                TimelineItemDiff::Update(update_diff) => {
                    let pos = update_diff.pos as usize;
                    let n_items = update_diff.n_items as usize;
                    let items = &*self.items.borrow();
                    let len = items.len();
                    assert!(
                        len >= pos + n_items,
                        "len = {len}; pos = {pos}; n_items = {n_items}"
                    );

                    // Mark them all and the first one after the current batch as processed.
                    for item in items.iter().skip(pos).take(n_items + 1) {
                        item.set_processed(true);
                    }
                }
            }
        }
    }
}

/// Timeline item data to test `TimelineItemDiffMinimizer`.
#[derive(Debug, Clone, Copy)]
struct TestTimelineItemData {
    timeline_id: &'static str,
    version: u8,
}

impl TimelineItemData for TestTimelineItemData {
    fn timeline_id(&self) -> &str {
        self.timeline_id
    }
}

mod imp {
    use std::cell::Cell;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::TestTimelineItem)]
    pub struct TestTimelineItem {
        /// The version of the item.
        #[property(get, set, construct)]
        version: Cell<u8>,
        /// Whether the item was processed in `apply_item_diff_list`.
        #[property(get, set)]
        processed: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TestTimelineItem {
        const NAME: &'static str = "TestTimelineItem";
        type Type = super::TestTimelineItem;
        type ParentType = TimelineItem;
    }

    #[glib::derived_properties]
    impl ObjectImpl for TestTimelineItem {}

    impl TimelineItemImpl for TestTimelineItem {}
}

glib::wrapper! {
    /// Timeline item to test `TimelineItemDiffMinimizer`.
    pub struct TestTimelineItem(ObjectSubclass<imp::TestTimelineItem>) @extends TimelineItem;
}

impl TestTimelineItem {
    fn new(data: &TestTimelineItemData) -> Self {
        glib::Object::builder()
            .property("timeline-id", data.timeline_id)
            .property("version", data.version)
            .build()
    }
}

/// Test diff lists for each `VectorDiff` variant.
///
/// Although we will not use the minimizer for a single `VectorDiff`, this tests
/// at least the correctness of the code.
#[test]
fn process_single_vector_diff() {
    let store = TestTimelineItemStore::default();

    // Append.
    let diff = VectorDiff::Append {
        values: Vector::from([
            TestTimelineItemData {
                timeline_id: "a",
                version: 0,
            },
            TestTimelineItemData {
                timeline_id: "b",
                version: 0,
            },
            TestTimelineItemData {
                timeline_id: "c",
                version: 0,
            },
        ]),
    };
    assert!(store.can_minimize_diff_list(&[diff.clone(), diff.clone()]));
    store.minimize_diff_list(vec![diff]);

    let items = store.items();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0].timeline_id(), "a");
    assert_eq!(items[0].version(), 0);
    assert!(items[0].processed());
    assert_eq!(items[1].timeline_id(), "b");
    assert_eq!(items[1].version(), 0);
    assert!(items[1].processed());
    assert_eq!(items[2].timeline_id(), "c");
    assert_eq!(items[2].version(), 0);
    assert!(items[2].processed());

    store.reset_processed_items();

    // Pop front.
    let diff = VectorDiff::PopFront;
    assert!(store.can_minimize_diff_list(&[diff.clone(), diff.clone()]));
    store.minimize_diff_list(vec![diff]);

    let items = store.items();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].timeline_id(), "b");
    assert_eq!(items[0].version(), 0);
    assert!(items[0].processed());
    assert_eq!(items[1].timeline_id(), "c");
    assert_eq!(items[1].version(), 0);
    assert!(!items[1].processed());

    store.reset_processed_items();

    // Pop back.
    let diff = VectorDiff::PopBack;
    assert!(store.can_minimize_diff_list(&[diff.clone(), diff.clone()]));
    store.minimize_diff_list(vec![diff]);

    let items = store.items();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].timeline_id(), "b");
    assert_eq!(items[0].version(), 0);
    assert!(!items[0].processed());

    store.reset_processed_items();

    // Push front.
    let diff = VectorDiff::PushFront {
        value: TestTimelineItemData {
            timeline_id: "a",
            version: 1,
        },
    };
    assert!(store.can_minimize_diff_list(&[diff.clone(), diff.clone()]));
    store.minimize_diff_list(vec![diff]);

    let items = store.items();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].timeline_id(), "a");
    assert_eq!(items[0].version(), 1);
    assert!(items[0].processed());
    assert_eq!(items[1].timeline_id(), "b");
    assert_eq!(items[1].version(), 0);
    assert!(items[1].processed());

    store.reset_processed_items();

    // Push back.
    let diff = VectorDiff::PushBack {
        value: TestTimelineItemData {
            timeline_id: "d",
            version: 0,
        },
    };
    assert!(store.can_minimize_diff_list(&[diff.clone(), diff.clone()]));
    store.minimize_diff_list(vec![diff]);

    let items = store.items();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0].timeline_id(), "a");
    assert_eq!(items[0].version(), 1);
    assert!(!items[0].processed());
    assert_eq!(items[1].timeline_id(), "b");
    assert_eq!(items[1].version(), 0);
    assert!(!items[1].processed());
    assert_eq!(items[2].timeline_id(), "d");
    assert_eq!(items[2].version(), 0);
    assert!(items[2].processed());

    store.reset_processed_items();

    // Insert.
    let diff = VectorDiff::Insert {
        index: 2,
        value: TestTimelineItemData {
            timeline_id: "c",
            version: 1,
        },
    };
    assert!(store.can_minimize_diff_list(&[diff.clone(), diff.clone()]));
    store.minimize_diff_list(vec![diff]);

    let items = store.items();
    assert_eq!(items.len(), 4);
    assert_eq!(items[0].timeline_id(), "a");
    assert_eq!(items[0].version(), 1);
    assert!(!items[0].processed());
    assert_eq!(items[1].timeline_id(), "b");
    assert_eq!(items[1].version(), 0);
    assert!(!items[1].processed());
    assert_eq!(items[2].timeline_id(), "c");
    assert_eq!(items[2].version(), 1);
    assert!(items[2].processed());
    assert_eq!(items[3].timeline_id(), "d");
    assert_eq!(items[3].version(), 0);
    assert!(items[3].processed());

    store.reset_processed_items();

    // Set same item (update).
    let diff = VectorDiff::Set {
        index: 1,
        value: TestTimelineItemData {
            timeline_id: "b",
            version: 1,
        },
    };
    assert!(store.can_minimize_diff_list(&[diff.clone(), diff.clone()]));
    store.minimize_diff_list(vec![diff]);

    let items = store.items();
    assert_eq!(items.len(), 4);
    assert_eq!(items[0].timeline_id(), "a");
    assert_eq!(items[0].version(), 1);
    assert!(!items[0].processed());
    assert_eq!(items[1].timeline_id(), "b");
    assert_eq!(items[1].version(), 1);
    assert!(items[1].processed());
    assert_eq!(items[2].timeline_id(), "c");
    assert_eq!(items[2].version(), 1);
    assert!(items[2].processed());
    assert_eq!(items[3].timeline_id(), "d");
    assert_eq!(items[3].version(), 0);
    assert!(!items[3].processed());

    store.reset_processed_items();

    // Set new item (replace).
    let diff = VectorDiff::Set {
        index: 1,
        value: TestTimelineItemData {
            timeline_id: "b1",
            version: 0,
        },
    };
    assert!(store.can_minimize_diff_list(&[diff.clone(), diff.clone()]));
    store.minimize_diff_list(vec![diff]);

    let items = store.items();
    assert_eq!(items.len(), 4);
    assert_eq!(items[0].timeline_id(), "a");
    assert_eq!(items[0].version(), 1);
    assert!(!items[0].processed());
    assert_eq!(items[1].timeline_id(), "b1");
    assert_eq!(items[1].version(), 0);
    assert!(items[1].processed());
    assert_eq!(items[2].timeline_id(), "c");
    assert_eq!(items[2].version(), 1);
    assert!(items[2].processed());
    assert_eq!(items[3].timeline_id(), "d");
    assert_eq!(items[3].version(), 0);
    assert!(!items[3].processed());

    store.reset_processed_items();

    // The following variants are not supported.
    let diff = VectorDiff::Clear;
    assert!(!store.can_minimize_diff_list(&[diff.clone(), diff.clone()]));

    let diff = VectorDiff::Truncate { length: 2 };
    assert!(!store.can_minimize_diff_list(&[diff.clone(), diff.clone()]));

    let diff = VectorDiff::Reset {
        values: Vector::new(),
    };
    assert!(!store.can_minimize_diff_list(&[diff.clone(), diff.clone()]));

    // And empty list or with a single item cannot be minimized.
    assert!(!store.can_minimize_diff_list(&[]));
    assert!(!store.can_minimize_diff_list(&[VectorDiff::PopBack]));
}

/// Minimize only insertions or only removals.
#[test]
fn minimize_simple_diff() {
    let store = TestTimelineItemStore::default();

    // Minimize out of order insertions.
    let diff_list = vec![
        VectorDiff::PushBack {
            value: TestTimelineItemData {
                timeline_id: "b",
                version: 0,
            },
        },
        VectorDiff::PushBack {
            value: TestTimelineItemData {
                timeline_id: "d",
                version: 0,
            },
        },
        VectorDiff::PushFront {
            value: TestTimelineItemData {
                timeline_id: "a",
                version: 0,
            },
        },
        VectorDiff::Insert {
            index: 2,
            value: TestTimelineItemData {
                timeline_id: "c",
                version: 0,
            },
        },
    ];
    assert!(store.can_minimize_diff_list(&diff_list));

    let mut minimizer = TimelineItemDiffMinimizer::new(&store);

    assert_eq!(store.items().len(), 0);
    let old_item_ids = minimizer.load_items();
    assert_eq!(old_item_ids.len(), 0);

    let new_item_ids = minimizer.apply_diff_to_items(&old_item_ids, diff_list);
    assert_eq!(new_item_ids.len(), 4);
    assert_eq!(new_item_ids[0], "a");
    assert_eq!(new_item_ids[1], "b");
    assert_eq!(new_item_ids[2], "c");
    assert_eq!(new_item_ids[3], "d");

    let item_diff_list = minimizer.item_diff_list(&old_item_ids, &new_item_ids);
    assert_eq!(item_diff_list.len(), 1);
    assert_matches!(&item_diff_list[0], TimelineItemDiff::Splice(splice_diff));
    assert_eq!(splice_diff.pos, 0);
    assert_eq!(splice_diff.n_removals, 0);
    assert_eq!(splice_diff.additions.len(), 4);

    store.apply_item_diff_list(item_diff_list);
    let items = store.items();
    assert_eq!(items.len(), 4);
    assert_eq!(items[0].timeline_id(), "a");
    assert_eq!(items[0].version(), 0);
    assert!(items[0].processed());
    assert_eq!(items[1].timeline_id(), "b");
    assert_eq!(items[1].version(), 0);
    assert!(items[1].processed());
    assert_eq!(items[2].timeline_id(), "c");
    assert_eq!(items[2].version(), 0);
    assert!(items[2].processed());
    assert_eq!(items[3].timeline_id(), "d");
    assert_eq!(items[3].version(), 0);
    assert!(items[3].processed());

    // Minimize out of order removals.
    let diff_list = vec![
        VectorDiff::PopBack,
        VectorDiff::Remove { index: 1 },
        VectorDiff::PopBack,
        VectorDiff::PopFront,
    ];
    assert!(store.can_minimize_diff_list(&diff_list));

    let mut minimizer = TimelineItemDiffMinimizer::new(&store);

    assert_eq!(store.items().len(), 4);
    let old_item_ids = minimizer.load_items();
    assert_eq!(old_item_ids.len(), 4);

    let new_item_ids = minimizer.apply_diff_to_items(&old_item_ids, diff_list);
    assert_eq!(new_item_ids.len(), 0);

    let item_diff_list = minimizer.item_diff_list(&old_item_ids, &new_item_ids);
    assert_eq!(item_diff_list.len(), 1);
    assert_matches!(&item_diff_list[0], TimelineItemDiff::Splice(splice_diff));
    assert_eq!(splice_diff.pos, 0);
    assert_eq!(splice_diff.n_removals, 4);
    assert_eq!(splice_diff.additions.len(), 0);

    store.apply_item_diff_list(item_diff_list);
    let items = store.items();
    assert_eq!(items.len(), 0);
}

/// Minimize mix of insertions and removals.
#[test]
fn minimize_complex_diff() {
    let store = TestTimelineItemStore::default();
    // Populate the store first.
    store.minimize_diff_list(vec![VectorDiff::Append {
        values: Vector::from([
            TestTimelineItemData {
                timeline_id: "a",
                version: 0,
            },
            TestTimelineItemData {
                timeline_id: "c",
                version: 0,
            },
            TestTimelineItemData {
                timeline_id: "d",
                version: 0,
            },
            TestTimelineItemData {
                timeline_id: "e",
                version: 0,
            },
            TestTimelineItemData {
                timeline_id: "f",
                version: 0,
            },
            TestTimelineItemData {
                timeline_id: "g",
                version: 0,
            },
            TestTimelineItemData {
                timeline_id: "h",
                version: 0,
            },
        ]),
    }]);
    store.reset_processed_items();

    let diff_list = vec![
        VectorDiff::Remove { index: 1 },
        VectorDiff::Insert {
            index: 1,
            value: TestTimelineItemData {
                timeline_id: "b",
                version: 0,
            },
        },
        VectorDiff::Insert {
            index: 2,
            value: TestTimelineItemData {
                timeline_id: "c",
                version: 1,
            },
        },
        VectorDiff::PopBack,
        VectorDiff::Set {
            index: 3,
            value: TestTimelineItemData {
                timeline_id: "d1",
                version: 0,
            },
        },
        VectorDiff::Set {
            index: 4,
            value: TestTimelineItemData {
                timeline_id: "e",
                version: 1,
            },
        },
    ];

    let mut minimizer = TimelineItemDiffMinimizer::new(&store);

    assert_eq!(store.items().len(), 7);
    let old_item_ids = minimizer.load_items();
    assert_eq!(old_item_ids.len(), 7);
    assert_eq!(old_item_ids[0], "a");
    assert_eq!(old_item_ids[1], "c");
    assert_eq!(old_item_ids[2], "d");
    assert_eq!(old_item_ids[3], "e");
    assert_eq!(old_item_ids[4], "f");
    assert_eq!(old_item_ids[5], "g");
    assert_eq!(old_item_ids[6], "h");

    let new_item_ids = minimizer.apply_diff_to_items(&old_item_ids, diff_list);
    assert_eq!(new_item_ids.len(), 7);
    assert_eq!(new_item_ids[0], "a");
    assert_eq!(new_item_ids[1], "b");
    assert_eq!(new_item_ids[2], "c");
    assert_eq!(new_item_ids[3], "d1");
    assert_eq!(new_item_ids[4], "e");
    assert_eq!(new_item_ids[5], "f");
    assert_eq!(new_item_ids[6], "g");

    let item_diff_list = minimizer.item_diff_list(&old_item_ids, &new_item_ids);
    assert_eq!(item_diff_list.len(), 5);
    assert_matches!(&item_diff_list[0], TimelineItemDiff::Splice(splice_diff));
    assert_eq!(splice_diff.pos, 1);
    assert_eq!(splice_diff.n_removals, 0);
    assert_eq!(splice_diff.additions.len(), 1);
    assert_matches!(&item_diff_list[1], TimelineItemDiff::Update(update_diff));
    assert_eq!(update_diff.pos, 2);
    assert_eq!(update_diff.n_items, 1);
    assert_matches!(&item_diff_list[2], TimelineItemDiff::Splice(splice_diff));
    assert_eq!(splice_diff.pos, 3);
    assert_eq!(splice_diff.n_removals, 1);
    assert_eq!(splice_diff.additions.len(), 1);
    assert_matches!(&item_diff_list[3], TimelineItemDiff::Update(update_diff));
    assert_eq!(update_diff.pos, 4);
    assert_eq!(update_diff.n_items, 1);
    assert_matches!(&item_diff_list[4], TimelineItemDiff::Splice(splice_diff));
    assert_eq!(splice_diff.pos, 7);
    assert_eq!(splice_diff.n_removals, 1);
    assert_eq!(splice_diff.additions.len(), 0);

    store.apply_item_diff_list(item_diff_list);
    let items = store.items();
    assert_eq!(items.len(), 7);
    assert_eq!(items[0].timeline_id(), "a");
    assert_eq!(items[0].version(), 0);
    assert!(!items[0].processed());
    assert_eq!(items[1].timeline_id(), "b");
    assert_eq!(items[1].version(), 0);
    assert!(items[1].processed());
    assert_eq!(items[2].timeline_id(), "c");
    assert_eq!(items[2].version(), 1);
    assert!(items[2].processed());
    assert_eq!(items[3].timeline_id(), "d1");
    assert_eq!(items[3].version(), 0);
    assert!(items[3].processed());
    assert_eq!(items[4].timeline_id(), "e");
    assert_eq!(items[4].version(), 1);
    assert!(items[4].processed());
    assert_eq!(items[5].timeline_id(), "f");
    assert_eq!(items[5].version(), 0);
    assert!(items[5].processed());
    assert_eq!(items[6].timeline_id(), "g");
    assert_eq!(items[6].version(), 0);
    assert!(!items[6].processed());
}
