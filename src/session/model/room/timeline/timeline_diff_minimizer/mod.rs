use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

use gtk::prelude::*;
use matrix_sdk_ui::{eyeball_im::VectorDiff, timeline::TimelineItem as SdkTimelineItem};

mod tests;

use super::TimelineItem;

/// Trait to access data from a type that stores [`TimelineDiffItem`]s.
pub(super) trait TimelineDiffItemStore: Sized {
    type Item: TimelineDiffItem;
    type Data: TimelineDiffItemData;

    /// The current list of items.
    fn items(&self) -> Vec<Self::Item>;

    /// Create a `TimelineItem` with the given `TimelineItemData`.
    fn create_item(&self, data: &Self::Data) -> Self::Item;

    /// Update the given item with the given timeline ID.
    fn update_item(&self, item: &Self::Item, data: &Self::Data);

    /// Apply the given list of item diffs to this store.
    fn apply_item_diff_list(&self, item_diff_list: Vec<TimelineDiff<Self::Item>>);

    /// Whether the given diff list can be minimized by calling
    /// `minimize_diff_list`.
    ///
    /// It can be minimized if there is more than 1 item in the list and if the
    /// list only includes supported `VectorDiff` variants.
    fn can_minimize_diff_list(&self, diff_list: &[VectorDiff<Self::Data>]) -> bool {
        diff_list.len() > 1
            && !diff_list.iter().any(|diff| {
                matches!(
                    diff,
                    VectorDiff::Clear | VectorDiff::Truncate { .. } | VectorDiff::Reset { .. }
                )
            })
    }

    /// Minimize the given diff list and apply it to this store.
    ///
    /// Panics if the diff list contains unsupported `VectorDiff` variants. This
    /// will never panic if `can_minimize_diff_list` returns `true`.
    fn minimize_diff_list(&self, diff_list: Vec<VectorDiff<Self::Data>>) {
        TimelineDiffMinimizer::new(self).apply(diff_list);
    }
}

/// Trait implemented by types that provide data for [`TimelineDiffItem`]s.
pub(super) trait TimelineDiffItemData {
    /// The unique timeline ID of the data.
    fn timeline_id(&self) -> &str;
}

impl TimelineDiffItemData for SdkTimelineItem {
    fn timeline_id(&self) -> &str {
        &self.unique_id().0
    }
}

impl<T> TimelineDiffItemData for Arc<T>
where
    T: TimelineDiffItemData,
{
    fn timeline_id(&self) -> &str {
        (**self).timeline_id()
    }
}

/// Trait implemented by items in the timeline.
pub(super) trait TimelineDiffItem: Clone {
    /// The unique timeline ID of the item.
    fn timeline_id(&self) -> String;
}

impl<T> TimelineDiffItem for T
where
    T: IsA<TimelineItem>,
{
    fn timeline_id(&self) -> String {
        self.upcast_ref().timeline_id()
    }
}

/// A helper struct to minimize a list of `VectorDiff`.
///
/// This does not support `VectorDiff::Clear`, `VectorDiff::Truncate` and
/// `VectorDiff::Reset` as we assume that lists including those cannot be
/// minimized in an optimal way.
struct TimelineDiffMinimizer<'a, S, I> {
    store: &'a S,
    item_map: HashMap<String, I>,
    updated_item_ids: Vec<String>,
}

impl<'a, S, I> TimelineDiffMinimizer<'a, S, I> {
    /// Construct a `TimelineDiffMinimizer` with the given store.
    fn new(store: &'a S) -> Self {
        Self {
            store,
            item_map: HashMap::new(),
            updated_item_ids: Vec::new(),
        }
    }
}

impl<S, I> TimelineDiffMinimizer<'_, S, I>
where
    S: TimelineDiffItemStore<Item = I>,
    I: TimelineDiffItem,
{
    /// Load the items from the store.
    ///
    /// Returns the list of timeline IDs of the items.
    fn load_items(&mut self) -> Vec<String> {
        let items = self.store.items();
        let item_ids = items.iter().map(S::Item::timeline_id).collect();

        self.item_map
            .extend(items.into_iter().map(|item| (item.timeline_id(), item)));

        item_ids
    }

    /// Update or create an item in the store using the given data.
    ///
    /// Returns the timeline ID of the item.
    fn update_or_create_item(&mut self, data: &S::Data) -> String {
        let timeline_id = data.timeline_id().to_owned();
        self.item_map
            .entry(timeline_id)
            .and_modify(|item| {
                self.store.update_item(item, data);
                self.updated_item_ids.push(item.timeline_id());
            })
            .or_insert_with(|| self.store.create_item(data))
            .timeline_id()
    }

    /// Apply the given diff to the given items.
    fn apply_diff_to_items(
        &mut self,
        item_ids: &[String],
        diff_list: Vec<VectorDiff<S::Data>>,
    ) -> Vec<String> {
        let mut new_item_ids = VecDeque::from(item_ids.to_owned());

        // Get the new state by applying the diffs.
        for diff in diff_list {
            match diff {
                VectorDiff::Append { values } => {
                    let items = values
                        .into_iter()
                        .map(|data| self.update_or_create_item(data));
                    new_item_ids.extend(items);
                }
                VectorDiff::PushFront { value } => {
                    let item = self.update_or_create_item(&value);
                    new_item_ids.push_front(item);
                }
                VectorDiff::PushBack { value } => {
                    let item = self.update_or_create_item(&value);
                    new_item_ids.push_back(item);
                }
                VectorDiff::PopFront => {
                    new_item_ids.pop_front();
                }
                VectorDiff::PopBack => {
                    new_item_ids.pop_back();
                }
                VectorDiff::Insert { index, value } => {
                    let item = self.update_or_create_item(&value);
                    new_item_ids.insert(index, item);
                }
                VectorDiff::Set { index, value } => {
                    let item_id = self.update_or_create_item(&value);
                    *new_item_ids
                        .get_mut(index)
                        .expect("an item should already exist at the given index") = item_id;
                }
                VectorDiff::Remove { index } => {
                    new_item_ids.remove(index);
                }
                VectorDiff::Clear | VectorDiff::Truncate { .. } | VectorDiff::Reset { .. } => {
                    unreachable!()
                }
            }
        }

        new_item_ids.into()
    }

    /// Compute the list of item diffs between the two given lists.
    ///
    /// Uses a diff algorithm to minimize the removals and additions.
    fn item_diff_list(
        &self,
        old_item_ids: &[String],
        new_item_ids: &[String],
    ) -> Vec<TimelineDiff<S::Item>> {
        let mut item_diff_list = Vec::new();
        let mut pos = 0;
        // Group diffs in batch.
        let mut n_removals = 0;
        let mut additions = None;
        let mut n_updates = 0;

        for result in diff::slice(old_item_ids, new_item_ids) {
            match result {
                diff::Result::Left(_) => {
                    if let Some(additions) = additions.take() {
                        let item_diff = SpliceDiff {
                            pos,
                            n_removals: 0,
                            additions,
                        };
                        pos += item_diff.additions.len() as u32;
                        item_diff_list.push(item_diff.into());
                    } else if n_updates > 0 {
                        let item_diff = UpdateDiff {
                            pos,
                            n_items: n_updates,
                        };
                        item_diff_list.push(item_diff.into());

                        pos += n_updates;
                        n_updates = 0;
                    }

                    n_removals += 1;
                }
                diff::Result::Both(timeline_id, _) => {
                    if additions.is_some() || n_removals > 0 {
                        let item_diff = SpliceDiff {
                            pos,
                            n_removals,
                            additions: additions.take().unwrap_or_default(),
                        };
                        pos += item_diff.additions.len() as u32;
                        item_diff_list.push(item_diff.into());

                        n_removals = 0;
                    }

                    if self.updated_item_ids.contains(timeline_id) {
                        n_updates += 1;
                    } else {
                        if n_updates > 0 {
                            let item_diff = UpdateDiff {
                                pos,
                                n_items: n_updates,
                            };
                            item_diff_list.push(item_diff.into());

                            pos += n_updates;
                            n_updates = 0;
                        }

                        pos += 1;
                    }
                }
                diff::Result::Right(timeline_id) => {
                    if n_updates > 0 {
                        let item_diff = UpdateDiff {
                            pos,
                            n_items: n_updates,
                        };
                        item_diff_list.push(item_diff.into());

                        pos += n_updates;
                        n_updates = 0;
                    }

                    let item = self
                        .item_map
                        .get(timeline_id)
                        .expect("item should exist in map")
                        .clone();
                    additions.get_or_insert_with(Vec::new).push(item);
                }
            }
        }

        // Process the remaining batches.
        if additions.is_some() || n_removals > 0 {
            let item_diff = SpliceDiff {
                pos,
                n_removals,
                additions: additions.take().unwrap_or_default(),
            };
            item_diff_list.push(item_diff.into());
        } else if n_updates > 0 {
            let item_diff = UpdateDiff {
                pos,
                n_items: n_updates,
            };
            item_diff_list.push(item_diff.into());
        }

        item_diff_list
    }

    /// Minimize the given diff and apply it to the store.
    fn apply(mut self, diff_list: Vec<VectorDiff<S::Data>>) {
        let old_item_ids = self.load_items();
        let new_item_ids = self.apply_diff_to_items(&old_item_ids, diff_list);
        let item_diff_list = self.item_diff_list(&old_item_ids, &new_item_ids);
        self.store.apply_item_diff_list(item_diff_list);
    }
}

/// A minimized diff for timeline items.
#[derive(Debug, Clone)]
pub(super) enum TimelineDiff<T> {
    /// Remove then add items.
    Splice(SpliceDiff<T>),

    /// Update items.
    Update(UpdateDiff),
}

impl<T> From<SpliceDiff<T>> for TimelineDiff<T> {
    fn from(value: SpliceDiff<T>) -> Self {
        Self::Splice(value)
    }
}

impl<T> From<UpdateDiff> for TimelineDiff<T> {
    fn from(value: UpdateDiff) -> Self {
        Self::Update(value)
    }
}

/// A diff to remove then add items.
#[derive(Debug, Clone)]
pub(super) struct SpliceDiff<T> {
    /// The position where the change happens
    pub(super) pos: u32,
    /// The number of items to remove.
    pub(super) n_removals: u32,
    /// The items to add.
    pub(super) additions: Vec<T>,
}

/// A diff to update items.
#[derive(Debug, Clone)]
pub(super) struct UpdateDiff {
    /// The position from where to start updating items.
    pub(super) pos: u32,
    /// The number of items to update.
    pub(super) n_items: u32,
}
