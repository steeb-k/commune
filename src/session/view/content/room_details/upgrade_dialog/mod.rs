use adw::{prelude::*, subclass::prelude::*};
use futures_channel::oneshot;
use gettextrs::gettext;
use gtk::{gio, glib, pango};
use ruma::{
    RoomVersionId,
    api::client::discovery::get_capabilities::v3::{RoomVersionStability, RoomVersionsCapability},
};
use tracing::error;

mod room_version;

use self::room_version::RoomVersion;

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_details/upgrade_dialog/mod.ui"
    )]
    pub struct UpgradeDialog {
        #[template_child]
        version_combo: TemplateChild<adw::ComboRow>,
        /// The sender for the response of the user.
        sender: RefCell<Option<oneshot::Sender<Option<RoomVersionId>>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for UpgradeDialog {
        const NAME: &'static str = "RoomDetailsUpgradeDialog";
        type Type = super::UpgradeDialog;
        type ParentType = adw::Dialog;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for UpgradeDialog {
        fn constructed(&self) {
            self.parent_constructed();

            self.version_combo
                .set_expression(Some(RoomVersion::this_expression("id-string")));

            // Construct the header factory to separate stable from experimental versions.
            let header_factory = gtk::SignalListItemFactory::new();
            header_factory.connect_setup(|_, header| {
                let Some(header) = header.downcast_ref::<gtk::ListHeader>() else {
                    error!("List item factory did not receive a list header: {header:?}");
                    return;
                };

                let label = gtk::Label::builder()
                    .margin_start(12)
                    .xalign(0.0)
                    .ellipsize(pango::EllipsizeMode::End)
                    .css_classes(["heading"])
                    .build();
                header.set_child(Some(&label));
            });
            header_factory.connect_bind(|_, header| {
                let Some(header) = header.downcast_ref::<gtk::ListHeader>() else {
                    error!("List item factory did not receive a list header: {header:?}");
                    return;
                };
                let Some(label) = header.child().and_downcast::<gtk::Label>() else {
                    error!("List header does not have a child GtkLabel");
                    return;
                };
                let Some(version) = header.item().and_downcast::<RoomVersion>() else {
                    error!("List header does not have a RoomVersion item");
                    return;
                };

                let text = match version.stability() {
                    // Translators: As in 'Stable version'.
                    RoomVersionStability::Stable => gettext("Stable"),
                    // Translators: As in 'Experimental version'.
                    _ => gettext("Experimental"),
                };
                label.set_label(&text);
            });
            self.version_combo.set_header_factory(Some(&header_factory));
        }
    }

    impl WidgetImpl for UpgradeDialog {}

    impl AdwDialogImpl for UpgradeDialog {
        fn closed(&self) {
            let Some(sender) = self.sender.take() else {
                return;
            };

            if sender.send(None).is_err() {
                error!("Could not cancel upgrade: receiver was dropped");
            }
        }
    }

    #[gtk::template_callbacks]
    impl UpgradeDialog {
        /// Ask the user to confirm the room upgrade and select a room version
        /// among the ones that are supported by the server.
        ///
        /// Returns the selected room version, or `None` if the user cancelled
        /// the upgrade.
        pub(super) async fn confirm_upgrade(
            &self,
            capability: RoomVersionsCapability,
            parent: &gtk::Widget,
        ) -> Option<RoomVersionId> {
            self.update_version_combo(capability);

            let (sender, receiver) = oneshot::channel();
            self.sender.replace(Some(sender));

            self.obj().present(Some(parent));
            receiver
                .await
                .expect("sender should not have been dropped prematurely")
        }

        /// Update the room versions combo row with the given capability.
        fn update_version_combo(&self, capability: RoomVersionsCapability) {
            // Build the lists.
            let default = capability.default;
            let (mut stable_list, mut experimental_list) = capability
                .available
                .into_iter()
                .map(|(id, stability)| {
                    let stability = if id == default {
                        // According to the spec, the default version is always assumed to be
                        // stable.
                        RoomVersionStability::Stable
                    } else {
                        stability
                    };

                    RoomVersion::new(id, stability)
                })
                .partition::<Vec<_>, _>(|version| {
                    *version.stability() == RoomVersionStability::Stable
                });

            stable_list.sort_unstable_by(RoomVersion::cmp_ids);
            experimental_list.sort_unstable_by(RoomVersion::cmp_ids);

            let default_pos = stable_list
                .iter()
                .position(|v| *v.id() == default)
                .unwrap_or_default();

            // Construct the list models for the combo row.
            let stable_model = stable_list.into_iter().collect::<gio::ListStore>();
            let experimental_model = experimental_list.into_iter().collect::<gio::ListStore>();

            let model_list = gio::ListStore::new::<gio::ListStore>();
            model_list.append(&stable_model);
            model_list.append(&experimental_model);
            let flatten_model = gtk::FlattenListModel::new(Some(model_list));

            self.version_combo.set_model(Some(&flatten_model));
            self.version_combo
                .set_selected(default_pos.try_into().unwrap_or(u32::MAX));
        }

        /// Confirm the upgrade.
        #[template_callback]
        fn upgrade(&self) {
            let Some(sender) = self.sender.take() else {
                error!("Could not confirm upgrade: response was already sent");
                return;
            };

            let room_version = self
                .version_combo
                .selected_item()
                .and_downcast::<RoomVersion>()
                .map(|v| v.id().clone());

            if sender.send(room_version).is_err() {
                error!("Could not confirm upgrade: receiver was dropped");
            }

            self.obj().close();
        }

        /// Cancel the upgrade.
        #[template_callback]
        fn cancel(&self) {
            self.obj().close();
        }
    }
}

glib::wrapper! {
    /// Dialog to confirm a room upgrade and select a room version.
    pub struct UpgradeDialog(ObjectSubclass<imp::UpgradeDialog>)
        @extends gtk::Widget, adw::Dialog, @implements gtk::Accessible;
}

impl UpgradeDialog {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// Ask the user to confirm the room upgrade and select a room version among
    /// the ones that are supported by the server.
    ///
    /// Returns the selected room version, or `None` if the user cancelled the
    /// upgrade.
    pub(crate) async fn confirm_upgrade(
        &self,
        capability: RoomVersionsCapability,
        parent: &impl IsA<gtk::Widget>,
    ) -> Option<RoomVersionId> {
        self.imp()
            .confirm_upgrade(capability, parent.upcast_ref())
            .await
    }
}
