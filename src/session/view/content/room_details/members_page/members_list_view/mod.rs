use adw::{prelude::*, subclass::prelude::*};
use gettextrs::ngettext;
use gtk::{
    gio, glib,
    glib::{clone, closure},
    CompositeTemplate,
};

mod extra_lists;
mod item_row;
mod member_row;
mod membership_subpage_item;
mod membership_subpage_row;

pub use self::{extra_lists::ExtraLists, membership_subpage_item::MembershipSubpageItem};
use self::{
    item_row::ItemRow, member_row::MemberRow, membership_subpage_row::MembershipSubpageRow,
};
use crate::{
    prelude::*,
    session::model::{Member, Membership},
    utils::expression,
};

mod imp {
    use std::{
        cell::{Cell, RefCell},
        marker::PhantomData,
    };

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_details/members_page/members_list_view/mod.ui"
    )]
    #[properties(wrapper_type = super::MembersListView)]
    pub struct MembersListView {
        #[template_child]
        pub search_bar: TemplateChild<gtk::SearchBar>,
        #[template_child]
        pub search_entry: TemplateChild<gtk::SearchEntry>,
        #[template_child]
        pub list_view: TemplateChild<gtk::ListView>,
        pub filtered_model: gtk::FilterListModel,
        /// The model used for this view.
        #[property(get = Self::model, set = Self::set_model, explicit_notify, nullable)]
        pub model: PhantomData<Option<gio::ListModel>>,
        /// Whether our own user can send an invite in the current room.
        #[property(get, set = Self::set_can_invite, explicit_notify)]
        pub can_invite: Cell<bool>,
        items_changed_handler: RefCell<Option<glib::SignalHandlerId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MembersListView {
        const NAME: &'static str = "ContentMembersListView";
        type Type = super::MembersListView;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            ItemRow::ensure_type();

            Self::bind_template(klass);

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
            let obj = self.obj();

            // Needed because the GtkSearchEntry is not the direct child of the
            // GtkSearchBear.
            self.search_bar.connect_entry(&*self.search_entry);

            fn search_string(member: Member) -> String {
                format!(
                    "{} {} {} {}",
                    member.display_name(),
                    member.user_id(),
                    member.role(),
                    member.power_level(),
                )
            }

            let member_expr = gtk::ClosureExpression::new::<String>(
                &[] as &[gtk::Expression],
                closure!(|item: Option<glib::Object>| {
                    item.and_downcast().map(search_string).unwrap_or_default()
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
            self.list_view.connect_activate(clone!(
                #[weak]
                obj,
                move |_, pos| {
                    let Some(item) = obj.imp().filtered_model.item(pos) else {
                        return;
                    };

                    if let Some(member) = item.downcast_ref::<Member>() {
                        obj.activate_action(
                            "members.show-member",
                            Some(&member.user_id().as_str().to_variant()),
                        )
                        .unwrap();
                    } else if let Some(item) = item.downcast_ref::<MembershipSubpageItem>() {
                        obj.activate_action(
                            "members.show-membership-list",
                            Some(&item.state().to_variant()),
                        )
                        .unwrap();
                    }
                }
            ));

            obj.connect_tag_notify(|obj| {
                obj.imp().update_title();
            });
            self.update_title();
        }

        fn dispose(&self) {
            if let Some(model) = self.model() {
                if let Some(handler) = self.items_changed_handler.take() {
                    model.disconnect(handler);
                }
            }
        }
    }

    impl WidgetImpl for MembersListView {}
    impl NavigationPageImpl for MembersListView {}

    impl MembersListView {
        /// The model used for this view.
        fn model(&self) -> Option<gio::ListModel> {
            self.filtered_model.model()
        }

        /// Set the model used for this view.
        fn set_model(&self, model: Option<gio::ListModel>) {
            let prev_model = self.model();

            if prev_model == model {
                return;
            }

            if let Some(model) = prev_model {
                if let Some(handler) = self.items_changed_handler.take() {
                    model.disconnect(handler);
                }
            }

            if let Some(model) = &model {
                let items_changed_handler = model.connect_items_changed(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_, _, _, _| {
                        imp.update_title();
                    }
                ));
                self.items_changed_handler
                    .replace(Some(items_changed_handler));
            }

            self.filtered_model.set_model(model.as_ref());
            self.obj().notify_model();
            self.update_title();
        }

        /// Set whether our own user can send an invite in the current room.
        fn set_can_invite(&self, can_invite: bool) {
            if self.can_invite.get() == can_invite {
                return;
            }

            self.can_invite.set(can_invite);
            self.obj().notify_can_invite();
        }

        /// Update the page title for the current state.
        fn update_title(&self) {
            let Some(model) = self.model() else {
                return;
            };
            let obj = self.obj();
            let Some(tag) = obj.tag() else {
                return;
            };

            let count = model.n_items();
            let title = match &*tag {
                "invited" => ngettext("Invited Room Member", "Invited Room Members", count),
                "banned" => ngettext("Banned Room Member", "Banned Room Members", count),
                _ => ngettext("Room Member", "Room Members", count),
            };

            obj.set_title(&title);
        }
    }
}

glib::wrapper! {
    pub struct MembersListView(ObjectSubclass<imp::MembersListView>)
        @extends gtk::Widget, adw::NavigationPage;
}

impl MembersListView {
    pub fn new(model: &impl IsA<gio::ListModel>, membership: Membership) -> Self {
        let tag = match membership {
            Membership::Invite => "invited",
            Membership::Ban => "banned",
            _ => "joined",
        };

        glib::Object::builder()
            .property("model", model)
            .property("tag", tag)
            .build()
    }
}
