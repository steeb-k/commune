use gettextrs::gettext;
use gtk::{
    gio, glib,
    glib::{clone, closure},
    prelude::*,
    subclass::prelude::*,
};

use super::MembershipSubpageItem;
use crate::{
    components::LoadingRow,
    session::model::{Member, MemberList, Membership},
    utils::{BoundConstructOnlyObject, ExpressionListModel, LoadingState},
};

mod imp {
    use std::cell::{Cell, OnceCell};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::MembershipLists)]
    pub struct MembershipLists {
        /// The list of all members.
        #[property(get, set = Self::set_members)]
        members: BoundConstructOnlyObject<MemberList>,
        /// The list of joined members.
        #[property(get)]
        joined: OnceCell<gio::ListModel>,
        /// The list of extra items in the joined list.
        #[property(get = Self::extra_joined_items_owned)]
        extra_joined_items: OnceCell<gio::ListStore>,
        /// The full list to present for joined members.
        #[property(get)]
        joined_full: OnceCell<gio::ListModel>,
        /// The list of invited members.
        #[property(get)]
        invited: OnceCell<gio::ListModel>,
        /// Whether the list of invited members is empty.
        #[property(get)]
        invited_is_empty: Cell<bool>,
        /// The list of banned members.
        #[property(get)]
        banned: OnceCell<gio::ListModel>,
        /// Whether the list of banned members is empty.
        #[property(get)]
        banned_is_empty: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MembershipLists {
        const NAME: &'static str = "ContentMembershipLists";
        type Type = super::MembershipLists;
    }

    #[glib::derived_properties]
    impl ObjectImpl for MembershipLists {}

    impl MembershipLists {
        /// The list of extra items in the joined list.
        fn extra_joined_items(&self) -> &gio::ListStore {
            self.extra_joined_items
                .get_or_init(gio::ListStore::new::<glib::Object>)
        }

        /// The owned list of extra items in the joined list.
        fn extra_joined_items_owned(&self) -> gio::ListStore {
            self.extra_joined_items().clone()
        }

        /// Set the list of all members.
        fn set_members(&self, members: MemberList) {
            // Watch the loading state.
            let signal_handler_ids = vec![members.connect_state_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |members| {
                    imp.update_loading_state(members.state());
                }
            ))];
            self.members.set(members.clone(), signal_handler_ids);
            self.update_loading_state(members.state());

            // Sort the members list by power level, then display name.
            let power_level_expr = Member::this_expression("power-level");
            let sorter = gtk::MultiSorter::new();
            sorter.append(
                gtk::NumericSorter::builder()
                    .expression(&power_level_expr)
                    .sort_order(gtk::SortType::Descending)
                    .build(),
            );

            let display_name_expr = Member::this_expression("display-name");
            sorter.append(gtk::StringSorter::new(Some(&display_name_expr)));

            // We need to notify when a watched property changes so the filter and sorter
            // can update the list.
            let expr_members = ExpressionListModel::new();
            expr_members.set_expressions(vec![
                power_level_expr.upcast(),
                display_name_expr.upcast(),
                Member::this_expression("membership").upcast(),
            ]);
            expr_members.set_model(Some(members));

            let sorted_members = gtk::SortListModel::new(Some(expr_members), Some(sorter));

            let joined = self
                .joined
                .get_or_init(|| build_filtered_list(sorted_members.clone(), Membership::Join));

            let model_list = gio::ListStore::new::<gio::ListModel>();
            model_list.append(self.extra_joined_items());
            model_list.append(joined);
            self.joined_full
                .set(gtk::FlattenListModel::new(Some(model_list)).upcast())
                .expect("full list for joined members is uninitialized");

            let invited = self
                .invited
                .get_or_init(|| build_filtered_list(sorted_members.clone(), Membership::Invite));
            invited.connect_items_changed(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_, _, _, _| {
                    imp.update_invited();
                }
            ));

            let banned = self
                .banned
                .get_or_init(|| build_filtered_list(sorted_members, Membership::Ban));
            banned.connect_items_changed(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_, _, _, _| {
                    imp.update_banned();
                }
            ));

            self.update_invited();
            self.update_banned();
        }

        /// Whether the extra joined items list contain a loading row.
        fn has_loading_row(&self) -> bool {
            self.extra_joined_items()
                .item(0)
                .is_some_and(|item| item.is::<LoadingRow>())
        }

        /// Update the extra joined items list for the given loading state.
        fn update_loading_state(&self, state: LoadingState) {
            if state == LoadingState::Ready {
                if self.has_loading_row() {
                    self.extra_joined_items().remove(0);
                }

                return;
            }

            let loading_row = if let Some(loading_row) = self
                .extra_joined_items()
                .item(0)
                .and_downcast::<LoadingRow>()
            {
                loading_row
            } else {
                let loading_row = LoadingRow::new();
                loading_row.connect_retry(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.members.obj().reload();
                    }
                ));

                self.extra_joined_items().insert(0, &loading_row);
                loading_row
            };

            let error = (state == LoadingState::Error)
                .then(|| gettext("Could not load the full list of room members"));
            loading_row.set_error(error.as_deref());
        }

        /// Whether the extra joined items list contain a membership subpage
        /// item for the given membership at the given position.
        fn has_membership_item_at(&self, membership: Membership, position: u32) -> bool {
            self.extra_joined_items()
                .item(position)
                .and_downcast::<MembershipSubpageItem>()
                .is_some_and(|item| item.membership() == membership)
        }

        /// Update the extra joined items list for the invited members.
        fn update_invited(&self) {
            let was_empty = self.invited_is_empty.get();
            let is_empty = self
                .invited
                .get()
                .expect("invited members are initialized")
                .n_items()
                == 0;

            if was_empty == is_empty {
                // Nothing changed.
                return;
            }

            self.invited_is_empty.set(is_empty);
            let position = self.has_loading_row().into();

            let has_invite_row = self.has_membership_item_at(Membership::Invite, position);
            if is_empty && has_invite_row {
                self.extra_joined_items().remove(position);
            } else if !is_empty && !has_invite_row {
                let invite_item = MembershipSubpageItem::new(
                    Membership::Invite,
                    self.invited.get().expect("invited members are initialized"),
                );
                self.extra_joined_items().insert(position, &invite_item);
            }

            self.obj().notify_invited_is_empty();
        }

        /// Update the extra joined items list for the banned members.
        fn update_banned(&self) {
            let was_empty = self.banned_is_empty.get();
            let is_empty = self
                .banned
                .get()
                .expect("banned members are initialized")
                .n_items()
                == 0;

            if was_empty == is_empty {
                // Nothing changed so don't do anything
                return;
            }

            self.banned_is_empty.set(is_empty);

            let mut position = u32::from(self.has_loading_row());
            position += u32::from(self.has_membership_item_at(Membership::Invite, position));

            let has_ban_row = self.has_membership_item_at(Membership::Ban, position);
            if is_empty && has_ban_row {
                self.extra_joined_items().remove(position);
            } else if !is_empty && !has_ban_row {
                let invite_item = MembershipSubpageItem::new(
                    Membership::Ban,
                    self.banned.get().expect("banned members are initialized"),
                );
                self.extra_joined_items().insert(position, &invite_item);
            }

            self.obj().notify_banned_is_empty();
        }
    }
}

glib::wrapper! {
    /// The list of room members split into several lists by membership.
    pub struct MembershipLists(ObjectSubclass<imp::MembershipLists>);
}

impl MembershipLists {
    /// Construct a new empty `MembershipLists`.
    pub fn new() -> Self {
        glib::Object::new()
    }
}

impl Default for MembershipLists {
    fn default() -> Self {
        Self::new()
    }
}

fn build_filtered_list(model: impl IsA<gio::ListModel>, state: Membership) -> gio::ListModel {
    let membership_expression = Member::this_expression("membership").chain_closure::<bool>(
        closure!(|_: Option<glib::Object>, this_state: Membership| this_state == state),
    );

    let membership_filter = gtk::BoolFilter::new(Some(&membership_expression));

    let filter_model = gtk::FilterListModel::new(Some(model), Some(membership_filter));
    filter_model.upcast()
}
