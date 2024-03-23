use gtk::{glib, glib::clone, prelude::*, subclass::prelude::*, CompositeTemplate};

use super::MemberPowerLevel;
use crate::{
    components::{Avatar, PowerLevelSelectionPopover, RoleBadge},
    session::model::Permissions,
    utils::{add_activate_binding_action, BoundObject},
};

mod imp {
    use std::cell::OnceCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, CompositeTemplate, glib::Properties)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_details/permissions/member_row.ui"
    )]
    #[properties(wrapper_type = super::PermissionsMemberRow)]
    pub struct PermissionsMemberRow {
        #[template_child]
        selected_level_label: TemplateChild<gtk::Label>,
        #[template_child]
        arrow_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub popover: TemplateChild<PowerLevelSelectionPopover>,
        /// The permissions of the room.
        #[property(get, set = Self::set_permissions, construct_only)]
        pub permissions: OnceCell<Permissions>,
        /// The room member presented by this row.
        #[property(get, set = Self::set_member, explicit_notify, nullable)]
        pub member: BoundObject<MemberPowerLevel>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PermissionsMemberRow {
        const NAME: &'static str = "RoomDetailsPermissionsMemberRow";
        type Type = super::PermissionsMemberRow;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            Avatar::ensure_type();
            RoleBadge::ensure_type();

            Self::bind_template(klass);
            Self::Type::bind_template_callbacks(klass);

            klass.set_css_name("permissions-member-row");

            klass.install_action("permissions-member.activate", None, |obj, _, _| {
                obj.activate_row();
            });

            add_activate_binding_action(klass, "permissions-member.activate");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for PermissionsMemberRow {}

    impl WidgetImpl for PermissionsMemberRow {}
    impl BoxImpl for PermissionsMemberRow {}

    impl PermissionsMemberRow {
        /// Set the permissions of the room.
        fn set_permissions(&self, permissions: Permissions) {
            self.permissions.set(permissions.clone()).unwrap();
            self.popover.set_permissions(Some(permissions));
        }

        /// Set the member displayed by this row.
        fn set_member(&self, member: Option<MemberPowerLevel>) {
            if self.member.obj() == member {
                return;
            }

            self.member.disconnect_signals();

            if let Some(member) = member {
                let power_level_handler =
                    member.connect_power_level_notify(clone!(@weak self as imp => move |_| {
                        imp.update_power_level();
                    }));
                let editable_handler =
                    member.connect_editable_notify(clone!(@weak self as imp => move |_| {
                        imp.update_accessible_role();
                    }));

                self.member
                    .set(member, vec![power_level_handler, editable_handler]);
                self.update_power_level();
                self.update_accessible_role();
            }

            self.obj().notify_member();
        }

        /// Update the power level label.
        fn update_power_level(&self) {
            let Some(member) = self.member.obj() else {
                return;
            };

            self.selected_level_label
                .set_label(&member.power_level().to_string());
        }

        /// Update the accessible role of this row.
        fn update_accessible_role(&self) {
            let Some(member) = self.member.obj() else {
                return;
            };

            let editable = member.editable();

            let role = if editable {
                gtk::AccessibleRole::ComboBox
            } else {
                gtk::AccessibleRole::ListItem
            };
            self.obj().set_accessible_role(role);

            self.arrow_box.set_opacity(editable.into());
        }
    }
}

glib::wrapper! {
    /// A row presenting a room member's permission and allowing optionally to edit it.
    pub struct PermissionsMemberRow(ObjectSubclass<imp::PermissionsMemberRow>)
        @extends gtk::Widget, gtk::Box, @implements gtk::Accessible;
}

#[gtk::template_callbacks]
impl PermissionsMemberRow {
    pub fn new(permissions: &Permissions) -> Self {
        glib::Object::builder()
            .property("permissions", permissions)
            .build()
    }

    /// The row was activated.
    #[template_callback]
    fn activate_row(&self) {
        let Some(member) = self.member() else {
            return;
        };

        if member.editable() {
            self.imp().popover.popup();
        }
    }

    /// The popover's visibility changed.
    #[template_callback]
    fn popover_visible(&self) {
        let is_visible = self.imp().popover.is_visible();

        if is_visible {
            self.add_css_class("has-open-popup");
        } else {
            self.remove_css_class("has-open-popup");
        }
    }

    /// The popover's selected power level changed.
    #[template_callback]
    fn power_level_changed(&self) {
        let Some(member) = self.member() else {
            return;
        };

        let pl = self.imp().popover.selected_power_level();
        member.set_power_level(pl);
    }
}
