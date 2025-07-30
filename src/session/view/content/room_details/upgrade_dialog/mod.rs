use std::cmp::Ordering;

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
    use std::cell::{OnceCell, RefCell};

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(
        resource = "/org/gnome/Fractal/ui/session/view/content/room_details/upgrade_dialog/mod.ui"
    )]
    pub struct UpgradeDialog {
        #[template_child]
        version_combo: TemplateChild<adw::ComboRow>,
        header_factory: OnceCell<gtk::SignalListItemFactory>,
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
        /// The header factory to separate stable from experimental versions.
        fn header_factory(&self) -> &gtk::SignalListItemFactory {
            self.header_factory.get_or_init(|| {
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

                    let text = if version.is_stable() {
                        // Translators: As in 'Stable version'.
                        gettext("Stable")
                    } else {
                        // Translators: As in 'Experimental version'.
                        gettext("Experimental")
                    };
                    label.set_label(&text);
                });

                header_factory
            })
        }

        /// Ask the user to confirm the room upgrade and select a room version
        /// among the ones that are supported by the server.
        ///
        /// Returns the selected room version, or `None` if the user cancelled
        /// the upgrade.
        pub(super) async fn confirm_upgrade(
            &self,
            info: &UpgradeInfo,
            parent: &gtk::Widget,
        ) -> Option<RoomVersionId> {
            self.update_version_combo(info);

            let (sender, receiver) = oneshot::channel();
            self.sender.replace(Some(sender));

            self.obj().present(Some(parent));
            receiver
                .await
                .expect("sender should not have been dropped prematurely")
        }

        /// Update the room versions combo row with the given details.
        fn update_version_combo(&self, info: &UpgradeInfo) {
            // Construct the list models for the combo row.
            let stable_model = (!info.stable_room_versions.is_empty()).then(|| {
                info.stable_room_versions
                    .iter()
                    .map(|version| RoomVersion::new(version.clone(), true))
                    .collect::<gio::ListStore>()
            });
            let unstable_model = (!info.unstable_room_versions.is_empty()).then(|| {
                info.unstable_room_versions
                    .iter()
                    .map(|version| RoomVersion::new(version.clone(), false))
                    .collect::<gio::ListStore>()
            });

            let use_header_factory = unstable_model.is_some();
            let model = match (stable_model, unstable_model) {
                (Some(model), None) | (None, Some(model)) => model.upcast::<gio::ListModel>(),
                (Some(stable_model), Some(unstable_model)) => {
                    let model_list = gio::ListStore::new::<gio::ListStore>();
                    model_list.append(&stable_model);
                    model_list.append(&unstable_model);
                    gtk::FlattenListModel::new(Some(model_list)).upcast()
                }
                // We always have at least the current room version.
                (None, None) => unreachable!(),
            };

            self.version_combo
                .set_header_factory(use_header_factory.then(|| self.header_factory()));
            self.version_combo.set_model(Some(&model));
            self.version_combo
                .set_selected(info.selected.try_into().unwrap_or(u32::MAX));
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
        info: &UpgradeInfo,
        parent: &impl IsA<gtk::Widget>,
    ) -> Option<RoomVersionId> {
        self.imp().confirm_upgrade(info, parent.upcast_ref()).await
    }
}

/// The information necessary for [`UpgradeDialog`].
#[derive(Debug, Clone)]
pub(crate) struct UpgradeInfo {
    /// The sorted stable room versions available for the upgrade.
    pub(crate) stable_room_versions: Vec<RoomVersionId>,
    /// The sorted unstable room versions available for the upgrade.
    pub(crate) unstable_room_versions: Vec<RoomVersionId>,
    /// The position of the room version that should be selected by default,
    /// when `stable_room_versions` and `unstable_room_versions` are
    /// concatenated.
    pub(crate) selected: usize,
}

impl UpgradeInfo {
    /// Construct the `UpgradeInfo` with the given capability and current room
    /// version.
    ///
    /// We do not allow users to:
    ///
    /// - Downgrade the room, i.e. use a lower room version.
    /// - Upgrade to a version lower than the server's default, if the server's
    ///   default is stable.
    /// - Upgrade to an experimental version, unless it is the current version
    ///   or the server's default.
    ///
    /// If the server's default is experimental, we also allow to upgrade to the
    /// highest stable version.
    pub(crate) fn new(
        current_room_version: &RoomVersionId,
        capability: &RoomVersionsCapability,
    ) -> Self {
        let current_is_stable = capability.is_stable_version(current_room_version);
        let default_is_stable = capability.is_stable_version(&capability.default);
        let maximum_stable_version = capability.maximum_stable_version();

        // The minimum stable version is the highest stable version between the current
        // version and the default version.
        let minimum_stable_version = match (current_is_stable, default_is_stable) {
            (true, false) => Some(current_room_version),
            (false, true) => Some(&capability.default),
            (true, true) => Some(
                match numeric_sort::cmp(current_room_version.as_ref(), capability.default.as_ref())
                {
                    Ordering::Less => &capability.default,
                    Ordering::Equal | Ordering::Greater => current_room_version,
                },
            ),
            (false, false) => None,
        };
        let selected_room_version = minimum_stable_version.unwrap_or(&capability.default);

        let mut stable_room_versions = if let Some(minimum) = minimum_stable_version {
            // Keep all the stable versions higher than the minimum.
            capability
                .available
                .iter()
                .filter_map(|(version, stability)| {
                    // Discard unstable versions.
                    if *stability != RoomVersionStability::Stable {
                        return None;
                    }

                    if numeric_sort::cmp(version.as_ref(), minimum.as_ref()) != Ordering::Less
                        || maximum_stable_version.is_some_and(|maximum| maximum == version)
                    {
                        Some(version)
                    } else {
                        None
                    }
                })
                .cloned()
                .collect::<Vec<_>>()
        } else {
            // The only allowed stable version will be the maximum.
            maximum_stable_version.into_iter().cloned().collect()
        };

        // Add the current and default room versions if they are unstable.
        let current_is_default = *current_room_version == capability.default;
        let mut unstable_room_versions = Some(current_room_version)
            .filter(|_| !current_is_stable)
            .cloned()
            .into_iter()
            .chain(
                Some(&capability.default)
                    .filter(|_| !current_is_default && !default_is_stable)
                    .cloned(),
            )
            .collect::<Vec<_>>();

        // Sort all the versions.
        numeric_sort::sort_unstable(&mut stable_room_versions);
        numeric_sort::sort_unstable(&mut unstable_room_versions);

        // Find the position of the selected version.
        let selected = stable_room_versions
            .binary_search_by(|version| {
                numeric_sort::cmp(version.as_ref(), selected_room_version.as_ref())
            })
            .or_else(|_| {
                unstable_room_versions
                    .binary_search_by(|version| {
                        numeric_sort::cmp(version.as_ref(), selected_room_version.as_ref())
                    })
                    .map(|pos| stable_room_versions.len() + pos)
            })
            .unwrap_or_default();

        Self {
            stable_room_versions,
            unstable_room_versions,
            selected,
        }
    }
}

/// Helper trait for [`RoomVersionsCapability`].
trait RoomVersionsCapabilityExt {
    /// Whether the given room version is stable.
    fn is_stable_version(&self, version: &RoomVersionId) -> bool;

    /// The maximum stable room version in these capabilities.
    fn maximum_stable_version(&self) -> Option<&RoomVersionId>;
}

impl RoomVersionsCapabilityExt for RoomVersionsCapability {
    fn is_stable_version(&self, version: &RoomVersionId) -> bool {
        self.available
            .get(version)
            .is_some_and(|stability| *stability == RoomVersionStability::Stable)
    }

    fn maximum_stable_version(&self) -> Option<&RoomVersionId> {
        self.available
            .iter()
            .fold(None, |maximum, (version, stability)| {
                // Discard unstable versions.
                if *stability != RoomVersionStability::Stable {
                    return maximum;
                }

                // Keep the maximum.
                if maximum.is_none_or(|maximum| {
                    numeric_sort::cmp(version.as_ref(), maximum.as_ref()) == Ordering::Greater
                }) {
                    Some(version)
                } else {
                    maximum
                }
            })
    }
}
