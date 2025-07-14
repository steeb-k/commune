use std::{cmp::Ordering, str::FromStr};

use adw::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib, pango, subclass::prelude::*};
use ruma::{
    RoomVersionId,
    api::client::discovery::get_capabilities::v3::{RoomVersionStability, RoomVersionsCapability},
};
use tracing::error;

/// Show a dialog to confirm the room upgrade and select a room version.
///
/// Returns the selected room version, or `None` if the user didn't confirm.
pub(crate) async fn confirm_room_upgrade(
    capability: RoomVersionsCapability,
    parent: &impl IsA<gtk::Widget>,
) -> Option<RoomVersionId> {
    // Build the lists.
    let default = capability.default;
    let (mut stable_list, mut experimental_list) = capability
        .available
        .into_iter()
        .map(|(id, stability)| {
            let stability = if id == default {
                // According to the spec, the default version is always assumed to be stable.
                RoomVersionStability::Stable
            } else {
                stability
            };

            RoomVersion::new(id, stability)
        })
        .partition::<Vec<_>, _>(|version| *version.stability() == RoomVersionStability::Stable);

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

    // Add an entry for the optional reason.
    let version_combo = adw::ComboRow::builder()
        .title(gettext("Version"))
        .selectable(false)
        .expression(RoomVersion::this_expression("id-string"))
        .header_factory(&header_factory)
        .model(&flatten_model)
        .selected(default_pos.try_into().unwrap_or(u32::MAX))
        .build();
    let list_box = gtk::ListBox::builder()
        .css_classes(["boxed-list"])
        .margin_top(6)
        .accessible_role(gtk::AccessibleRole::Group)
        .build();
    list_box.append(&version_combo);

    // Build dialog.
    let upgrade_dialog = adw::AlertDialog::builder()
        .default_response("cancel")
        .heading(gettext("Upgrade Room"))
        .body(gettext("Upgrading a room to a more recent version allows to benefit from new features from the Matrix specification. It can also be used to reset the room state, which should make the room faster to join. However it should be used sparingly because it can be disruptive, as room members need to join the new room manually."))
        .extra_child(&list_box)
        .build();
    upgrade_dialog.add_responses(&[
        ("cancel", &gettext("Cancel")),
        // Translators: In this string, 'Upgrade' is a verb, as in 'Upgrade Room'.
        ("upgrade", &gettext("Upgrade")),
    ]);
    upgrade_dialog.set_response_appearance("upgrade", adw::ResponseAppearance::Destructive);

    if upgrade_dialog.choose_future(parent).await != "upgrade" {
        return None;
    }

    version_combo
        .selected_item()
        .and_downcast::<RoomVersion>()
        .map(|v| v.id().clone())
}

mod imp {
    use std::{cell::OnceCell, marker::PhantomData};

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::RoomVersion)]
    pub struct RoomVersion {
        /// The ID of the version.
        id: OnceCell<RoomVersionId>,
        /// The ID of the version as a string.
        #[property(get = Self::id_string)]
        id_string: PhantomData<String>,
        /// The stability of the version.
        stability: OnceCell<RoomVersionStability>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RoomVersion {
        const NAME: &'static str = "RoomUpgradeDialogRoomVersion";
        type Type = super::RoomVersion;
    }

    #[glib::derived_properties]
    impl ObjectImpl for RoomVersion {}

    impl RoomVersion {
        /// Set the ID of this version.
        pub(super) fn set_id(&self, id: RoomVersionId) {
            self.id.set(id).expect("id is uninitialized");
        }

        /// The ID of this version.
        pub(super) fn id(&self) -> &RoomVersionId {
            self.id.get().expect("id is initialized")
        }

        /// The ID of this version as a string.
        fn id_string(&self) -> String {
            self.id().to_string()
        }

        /// Set the stability of this version.
        pub(super) fn set_stability(&self, stability: RoomVersionStability) {
            self.stability
                .set(stability)
                .expect("stability is uninitialized");
        }

        /// The stability of this version.
        pub(super) fn stability(&self) -> &RoomVersionStability {
            self.stability.get().expect("stability is initialized")
        }
    }
}

glib::wrapper! {
    /// A room version.
    pub struct RoomVersion(ObjectSubclass<imp::RoomVersion>);
}

impl RoomVersion {
    /// Constructs a new `RoomVersion`.
    pub fn new(id: RoomVersionId, stability: RoomVersionStability) -> Self {
        let obj = glib::Object::new::<Self>();

        let imp = obj.imp();
        imp.set_id(id);
        imp.set_stability(stability);

        obj
    }

    /// The ID of this version.
    pub(crate) fn id(&self) -> &RoomVersionId {
        self.imp().id()
    }

    /// The stability of this version.
    pub(crate) fn stability(&self) -> &RoomVersionStability {
        self.imp().stability()
    }

    /// Compare the IDs of the two given `RoomVersion`s.
    ///
    /// Correctly sorts numbers: string comparison will sort `1, 10, 2`, we want
    /// `1, 2, 10`.
    fn cmp_ids(a: &RoomVersion, b: &RoomVersion) -> Ordering {
        match (
            i64::from_str(a.id().as_str()),
            i64::from_str(b.id().as_str()),
        ) {
            (Ok(a), Ok(b)) => a.cmp(&b),
            (Ok(_), _) => Ordering::Less,
            (_, Ok(_)) => Ordering::Greater,
            _ => a.id().cmp(b.id()),
        }
    }
}
