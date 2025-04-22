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
        #[property(get)]
        extra_joined_items: gtk::FilterListModel,
        /// The filter of the list of extra items in the joined list.
        extra_joined_items_filter: gtk::CustomFilter,
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
        /// The row presented when the list is loading or an error occurred.
        loading_row: LoadingRow,
        /// Whether the loading row is visible.
        #[property(get)]
        is_loading_row_visible: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MembershipLists {
        const NAME: &'static str = "ContentMembershipLists";
        type Type = super::MembershipLists;
    }

    #[glib::derived_properties]
    impl ObjectImpl for MembershipLists {
        fn constructed(&self) {
            self.parent_constructed();

            self.extra_joined_items_filter.set_filter_func(clone!(
                #[weak(rename_to = imp)]
                self,
                #[upgrade_or]
                false,
                move |item| {
                    if *item == imp.loading_row {
                        return imp.is_loading_row_visible.get();
                    }

                    if let Some(subpage_item) = item.downcast_ref::<MembershipSubpageItem>() {
                        let visible = match subpage_item.membership() {
                            Membership::Invite => !imp.invited_is_empty.get(),
                            Membership::Ban => !imp.banned_is_empty.get(),
                            _ => false,
                        };

                        return visible;
                    }

                    false
                }
            ));
            self.extra_joined_items
                .set_filter(Some(&self.extra_joined_items_filter));

            self.loading_row.connect_retry(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.members.obj().reload();
                }
            ));
        }
    }

    impl MembershipLists {
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
            self.update_invited();

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
            self.update_banned();

            let extra_joined_items_base = gio::ListStore::new::<glib::Object>();
            extra_joined_items_base.append(&self.loading_row);
            extra_joined_items_base
                .append(&MembershipSubpageItem::new(Membership::Invite, invited));
            extra_joined_items_base.append(&MembershipSubpageItem::new(Membership::Ban, banned));
            self.extra_joined_items
                .set_model(Some(&extra_joined_items_base));

            let model_list = gio::ListStore::new::<gio::ListModel>();
            model_list.append(&self.extra_joined_items);
            model_list.append(joined);
            self.joined_full
                .set(gtk::FlattenListModel::new(Some(model_list)).upcast())
                .expect("full list for joined members should be uninitialized");
        }

        /// Update the extra joined items list for the given loading state.
        fn update_loading_state(&self, state: LoadingState) {
            let error = (state == LoadingState::Error)
                .then(|| gettext("Could not load the full list of room members"));
            self.loading_row.set_error(error.as_deref());

            let is_row_visible = state != LoadingState::Ready;
            if self.is_loading_row_visible.get() != is_row_visible {
                self.is_loading_row_visible.set(is_row_visible);
                self.extra_joined_items_filter
                    .changed(gtk::FilterChange::Different);
                self.obj().notify_is_loading_row_visible();
            }
        }

        /// Update the extra joined items list for the invited members.
        fn update_invited(&self) {
            let invited = self
                .invited
                .get()
                .expect("invited members should be initialized");
            let is_empty = invited.n_items() == 0;

            if self.invited_is_empty.get() != is_empty {
                self.invited_is_empty.set(is_empty);
                self.extra_joined_items_filter
                    .changed(gtk::FilterChange::Different);
                self.obj().notify_invited_is_empty();
            }
        }

        /// Update the extra joined items list for the banned members.
        fn update_banned(&self) {
            let banned = self
                .banned
                .get()
                .expect("banned members should be initialized");
            let is_empty = banned.n_items() == 0;

            if self.banned_is_empty.get() != is_empty {
                self.banned_is_empty.set(is_empty);
                self.extra_joined_items_filter
                    .changed(gtk::FilterChange::Different);
                self.obj().notify_banned_is_empty();
            }
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

fn build_filtered_list(model: gtk::SortListModel, state: Membership) -> gio::ListModel {
    let membership_expression = Member::this_expression("membership").chain_closure::<bool>(
        closure!(|_: Option<glib::Object>, this_state: Membership| this_state == state),
    );

    let membership_filter = gtk::BoolFilter::new(Some(&membership_expression));

    let filter_model = gtk::FilterListModel::new(Some(model), Some(membership_filter));
    filter_model.upcast()
}
