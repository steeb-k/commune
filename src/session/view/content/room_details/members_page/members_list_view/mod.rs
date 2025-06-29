use adw::{prelude::*, subclass::prelude::*};
use gettextrs::{gettext, ngettext};
use gtk::{
    CompositeTemplate, gio, glib,
    glib::{clone, closure},
};

mod item_row;
mod membership_subpage_row;

use self::{item_row::ItemRow, membership_subpage_row::MembershipSubpageRow};
use super::membership_as_tag;
use crate::{
    prelude::*,
    session::{
        model::{Member, Membership, Room},
        view::content::room_details::{MembershipLists, MembershipSubpageItem},
    },
    utils::{LoadingState, expression},
};

mod imp {
    use std::cell::{Cell, RefCell};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_details/members_page/members_list_view/mod.ui"
    )]
    #[properties(wrapper_type = super::MembersListView)]
    pub struct MembersListView {
        #[template_child]
        search_button: TemplateChild<gtk::ToggleButton>,
        #[template_child]
        search_bar: TemplateChild<gtk::SearchBar>,
        #[template_child]
        search_entry: TemplateChild<gtk::SearchEntry>,
        #[template_child]
        stack: TemplateChild<gtk::Stack>,
        #[template_child]
        empty_stack_page: TemplateChild<gtk::StackPage>,
        #[template_child]
        empty_page: TemplateChild<adw::StatusPage>,
        #[template_child]
        empty_listbox: TemplateChild<gtk::ListBox>,
        #[template_child]
        members_stack_page: TemplateChild<gtk::StackPage>,
        #[template_child]
        list_view: TemplateChild<gtk::ListView>,
        /// The room containing the members to present.
        #[property(get, set = Self::set_room, construct_only)]
        room: glib::WeakRef<Room>,
        /// The lists of members filtered by membership for the room.
        #[property(get, set = Self::set_membership_lists, construct_only)]
        membership_lists: glib::WeakRef<MembershipLists>,
        /// The model with the search filter.
        filtered_model: gtk::FilterListModel,
        /// The membership used to filter the model.
        #[property(get, set = Self::set_membership, construct_only, builder(Membership::default()))]
        membership: Cell<Membership>,
        /// Whether our own user can send an invite in the current room.
        #[property(get, set = Self::set_can_invite, explicit_notify)]
        can_invite: Cell<bool>,
        members_state_handler: RefCell<Option<glib::SignalHandlerId>>,
        items_changed_handler: RefCell<Option<glib::SignalHandlerId>>,
        extra_items_changed_handler: RefCell<Option<glib::SignalHandlerId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MembersListView {
        const NAME: &'static str = "ContentMembersListView";
        type Type = super::MembersListView;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            ItemRow::ensure_type();

            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);

            klass.set_css_name("members-list");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for MembersListView {
        fn constructed(&self) {
            self.parent_constructed();

            // Needed because the GtkSearchEntry is not the direct child of the
            // GtkSearchBear.
            self.search_bar.connect_entry(&*self.search_entry);

            let member_expr = gtk::ClosureExpression::new::<String>(
                &[] as &[gtk::Expression],
                closure!(|item: Option<glib::Object>| {
                    item.and_downcast_ref()
                        .map(Member::search_string)
                        .unwrap_or_default()
                }),
            );
            let search_filter = gtk::StringFilter::builder()
                .match_mode(gtk::StringFilterMatchMode::Substring)
                .expression(expression::normalize_string(member_expr))
                .ignore_case(true)
                .build();

            expression::normalize_string(self.search_entry.property_expression("text")).bind(
                &search_filter,
                "search",
                None::<&glib::Object>,
            );

            self.filtered_model.set_filter(Some(&search_filter));
            self.list_view.set_model(Some(&gtk::NoSelection::new(Some(
                self.filtered_model.clone(),
            ))));

            self.init_members_list();
        }

        fn dispose(&self) {
            if let Some(membership_lists) = self.membership_lists.upgrade() {
                if let Some(handler) = self.members_state_handler.take() {
                    membership_lists.members().disconnect(handler);
                }
                if let Some(handler) = self.items_changed_handler.take() {
                    self.members_only_model(&membership_lists)
                        .disconnect(handler);
                }
                if let Some(handler) = self.extra_items_changed_handler.take() {
                    if let Some(model) = self.extra_items_model(&membership_lists) {
                        model.disconnect(handler);
                    }
                }
            }
        }
    }

    impl WidgetImpl for MembersListView {}
    impl NavigationPageImpl for MembersListView {}

    #[gtk::template_callbacks]
    impl MembersListView {
        /// Set the room containing the members to present.
        fn set_room(&self, room: &Room) {
            self.room.set(Some(room));

            // Show the invite button when we can invite but it is not a direct room.
            let can_invite_expr = room.permissions().property_expression("can-invite");
            let is_direct_expr = room.property_expression("is-direct");
            expression::and(can_invite_expr, expression::not(is_direct_expr)).bind(
                &*self.obj(),
                "can-invite",
                None::<&glib::Object>,
            );
        }

        /// Set the room containing the members to present.
        fn set_membership_lists(&self, membership_lists: &MembershipLists) {
            self.membership_lists.set(Some(membership_lists));

            let members_state_handler = membership_lists.members().connect_state_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.update_view();
                }
            ));
            self.members_state_handler
                .replace(Some(members_state_handler));
        }

        /// Set the membership used to filter the model.
        fn set_membership(&self, membership: Membership) {
            self.membership.set(membership);
            self.obj().set_tag(Some(membership_as_tag(membership)));
            self.update_empty_page();
        }

        /// Set whether our own user can send an invite in the current room.
        fn set_can_invite(&self, can_invite: bool) {
            if self.can_invite.get() == can_invite {
                return;
            }

            self.can_invite.set(can_invite);
            self.obj().notify_can_invite();
        }

        /// The full list model from the given lists of members used by the list
        /// view.
        fn full_model(&self, membership_lists: &MembershipLists) -> gio::ListModel {
            match self.membership.get() {
                Membership::Invite => membership_lists.invited(),
                Membership::Ban => membership_lists.banned(),
                _ => membership_lists.joined_full(),
            }
        }

        /// The list model from the given lists of members containing only
        /// members.
        fn members_only_model(&self, membership_lists: &MembershipLists) -> gio::ListModel {
            match self.membership.get() {
                Membership::Invite => membership_lists.invited(),
                Membership::Ban => membership_lists.banned(),
                _ => membership_lists.joined(),
            }
        }

        /// The list model from the given lists of members containing extra
        /// items.
        fn extra_items_model(&self, membership_lists: &MembershipLists) -> Option<gio::ListModel> {
            match self.membership.get() {
                Membership::Invite | Membership::Ban => None,
                _ => Some(membership_lists.extra_joined_items().upcast()),
            }
        }

        /// Initialize the members list used for this view.
        fn init_members_list(&self) {
            let Some(membership_lists) = self.membership_lists.upgrade() else {
                return;
            };

            let full_model = self.full_model(&membership_lists);
            self.filtered_model.set_model(Some(&full_model));

            let members_only_model = self.members_only_model(&membership_lists);
            let items_changed_handler = members_only_model.connect_items_changed(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_, _, _, _| {
                    imp.update_view();
                }
            ));
            self.items_changed_handler
                .replace(Some(items_changed_handler));

            if let Some(extra_items_model) = self.extra_items_model(&membership_lists) {
                let extra_items_changed_handler = extra_items_model.connect_items_changed(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_, _, _, _| {
                        imp.update_empty_listbox();
                    }
                ));
                self.extra_items_changed_handler
                    .replace(Some(extra_items_changed_handler));
            }

            self.update_view();
            self.update_empty_listbox();
        }

        /// Update the view for the current state.
        fn update_view(&self) {
            let Some(membership_lists) = self.membership_lists.upgrade() else {
                self.stack.set_visible_child_name("no-members");
                return;
            };

            let model = self.members_only_model(&membership_lists);
            let count = model.n_items();
            let is_empty = count == 0;
            let membership = self.membership.get();

            let title = match membership {
                Membership::Invite => {
                    ngettext("Invited Room Member", "Invited Room Members", count)
                }
                Membership::Ban => ngettext("Banned Room Member", "Banned Room Members", count),
                _ => ngettext("Room Member", "Room Members", count),
            };

            self.obj().set_title(&title);
            self.members_stack_page.set_title(&title);

            let (visible_page, extra_items_model) = if is_empty {
                match membership_lists.members().state() {
                    LoadingState::Initial | LoadingState::Loading => ("loading", None),
                    LoadingState::Error => ("error", None),
                    LoadingState::Ready => {
                        let extra_items_model = self.extra_items_model(&membership_lists);
                        ("empty", extra_items_model)
                    }
                }
            } else {
                ("members", None)
            };

            self.empty_listbox
                .bind_model(extra_items_model.as_ref(), |item| {
                    let row = MembershipSubpageRow::new();
                    row.set_item(item.downcast_ref::<MembershipSubpageItem>().cloned());

                    row.upcast()
                });

            // Hide the search button and bar if the list is empty, since there is no search
            // possible.
            self.search_button.set_visible(!is_empty);
            self.search_bar.set_visible(!is_empty);

            self.stack.set_visible_child_name(visible_page);
        }

        /// Update the "empty" page for the current state.
        fn update_empty_page(&self) {
            let membership = self.membership.get();

            let (title, description) = match membership {
                Membership::Invite => {
                    let title = gettext("No Invited Room Members");
                    let description = gettext("There are no invited members in this room");
                    (title, description)
                }
                Membership::Ban => {
                    let title = gettext("No Banned Room Members");
                    let description = gettext("There are no banned members in this room");
                    (title, description)
                }
                _ => {
                    let title = gettext("No Room Members");
                    let description = gettext("There are no members in this room");
                    (title, description)
                }
            };

            self.empty_stack_page.set_title(&title);
            self.empty_page.set_title(&title);
            self.empty_page.set_description(Some(&description));
            self.empty_page.set_icon_name(Some(membership.icon_name()));
        }

        /// Update the `GtkListBox` of the "empty" page for the current state.
        fn update_empty_listbox(&self) {
            let has_extra_items = self
                .membership_lists
                .upgrade()
                .and_then(|membership_lists| self.extra_items_model(&membership_lists))
                .is_some_and(|model| model.n_items() > 0);
            self.empty_listbox.set_visible(has_extra_items);
        }

        /// Activate the row of the members `GtkListView` at the given position.
        #[template_callback]
        fn activate_listview_row(&self, pos: u32) {
            let Some(item) = self.filtered_model.item(pos) else {
                return;
            };
            let obj = self.obj();

            if let Some(member) = item.downcast_ref::<Member>() {
                obj.activate_action(
                    "details.show-member",
                    Some(&member.user_id().as_str().to_variant()),
                )
                .expect("action exists");
            } else if let Some(item) = item.downcast_ref::<MembershipSubpageItem>() {
                obj.activate_action(
                    "members.show-membership-list",
                    Some(&item.membership().to_variant()),
                )
                .expect("action exists");
            }
        }

        /// Activate the given row from the `GtkListBox`.
        #[template_callback]
        fn activate_listbox_row(&self, row: &gtk::ListBoxRow) {
            let row = row
                .downcast_ref::<MembershipSubpageRow>()
                .expect("list box contains only membership subpage rows");

            let Some(item) = row.item() else {
                return;
            };

            self.obj()
                .activate_action(
                    "members.show-membership-list",
                    Some(&item.membership().to_variant()),
                )
                .expect("action exists");
        }

        /// Reload the list of members of the room.
        #[template_callback]
        fn reload_members(&self) {
            let Some(membership_lists) = self.membership_lists.upgrade() else {
                return;
            };

            membership_lists.members().reload();
        }
    }
}

glib::wrapper! {
    /// A page to display a list of members.
    pub struct MembersListView(ObjectSubclass<imp::MembersListView>)
        @extends gtk::Widget, adw::NavigationPage;
}

impl MembersListView {
    /// Construct a new `MembersListView` with the given room, membership lists
    /// and membership.
    pub fn new(room: &Room, membership_lists: &MembershipLists, membership: Membership) -> Self {
        glib::Object::builder()
            .property("room", room)
            .property("membership-lists", membership_lists)
            .property("membership", membership)
            .build()
    }
}
