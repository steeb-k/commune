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

/// Apply a diff whose values are already wrapped and keyed.
///
/// Wrapping happens before this is called, so a wrapper's constructor may
/// look the list up without borrowing it twice; the caller reuses the
/// existing wrapper for a key it still has. Returns the changes to report,
/// in order.
pub(crate) fn apply_diff<K, W>(
    map: &mut IndexMap<K, W>,
    diff: VectorDiff<(K, W)>,
) -> Vec<ItemsChange>
where
    K: Hash + Eq + Clone,
    W: Clone,
{
    let mut changes = Vec::new();

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
            map.clear();
            push(&mut changes, 0, removed, 0);
        }
        VectorDiff::PushFront { value } => insert_at(map, 0, value, &mut changes),
        VectorDiff::PushBack { value } => {
            let index = map.len();
            insert_at(map, index, value, &mut changes);
        }
        VectorDiff::PopFront => remove_at(map, 0, &mut changes),
        VectorDiff::PopBack => {
            if let Some(index) = map.len().checked_sub(1) {
                remove_at(map, index, &mut changes);
            }
        }
        VectorDiff::Insert { index, value } => insert_at(map, index, value, &mut changes),
        VectorDiff::Set { index, value } => {
            let (key, wrapper) = value;
            if map.get_index(index).is_some_and(|(k, _)| *k == key) {
                // The same value in the same place; its wrapper follows the
                // value itself.
                return changes;
            }
            remove_at(map, index, &mut changes);
            insert_at(map, index, (key, wrapper), &mut changes);
        }
        VectorDiff::Remove { index } => remove_at(map, index, &mut changes),
        VectorDiff::Truncate { length } => {
            let removed = map.len().saturating_sub(length);
            map.truncate(length);
            push(&mut changes, length, removed, 0);
        }
        VectorDiff::Reset { values } => {
            let removed = map.len();
            map.clear();
            for (key, wrapper) in values {
                map.insert(key, wrapper);
            }
            push(&mut changes, 0, removed, map.len());
        }
    }

    changes
}

/// Insert a wrapper at the given index, moving it there if its key is
/// already in the map.
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

/// Remove the wrapper at the given index, if there is one.
fn remove_at<K, W>(map: &mut IndexMap<K, W>, index: usize, changes: &mut Vec<ItemsChange>)
where
    K: Hash + Eq,
{
    if map.shift_remove_index(index).is_some() {
        push(changes, index, 1, 0);
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

        let changes = apply_diff(
            &mut map,
            VectorDiff::Append {
                values: keyed(&[2, 3]).into_iter().collect(),
            },
        );

        assert_eq!(keys(&map), [1, 2, 3]);
        assert_eq!(
            changes,
            [ItemsChange {
                position: 1,
                removed: 0,
                added: 2
            }]
        );
    }

    #[test]
    fn remove_and_insert_keep_order() {
        let mut map: IndexMap<u32, String> = keyed(&[1, 2, 3]).into_iter().collect();

        let removed = apply_diff(&mut map, VectorDiff::Remove { index: 1 });
        assert_eq!(keys(&map), [1, 3]);
        assert_eq!(
            removed,
            [ItemsChange {
                position: 1,
                removed: 1,
                added: 0
            }]
        );

        let inserted = apply_diff(
            &mut map,
            VectorDiff::Insert {
                index: 1,
                value: (4, "4".to_owned()),
            },
        );
        assert_eq!(keys(&map), [1, 4, 3]);
        assert_eq!(
            inserted,
            [ItemsChange {
                position: 1,
                removed: 0,
                added: 1
            }]
        );
    }

    #[test]
    fn reset_keeps_nothing_but_reports_both_sides() {
        let mut map: IndexMap<u32, String> = keyed(&[1, 2]).into_iter().collect();

        let changes = apply_diff(
            &mut map,
            VectorDiff::Reset {
                values: keyed(&[3]).into_iter().collect(),
            },
        );

        assert_eq!(keys(&map), [3]);
        assert_eq!(
            changes,
            [ItemsChange {
                position: 0,
                removed: 2,
                added: 1
            }]
        );
    }

    #[test]
    fn set_of_the_same_key_changes_nothing() {
        let mut map: IndexMap<u32, String> = keyed(&[1, 2]).into_iter().collect();

        let changes = apply_diff(
            &mut map,
            VectorDiff::Set {
                index: 1,
                value: (2, "two".to_owned()),
            },
        );

        assert!(changes.is_empty());
        assert_eq!(map[&2], "2");
    }

    #[test]
    fn inserting_a_known_key_moves_it() {
        let mut map: IndexMap<u32, String> = keyed(&[1, 2, 3]).into_iter().collect();

        let changes = apply_diff(
            &mut map,
            VectorDiff::PushFront {
                value: (3, "three".to_owned()),
            },
        );

        assert_eq!(keys(&map), [3, 1, 2]);
        assert_eq!(map[&3], "3");
        assert_eq!(
            changes,
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
    }
}
