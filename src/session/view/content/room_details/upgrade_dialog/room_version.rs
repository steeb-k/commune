use std::{cmp::Ordering, str::FromStr};

use gtk::{glib, prelude::*, subclass::prelude::*};
use ruma::{RoomVersionId, api::client::discovery::get_capabilities::v3::RoomVersionStability};

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
    pub(crate) fn cmp_ids(a: &RoomVersion, b: &RoomVersion) -> Ordering {
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
