use std::{cmp::Ordering, str::FromStr};

use adw::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib, subclass::prelude::*};
use ruma::{
    api::client::discovery::get_capabilities::{RoomVersionStability, RoomVersionsCapability},
    RoomVersionId,
};

use crate::gettext_f;

/// Show a dialog to confirm the room upgrade and select a room version.
///
/// Returns the selected room version, or `None` if the user didn't confirm.
pub async fn confirm_room_upgrade(
    capability: RoomVersionsCapability,
    transient_for: &gtk::Window,
) -> Option<RoomVersionId> {
    // Build the model.
    let default = capability.default;
    let mut list = capability
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
        .collect::<Vec<_>>();

    // Correctly sort numbers (string comparison will sort `1, 10, 2`, we want `1,
    // 2, 10`).
    list.sort_unstable_by(|a, b| {
        match (
            i64::from_str(a.id().as_str()),
            i64::from_str(b.id().as_str()),
        ) {
            (Ok(a), Ok(b)) => a.cmp(&b),
            (Ok(_), _) => Ordering::Less,
            (_, Ok(_)) => Ordering::Greater,
            _ => a.id().cmp(b.id()),
        }
    });

    let default_pos = list
        .iter()
        .position(|v| *v.id() == default)
        .unwrap_or_default();
    let model = list.into_iter().collect::<gio::ListStore>();

    // Add an entry for the optional reason.
    let version_combo = adw::ComboRow::builder()
        .title(gettext("Version"))
        .selectable(false)
        .expression(RoomVersion::this_expression("display-string"))
        .model(&model)
        .selected(default_pos.try_into().unwrap_or(u32::MAX))
        .build();
    let list_box = gtk::ListBox::builder()
        .css_classes(["boxed-list"])
        .margin_top(6)
        .accessible_role(gtk::AccessibleRole::Group)
        .build();
    list_box.append(&version_combo);

    // Build dialog.
    let upgrade_dialog = adw::MessageDialog::builder()
        .transient_for(transient_for)
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

    if upgrade_dialog.choose_future().await != "upgrade" {
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
        pub id: OnceCell<RoomVersionId>,
        /// The stability of the version.
        pub stability: OnceCell<RoomVersionStability>,
        /// The string used to display this version.
        #[property(get = Self::display_string)]
        display_string: PhantomData<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RoomVersion {
        const NAME: &'static str = "RoomUpgradeDialogRoomVersion";
        type Type = super::RoomVersion;
    }

    #[glib::derived_properties]
    impl ObjectImpl for RoomVersion {}

    impl RoomVersion {
        /// The string used to display this version.
        fn display_string(&self) -> String {
            let id = self.id.get().unwrap();
            let stability = self.stability.get().unwrap();

            if *stability != RoomVersionStability::Stable {
                // Translators: Do NOT translate the content between '{' and '}', this is a
                // variable name.
                gettext_f("{version} (unstable)", &[("version", id.as_str())])
            } else {
                id.to_string()
            }
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
        imp.id.set(id).unwrap();
        imp.stability.set(stability).unwrap();

        obj
    }

    /// The ID of this version.
    pub fn id(&self) -> &RoomVersionId {
        self.imp().id.get().unwrap()
    }

    /// The stability of this version.
    pub fn stability(&self) -> &RoomVersionStability {
        self.imp().stability.get().unwrap()
    }
}
