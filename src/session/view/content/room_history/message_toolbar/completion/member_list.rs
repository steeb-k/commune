use gtk::{
    gio, glib,
    glib::{clone, closure},
    prelude::*,
    subclass::prelude::*,
};

use crate::{
    components::{AtRoom, PillSource},
    session::model::{Member, MemberList, Membership},
    utils::{ExpressionListModel, expression},
};

mod imp {
    use std::{cell::RefCell, marker::PhantomData};

    use super::*;

    #[derive(Debug, glib::Properties)]
    #[properties(wrapper_type = super::CompletionMemberList)]
    pub struct CompletionMemberList {
        /// The room members used for completion.
        #[property(get = Self::members, set = Self::set_members, explicit_notify, nullable)]
        members: PhantomData<Option<MemberList>>,
        /// The members list with expression watches.
        members_expr: ExpressionListModel,
        room_handler: RefCell<Option<glib::SignalHandlerId>>,
        permissions_handler: RefCell<Option<glib::SignalHandlerId>>,
        /// The list model for the `@room` item.
        at_room_model: gio::ListStore,
        /// The search filter.
        search_filter: gtk::StringFilter,
        /// The list of sorted and filtered room members.
        #[property(get)]
        list: gtk::FilterListModel,
    }

    impl Default for CompletionMemberList {
        fn default() -> Self {
            Self {
                members: Default::default(),
                members_expr: Default::default(),
                room_handler: Default::default(),
                permissions_handler: Default::default(),
                at_room_model: gio::ListStore::new::<AtRoom>(),
                search_filter: Default::default(),
                list: Default::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CompletionMemberList {
        const NAME: &'static str = "ContentCompletionMemberList";
        type Type = super::CompletionMemberList;
    }

    #[glib::derived_properties]
    impl ObjectImpl for CompletionMemberList {
        fn constructed(&self) {
            self.parent_constructed();

            // Filter the members, the criteria:
            // - not our user
            // - not ignored
            // - joined
            let not_own_user = gtk::BoolFilter::builder()
                .expression(expression::not(Member::this_expression("is-own-user")))
                .build();

            let ignored_expr = Member::this_expression("is-ignored");
            let not_ignored = gtk::BoolFilter::builder()
                .expression(&ignored_expr)
                .invert(true)
                .build();

            let joined_expr = Member::this_expression("membership").chain_closure::<bool>(
                closure!(|_obj: Option<glib::Object>, membership: Membership| {
                    membership == Membership::Join
                }),
            );
            let joined = gtk::BoolFilter::new(Some(&joined_expr));

            let filter = gtk::EveryFilter::new();
            filter.append(not_own_user);
            filter.append(not_ignored);
            filter.append(joined);

            let first_model = gtk::FilterListModel::builder()
                .filter(&filter)
                .model(&self.members_expr)
                .build();

            // Sort the members list by activity, then display name.
            let latest_activity_expr = Member::this_expression("latest-activity");
            let activity = gtk::NumericSorter::builder()
                .sort_order(gtk::SortType::Descending)
                .expression(&latest_activity_expr)
                .build();

            let display_name_expr = Member::this_expression("display-name");
            let display_name = gtk::StringSorter::builder()
                .ignore_case(true)
                .expression(&display_name_expr)
                .build();

            let sorter = gtk::MultiSorter::new();
            sorter.append(activity);
            sorter.append(display_name);
            let sorted_members_model = gtk::SortListModel::builder()
                .sorter(&sorter)
                .model(&first_model)
                .build();

            // Add `@room` model.
            let models_list = gio::ListStore::new::<gio::ListModel>();
            models_list.append(&self.at_room_model);
            models_list.append(&sorted_members_model);
            let flatten_model = gtk::FlattenListModel::new(Some(models_list));

            // Setup the search filter.
            let item_search_string_expr = gtk::ClosureExpression::new::<String>(
                &[
                    PillSource::this_expression("identifier"),
                    PillSource::this_expression("display-name"),
                ],
                closure!(
                    |_: Option<glib::Object>, identifier: &str, display_name: &str| {
                        format!("{display_name} {identifier}")
                    }
                ),
            );
            self.search_filter.set_ignore_case(true);
            self.search_filter
                .set_match_mode(gtk::StringFilterMatchMode::Substring);
            self.search_filter
                .set_expression(Some(expression::normalize_string(item_search_string_expr)));

            self.list.set_filter(Some(&self.search_filter));
            self.list.set_model(Some(&flatten_model));

            self.members_expr.set_expressions(vec![
                ignored_expr.upcast(),
                joined_expr.upcast(),
                latest_activity_expr.upcast(),
                display_name_expr.upcast(),
            ]);
        }

        fn dispose(&self) {
            if let Some(room) = self.members().and_then(|m| m.room()) {
                if let Some(handler) = self.room_handler.take() {
                    room.disconnect(handler);
                }
                if let Some(handler) = self.permissions_handler.take() {
                    room.permissions().disconnect(handler);
                }
            }
        }
    }

    impl CompletionMemberList {
        /// The room members used for completion.
        fn members(&self) -> Option<MemberList> {
            self.members_expr.model().and_downcast()
        }

        /// Set the room members used for completion.
        fn set_members(&self, members: Option<MemberList>) {
            let prev_members = self.members();

            if prev_members == members {
                return;
            }

            if let Some(room) = prev_members.and_then(|m| m.room()) {
                if let Some(handler) = self.room_handler.take() {
                    room.disconnect(handler);
                }
                if let Some(handler) = self.permissions_handler.take() {
                    room.permissions().disconnect(handler);
                }
            }

            if let Some(room) = members.as_ref().and_then(MemberList::room) {
                let room_handler = room.connect_is_direct_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_at_room_model();
                    }
                ));
                self.room_handler.replace(Some(room_handler));

                let permissions_handler =
                    room.permissions().connect_can_notify_room_notify(clone!(
                        #[weak(rename_to = imp)]
                        self,
                        move |_| {
                            imp.update_at_room_model();
                        }
                    ));
                self.permissions_handler.replace(Some(permissions_handler));
            }

            self.members_expr.set_model(members);
            self.update_at_room_model();
            self.obj().notify_members();
        }

        /// Update whether `@room` should be present in the suggestions.
        fn update_at_room_model(&self) {
            // Only present `@room` if it's not a DM and user can notify the room.
            let room = self
                .members()
                .and_then(|m| m.room())
                .filter(|r| !r.is_direct() && r.permissions().can_notify_room());

            if let Some(room) = room {
                if let Some(at_room) = self.at_room_model.item(0).and_downcast::<AtRoom>() {
                    if at_room.room_id() == room.room_id() {
                        return;
                    }

                    self.at_room_model.remove(0);
                }

                self.at_room_model.append(&room.at_room());
            } else if self.at_room_model.n_items() > 0 {
                self.at_room_model.remove(0);
            }
        }

        /// Set the search term.
        pub(super) fn set_search_term(&self, term: Option<&str>) {
            self.search_filter.set_search(term);
        }
    }
}

glib::wrapper! {
    /// The filtered and sorted members list for completion.
    ///
    /// Also includes an `@room` item for notifying the whole room.
    pub struct CompletionMemberList(ObjectSubclass<imp::CompletionMemberList>);
}

impl CompletionMemberList {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Set the search term.
    pub(crate) fn set_search_term(&self, term: Option<&str>) {
        self.imp().set_search_term(term);
    }
}

impl Default for CompletionMemberList {
    fn default() -> Self {
        Self::new()
    }
}
