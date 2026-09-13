//! The list half of the bridge: a `VectorDiff` applied to a wrapper cache.
//!
//! A `GObject` subclass cannot be generic, so this is not a `gio::ListModel`;
//! it is the function every list model over a core vector calls from the
//! thread the diff was handed to, and the `IndexMap` it maintains is the
//! wrapper cache the sidebar's filter and sort stacks depend on: keyed by the
//! core value's identity, it hands back the same `GObject` for the same value
//! across diffs, so identity survives and every `.blp` binding holds.

use std::hash::Hash;

use commune_core::VectorDiff;
use indexmap::IndexMap;

/// What `items_changed` should hear.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ItemsChange {
    /// Where the change starts.
    pub position: u32,
    /// How many items were removed there.
    pub removed: u32,
    /// How many items were added there.
    pub added: u32,
}

/// What applying a diff produced: the changes to report, in order, and the
/// wrappers the diff retired.
///
/// The caller drops `retired` only after it has released the map's borrow
/// and emitted `changes`: a wrapper's finalize can re-enter the list (a
/// `GtkFilterListModel` watching its items looks the item up by scanning
/// `n_items`/`item` when a watched item is finalized), and a live `RefCell`
/// borrow at that point aborts the process.
#[derive(Debug)]
pub(crate) struct Applied<W> {
    pub changes: Vec<ItemsChange>,
    pub retired: Vec<W>,
}

/// Apply a diff whose values are already wrapped and keyed.
///
/// Wrapping happens before this is called, so a wrapper's constructor may
/// look the list up without borrowing it twice; the caller reuses the
/// existing wrapper for a key it still has. Returns the changes to report,
/// in order, and the wrappers the diff retired: this never drops one while
/// `map` is still reachable through the caller's borrow.
pub(crate) fn apply_diff<K, W>(map: &mut IndexMap<K, W>, diff: VectorDiff<(K, W)>) -> Applied<W>
where
    K: Hash + Eq + Clone,
    W: Clone,
{
    let mut changes = Vec::new();
    let mut retired = Vec::new();

    match diff {
        VectorDiff::Append { values } => {
            let position = map.len();
            let mut added = 0;
            for (key, wrapper) in values {
                if map.contains_key(&key) {
                    continue;
                }
                map.insert(key, wrapper);
                added += 1;
            }
            push(&mut changes, position, 0, added);
        }
        VectorDiff::Clear => {
            let removed = map.len();
            retired.extend(map.drain(..).map(|(_, wrapper)| wrapper));
            push(&mut changes, 0, removed, 0);
        }
        VectorDiff::PushFront { value } => insert_at(map, 0, value, &mut changes),
        VectorDiff::PushBack { value } => {
            let index = map.len();
            insert_at(map, index, value, &mut changes);
        }
        VectorDiff::PopFront => remove_at(map, 0, &mut changes, &mut retired),
        VectorDiff::PopBack => {
            if let Some(index) = map.len().checked_sub(1) {
                remove_at(map, index, &mut changes, &mut retired);
            }
        }
        VectorDiff::Insert { index, value } => insert_at(map, index, value, &mut changes),
        VectorDiff::Set { index, value } => {
            let (key, wrapper) = value;
            if map.get_index(index).is_some_and(|(k, _)| *k == key) {
                // The same value in the same place; its wrapper follows the
                // value itself.
                return Applied { changes, retired };
            }
            remove_at(map, index, &mut changes, &mut retired);
            insert_at(map, index, (key, wrapper), &mut changes);
        }
        VectorDiff::Remove { index } => remove_at(map, index, &mut changes, &mut retired),
        VectorDiff::Truncate { length } => {
            let removed = map.len().saturating_sub(length);
            if removed > 0 {
                retired.extend(map.drain(length..).map(|(_, wrapper)| wrapper));
            }
            push(&mut changes, length, removed, 0);
        }
        VectorDiff::Reset { values } => {
            let removed = map.len();
            retired.extend(map.drain(..).map(|(_, wrapper)| wrapper));
            for (key, wrapper) in values {
                map.insert(key, wrapper);
            }
            push(&mut changes, 0, removed, map.len());
        }
    }

    Applied { changes, retired }
}

/// Insert a wrapper at the given index, moving it there if its key is
/// already in the map.
///
/// The move path hands the same wrapper straight back to the map: it is
/// not retired, since it never stops being reachable from the list.
fn insert_at<K, W>(
    map: &mut IndexMap<K, W>,
    index: usize,
    value: (K, W),
    changes: &mut Vec<ItemsChange>,
) where
    K: Hash + Eq,
{
    let (key, wrapper) = value;

    let wrapper = match map.get_index_of(&key) {
        // Already where the diff puts it: an embedder that adds a wrapper
        // ahead of the diff, at the index the core promised.
        Some(old_index) if old_index == index => return,
        Some(old_index) => {
            let (_, existing) = map
                .shift_remove_index(old_index)
                .expect("the index was just looked up");
            push(changes, old_index, 1, 0);
            existing
        }
        None => wrapper,
    };

    let index = index.min(map.len());
    map.shift_insert(index, key, wrapper);
    push(changes, index, 0, 1);
}

/// Remove the wrapper at the given index, if there is one, and push it onto
/// `retired` rather than dropping it here.
fn remove_at<K, W>(
    map: &mut IndexMap<K, W>,
    index: usize,
    changes: &mut Vec<ItemsChange>,
    retired: &mut Vec<W>,
) where
    K: Hash + Eq,
{
    if let Some((_, wrapper)) = map.shift_remove_index(index) {
        push(changes, index, 1, 0);
        retired.push(wrapper);
    }
}

/// Record a change, unless it changes nothing.
fn push(changes: &mut Vec<ItemsChange>, position: usize, removed: usize, added: usize) {
    if removed == 0 && added == 0 {
        return;
    }

    changes.push(ItemsChange {
        position: position as u32,
        removed: removed as u32,
        added: added as u32,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keyed(values: &[u32]) -> Vec<(u32, String)> {
        values.iter().map(|v| (*v, v.to_string())).collect()
    }

    fn keys(map: &IndexMap<u32, String>) -> Vec<u32> {
        map.keys().copied().collect()
    }

    #[test]
    fn append_reports_the_tail() {
        let mut map = IndexMap::new();
        map.insert(1, "1".to_owned());

        let applied = apply_diff(
            &mut map,
            VectorDiff::Append {
                values: keyed(&[2, 3]).into_iter().collect(),
            },
        );

        assert_eq!(keys(&map), [1, 2, 3]);
        assert_eq!(
            applied.changes,
            [ItemsChange {
                position: 1,
                removed: 0,
                added: 2
            }]
        );
        assert!(applied.retired.is_empty());
    }

    #[test]
    fn remove_and_insert_keep_order() {
        let mut map: IndexMap<u32, String> = keyed(&[1, 2, 3]).into_iter().collect();

        let removed = apply_diff(&mut map, VectorDiff::Remove { index: 1 });
        assert_eq!(keys(&map), [1, 3]);
        assert_eq!(
            removed.changes,
            [ItemsChange {
                position: 1,
                removed: 1,
                added: 0
            }]
        );
        assert_eq!(removed.retired, ["2".to_owned()]);

        let inserted = apply_diff(
            &mut map,
            VectorDiff::Insert {
                index: 1,
                value: (4, "4".to_owned()),
            },
        );
        assert_eq!(keys(&map), [1, 4, 3]);
        assert_eq!(
            inserted.changes,
            [ItemsChange {
                position: 1,
                removed: 0,
                added: 1
            }]
        );
        assert!(inserted.retired.is_empty());
    }

    #[test]
    fn pop_front_and_pop_back_retire_the_ends() {
        let mut map: IndexMap<u32, String> = keyed(&[1, 2, 3]).into_iter().collect();

        let front = apply_diff(&mut map, VectorDiff::PopFront);
        assert_eq!(keys(&map), [2, 3]);
        assert_eq!(
            front.changes,
            [ItemsChange {
                position: 0,
                removed: 1,
                added: 0
            }]
        );
        assert_eq!(front.retired, ["1".to_owned()]);

        let back = apply_diff(&mut map, VectorDiff::PopBack);
        assert_eq!(keys(&map), [2]);
        assert_eq!(
            back.changes,
            [ItemsChange {
                position: 1,
                removed: 1,
                added: 0
            }]
        );
        assert_eq!(back.retired, ["3".to_owned()]);
    }

    #[test]
    fn set_of_a_different_key_retires_the_old_wrapper() {
        let mut map: IndexMap<u32, String> = keyed(&[1, 2, 3]).into_iter().collect();

        let applied = apply_diff(
            &mut map,
            VectorDiff::Set {
                index: 1,
                value: (4, "4".to_owned()),
            },
        );

        assert_eq!(keys(&map), [1, 4, 3]);
        assert_eq!(
            applied.changes,
            [
                ItemsChange {
                    position: 1,
                    removed: 1,
                    added: 0
                },
                ItemsChange {
                    position: 1,
                    removed: 0,
                    added: 1
                }
            ]
        );
        assert_eq!(applied.retired, ["2".to_owned()]);
    }

    #[test]
    fn clear_retires_everything() {
        let mut map: IndexMap<u32, String> = keyed(&[1, 2, 3]).into_iter().collect();

        let applied = apply_diff(&mut map, VectorDiff::Clear);

        assert!(map.is_empty());
        assert_eq!(
            applied.changes,
            [ItemsChange {
                position: 0,
                removed: 3,
                added: 0
            }]
        );
        assert_eq!(
            applied.retired,
            ["1".to_owned(), "2".to_owned(), "3".to_owned()]
        );
    }

    #[test]
    fn truncate_retires_the_dropped_tail() {
        let mut map: IndexMap<u32, String> = keyed(&[1, 2, 3]).into_iter().collect();

        let applied = apply_diff(&mut map, VectorDiff::Truncate { length: 1 });

        assert_eq!(keys(&map), [1]);
        assert_eq!(
            applied.changes,
            [ItemsChange {
                position: 1,
                removed: 2,
                added: 0
            }]
        );
        assert_eq!(applied.retired, ["2".to_owned(), "3".to_owned()]);
    }

    #[test]
    fn reset_keeps_nothing_but_reports_both_sides() {
        let mut map: IndexMap<u32, String> = keyed(&[1, 2]).into_iter().collect();

        let applied = apply_diff(
            &mut map,
            VectorDiff::Reset {
                values: keyed(&[3]).into_iter().collect(),
            },
        );

        assert_eq!(keys(&map), [3]);
        assert_eq!(
            applied.changes,
            [ItemsChange {
                position: 0,
                removed: 2,
                added: 1
            }]
        );
        assert_eq!(applied.retired, ["1".to_owned(), "2".to_owned()]);
    }

    #[test]
    fn set_of_the_same_key_changes_nothing() {
        let mut map: IndexMap<u32, String> = keyed(&[1, 2]).into_iter().collect();

        let applied = apply_diff(
            &mut map,
            VectorDiff::Set {
                index: 1,
                value: (2, "two".to_owned()),
            },
        );

        assert!(applied.changes.is_empty());
        assert!(applied.retired.is_empty());
        assert_eq!(map[&2], "2");
    }

    #[test]
    fn inserting_a_known_key_moves_it_without_retiring_it() {
        let mut map: IndexMap<u32, String> = keyed(&[1, 2, 3]).into_iter().collect();

        let applied = apply_diff(
            &mut map,
            VectorDiff::PushFront {
                value: (3, "three".to_owned()),
            },
        );

        assert_eq!(keys(&map), [3, 1, 2]);
        assert_eq!(map[&3], "3");
        assert_eq!(
            applied.changes,
            [
                ItemsChange {
                    position: 2,
                    removed: 1,
                    added: 0
                },
                ItemsChange {
                    position: 0,
                    removed: 0,
                    added: 1
                }
            ]
        );
        assert!(applied.retired.is_empty());
    }
}
