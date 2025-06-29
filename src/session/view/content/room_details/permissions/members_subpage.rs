use adw::{prelude::*, subclass::prelude::*};
use gtk::{
    CompositeTemplate, glib,
    glib::{clone, closure},
};
use tracing::error;

use super::{MemberPowerLevel, PermissionsMemberRow, PrivilegedMembers};
use crate::{session::model::User, utils::expression};

mod imp {
    use std::{cell::Cell, marker::PhantomData};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_details/permissions/members_subpage.ui"
    )]
    #[properties(wrapper_type = super::PermissionsMembersSubpage)]
    pub struct PermissionsMembersSubpage {
        #[template_child]
        search_bar: TemplateChild<gtk::SearchBar>,
        #[template_child]
        search_entry: TemplateChild<gtk::SearchEntry>,
        #[template_child]
        list_view: TemplateChild<gtk::ListView>,
        filtered_model: gtk::FilterListModel,
        /// The list used for this view.
        #[property(get = Self::list, set = Self::set_list, explicit_notify, nullable)]
        list: PhantomData<Option<PrivilegedMembers>>,
        /// Whether our own user can change the power levels in this room.
        #[property(get, set = Self::set_editable, explicit_notify)]
        editable: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PermissionsMembersSubpage {
        const NAME: &'static str = "RoomDetailsPermissionsMembersSubpage";
        type Type = super::PermissionsMembersSubpage;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for PermissionsMembersSubpage {
        fn constructed(&self) {
            self.parent_constructed();

            // Needed because the GtkSearchEntry is not the direct child of the
            // GtkSearchBar.
            self.search_bar.connect_entry(&*self.search_entry);

            let user_expr = gtk::ClosureExpression::new::<String>(
                &[] as &[gtk::Expression],
                closure!(|item: Option<glib::Object>| {
                    item.and_downcast_ref()
                        .map(MemberPowerLevel::search_string)
                        .unwrap_or_default()
                }),
            );
            let search_filter = gtk::StringFilter::builder()
                .match_mode(gtk::StringFilterMatchMode::Substring)
                .expression(expression::normalize_string(user_expr))
                .ignore_case(true)
                .build();

            expression::normalize_string(self.search_entry.property_expression("text")).bind(
                &search_filter,
                "search",
                None::<&glib::Object>,
            );

            self.filtered_model.set_filter(Some(&search_filter));

            // Sort members by power level, then display name, then user ID.
            let power_level_expr = MemberPowerLevel::this_expression("power-level");
            let power_level_sorter = gtk::NumericSorter::builder()
                .expression(power_level_expr)
                .sort_order(gtk::SortType::Descending)
                .build();

            let display_name_expr =
                MemberPowerLevel::this_expression("user").chain_property::<User>("display-name");
            let display_name_sorter = gtk::StringSorter::new(Some(display_name_expr));

            let user_id_expr =
                MemberPowerLevel::this_expression("user").chain_property::<User>("user-id-string");
            let user_id_sorter = gtk::StringSorter::new(Some(user_id_expr));

            let sorter = gtk::MultiSorter::new();
            sorter.append(power_level_sorter);
            sorter.append(display_name_sorter);
            sorter.append(user_id_sorter);

            let sorted_model =
                gtk::SortListModel::new(Some(self.filtered_model.clone()), Some(sorter));

            self.list_view
                .set_model(Some(&gtk::NoSelection::new(Some(sorted_model))));

            let factory = gtk::SignalListItemFactory::new();
            factory.connect_setup(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_, item| {
                    let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                        error!("List item factory did not receive a list item: {item:?}");
                        return;
                    };
                    let Some(permissions) = imp.list().and_then(|l| l.permissions()) else {
                        return;
                    };
                    let row = PermissionsMemberRow::new(&permissions);
                    item.set_child(Some(&row));
                    item.bind_property("item", &row, "member")
                        .sync_create()
                        .build();
                    item.set_activatable(false);
                    item.set_selectable(false);
                }
            ));
            self.list_view.set_factory(Some(&factory));
        }
    }

    impl WidgetImpl for PermissionsMembersSubpage {}
    impl NavigationPageImpl for PermissionsMembersSubpage {}

    impl PermissionsMembersSubpage {
        /// The list used for this view.
        fn list(&self) -> Option<PrivilegedMembers> {
            self.filtered_model.model().and_downcast()
        }

        /// Set the list used for this view.
        fn set_list(&self, list: Option<&PrivilegedMembers>) {
            if self.list().as_ref() == list {
                return;
            }

            self.filtered_model.set_model(list);
            self.obj().notify_list();
        }

        /// Set whether our own user can edit the list.
        fn set_editable(&self, editable: bool) {
            if self.editable.get() == editable {
                return;
            }

            self.editable.set(editable);
            self.obj().notify_editable();
        }
    }
}

glib::wrapper! {
    /// A subpage to see and possibly edit the room members with custom power levels.
    pub struct PermissionsMembersSubpage(ObjectSubclass<imp::PermissionsMembersSubpage>)
        @extends gtk::Widget, adw::NavigationPage;
}

impl PermissionsMembersSubpage {
    pub fn new() -> Self {
        glib::Object::new()
    }
}
