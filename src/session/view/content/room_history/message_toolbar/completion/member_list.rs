use gtk::{glib, glib::closure, prelude::*, subclass::prelude::*};

use crate::{
    session::model::{Member, MemberList, Membership},
    utils::{expression, ExpressionListModel},
};

mod imp {
    use std::marker::PhantomData;

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::CompletionMemberList)]
    pub struct CompletionMemberList {
        /// The room members used for completion.
        #[property(get = Self::members, set = Self::set_members, explicit_notify, nullable)]
        members: PhantomData<Option<MemberList>>,
        /// The members list with expression watches.
        pub members_expr: ExpressionListModel,
        /// The search filter.
        pub search_filter: gtk::StringFilter,
        /// The list of sorted and filtered room members.
        #[property(get)]
        pub list: gtk::FilterListModel,
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
            let second_model = gtk::SortListModel::builder()
                .sorter(&sorter)
                .model(&first_model)
                .build();

            // Setup the search filter.
            let member_search_string_expr = gtk::ClosureExpression::new::<String>(
                &[
                    Member::this_expression("user-id-string"),
                    Member::this_expression("display-name"),
                ],
                closure!(
                    |_: Option<glib::Object>, user_id: &str, display_name: &str| {
                        format!("{display_name} {user_id}")
                    }
                ),
            );
            self.search_filter.set_ignore_case(true);
            self.search_filter
                .set_match_mode(gtk::StringFilterMatchMode::Substring);
            self.search_filter
                .set_expression(Some(expression::normalize_string(
                    member_search_string_expr,
                )));

            self.list.set_filter(Some(&self.search_filter));
            self.list.set_model(Some(&second_model));

            self.members_expr.set_expressions(vec![
                ignored_expr.upcast(),
                joined_expr.upcast(),
                latest_activity_expr.upcast(),
                display_name_expr.upcast(),
            ]);
        }
    }

    impl WidgetImpl for CompletionMemberList {}
    impl PopoverImpl for CompletionMemberList {}

    impl CompletionMemberList {
        /// The room members used for completion.
        fn members(&self) -> Option<MemberList> {
            self.members_expr.model().and_downcast()
        }

        /// Set the room members used for completion.
        fn set_members(&self, members: Option<MemberList>) {
            if self.members() == members {
                return;
            }

            self.members_expr.set_model(members.and_upcast());
            self.obj().notify_members();
        }
    }
}

glib::wrapper! {
    /// The filtered and sorted members list for completion.
    pub struct CompletionMemberList(ObjectSubclass<imp::CompletionMemberList>);
}

impl CompletionMemberList {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Set the search term.
    pub fn set_search_term(&self, term: Option<&str>) {
        self.imp().search_filter.set_search(term);
    }
}

impl Default for CompletionMemberList {
    fn default() -> Self {
        Self::new()
    }
}
