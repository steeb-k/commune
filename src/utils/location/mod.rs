//! Location API.

use futures_util::{future::LocalBoxFuture, stream::LocalBoxStream, FutureExt, Stream, StreamExt};
use geo_uri::GeoUri;
use gtk::{glib, prelude::*, subclass::prelude::*};
use tracing::error;

#[cfg(target_os = "linux")]
mod linux;

/// High-level errors that can occur while fetching the location.
#[derive(Debug, Clone, Copy)]
pub enum LocationError {
    /// The user cancelled the request to get the location.
    Cancelled,
    /// The location services are disabled on the system.
    Disabled,
    /// Another error occurred.
    Other,
}

mod imp {
    use super::*;

    type LocationUpdatesFn =
        fn(&super::Location) -> LocalBoxFuture<Result<LocalBoxStream<GeoUri>, LocationError>>;

    #[repr(C)]
    pub struct LocationClass {
        pub parent_class: glib::object::Class<glib::Object>,
        pub is_available: fn(&super::Location) -> bool,
        pub init: fn(&super::Location) -> LocalBoxFuture<Result<(), LocationError>>,
        pub updates_stream: LocationUpdatesFn,
    }

    unsafe impl ClassStruct for LocationClass {
        type Type = Location;
    }

    pub(super) fn location_is_available(this: &super::Location) -> bool {
        let klass = this.class();
        (klass.as_ref().is_available)(this)
    }

    pub(super) async fn location_init(this: &super::Location) -> Result<(), LocationError> {
        let klass = this.class();
        (klass.as_ref().init)(this).await
    }

    pub(super) async fn location_updates_stream(
        this: &super::Location,
    ) -> Result<impl Stream<Item = GeoUri> + '_, LocationError> {
        let klass = this.class();
        (klass.as_ref().updates_stream)(this).await
    }

    #[derive(Debug, Default)]
    pub struct Location;

    #[glib::object_subclass]
    impl ObjectSubclass for Location {
        const NAME: &'static str = "Location";
        type Type = super::Location;
        type Class = LocationClass;
    }

    impl ObjectImpl for Location {}
}

glib::wrapper! {
    /// Subclassable Location API.
    ///
    /// The default implementation, for unsupported platforms, makes sure the location support is disabled.
    pub struct Location(ObjectSubclass<imp::Location>);
}

impl Location {
    /// Create a new `Location`.
    pub fn new() -> Self {
        #[cfg(target_os = "linux")]
        let obj = linux::LinuxLocation::new().upcast();

        #[cfg(not(target_os = "linux"))]
        let obj = glib::Object::new();

        obj
    }
}

impl Default for Location {
    fn default() -> Self {
        Self::new()
    }
}

unsafe impl Send for Location {}
unsafe impl Sync for Location {}

pub trait LocationExt: 'static {
    /// Whether the location API is available.
    fn is_available(&self) -> bool;

    /// Initialize the location API.
    async fn init(&self) -> Result<(), LocationError>;

    /// Listen to a stream of location updates.
    async fn updates_stream(&self) -> Result<impl Stream<Item = GeoUri> + '_, LocationError>;
}

impl<O: IsA<Location>> LocationExt for O {
    fn is_available(&self) -> bool {
        imp::location_is_available(self.upcast_ref())
    }

    async fn init(&self) -> Result<(), LocationError> {
        imp::location_init(self.upcast_ref()).await
    }

    async fn updates_stream(&self) -> Result<impl Stream<Item = GeoUri> + '_, LocationError> {
        imp::location_updates_stream(self.upcast_ref()).await
    }
}

/// Public trait that must be implemented for everything that derives from
/// `Location`.
///
/// Overriding a method from this Trait overrides also its behavior in
/// `LocationExt`.
#[allow(async_fn_in_trait)]
#[allow(clippy::unused_async)]
pub trait LocationImpl: ObjectImpl {
    /// Whether the location API is available.
    fn is_available(&self) -> bool {
        false
    }

    /// Initialize the location API.
    async fn init(&self) -> Result<(), LocationError> {
        error!("The location API is not supported on this platform");
        Err(LocationError::Other)
    }

    /// Listen to a stream of location updates.
    async fn updates_stream(&self) -> Result<impl Stream<Item = GeoUri> + '_, LocationError> {
        error!("The location API is not supported on this platform");
        Err::<LocalBoxStream<'_, GeoUri>, _>(LocationError::Other)
    }
}

unsafe impl<T> IsSubclassable<T> for Location
where
    T: LocationImpl,
    T::Type: IsA<Location>,
{
    fn class_init(class: &mut glib::Class<Self>) {
        Self::parent_class_init::<T>(class.upcast_ref_mut());

        let klass = class.as_mut();

        klass.is_available = is_available_trampoline::<T>;
        klass.init = init_trampoline::<T>;
        klass.updates_stream = updates_stream_trampoline::<T>;
    }
}

// Virtual method implementation trampolines.
fn is_available_trampoline<T>(this: &Location) -> bool
where
    T: ObjectSubclass + LocationImpl,
    T::Type: IsA<Location>,
{
    let this = this.downcast_ref::<T::Type>().unwrap();
    this.imp().is_available()
}

fn init_trampoline<T>(this: &Location) -> LocalBoxFuture<Result<(), LocationError>>
where
    T: ObjectSubclass + LocationImpl,
    T::Type: IsA<Location>,
{
    let this = this.downcast_ref::<T::Type>().unwrap();
    this.imp().init().boxed_local()
}

fn updates_stream_trampoline<T>(
    this: &Location,
) -> LocalBoxFuture<Result<LocalBoxStream<GeoUri>, LocationError>>
where
    T: ObjectSubclass + LocationImpl,
    T::Type: IsA<Location>,
{
    let this = this.downcast_ref::<T::Type>().unwrap();
    async move {
        let stream = this.imp().updates_stream().await?;
        Ok(stream.boxed_local())
    }
    .boxed_local()
}
