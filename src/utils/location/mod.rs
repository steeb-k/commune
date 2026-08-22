//! Location API.

use futures_util::Stream;
use geo_uri::GeoUri;

#[cfg(target_os = "linux")]
mod linux;

cfg_if::cfg_if! {
    if #[cfg(target_os = "linux")] {
        /// The location API.
        pub(crate) type Location = linux::LinuxLocation;
    } else {
        /// The location API.
        pub(crate) type Location = unimplemented::UnimplementedLocation;
    }
}

/// Trait implemented by location backends.
pub(crate) trait LocationExt {
    /// Whether the location API is available.
    fn is_available(&self) -> bool;

    /// Initialize the location API.
    async fn init(&self) -> Result<(), LocationError>;

    /// Listen to a stream of location updates.
    async fn updates_stream(&self) -> Result<impl Stream<Item = GeoUri> + '_, LocationError>;
}

/// The fallback location API, used on platforms where it is unimplemented.
#[cfg(not(target_os = "linux"))]
mod unimplemented {
    use futures_util::stream;

    use super::*;

    #[derive(Debug)]
    pub(crate) struct UnimplementedLocation;

    impl UnimplementedLocation {
        /// Construct an `UnimplementedLocation`.
        ///
        /// This mirrors `LinuxLocation::new()` so that call sites do not need
        /// to know which backend they got.
        pub(crate) fn new() -> Self {
            Self
        }
    }

    impl LocationExt for UnimplementedLocation {
        /// Whether the location API is available.
        fn is_available(&self) -> bool {
            false
        }

        /// Initialize the location API.
        async fn init(&self) -> Result<(), LocationError> {
            Err(LocationError::Disabled)
        }

        /// Listen to a stream of location updates.
        ///
        /// Returning an error rather than panicking keeps a caller that forgot
        /// to check `is_available()` from taking the app down. The empty stream
        /// only names the return type; it is never produced.
        async fn updates_stream(&self) -> Result<impl Stream<Item = GeoUri> + '_, LocationError> {
            Err::<stream::Empty<GeoUri>, _>(LocationError::Disabled)
        }
    }
}

/// High-level errors that can occur while fetching the location.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(
    not(target_os = "linux"),
    allow(
        dead_code,
        reason = "only a real location backend reports anything but Disabled"
    )
)]
pub(crate) enum LocationError {
    /// The user cancelled the request to get the location.
    Cancelled,
    /// The location services are disabled on the system.
    Disabled,
    /// Another error occurred.
    Other,
}
