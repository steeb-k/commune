//! Camera API.

use futures_util::{future::LocalBoxFuture, FutureExt};
use gtk::{glib, prelude::*, subclass::prelude::*};
use once_cell::sync::Lazy;
use tracing::error;

mod camera_paintable;
#[cfg(target_os = "linux")]
mod linux;

pub use self::camera_paintable::Action;
use self::camera_paintable::CameraPaintable;

mod imp {

    use super::*;

    #[repr(C)]
    pub struct CameraClass {
        pub parent_class: glib::object::Class<glib::Object>,
        pub has_cameras: fn(&super::Camera) -> LocalBoxFuture<bool>,
        pub paintable: fn(&super::Camera) -> LocalBoxFuture<Option<CameraPaintable>>,
    }

    unsafe impl ClassStruct for CameraClass {
        type Type = Camera;
    }

    pub(super) async fn camera_has_cameras(this: &super::Camera) -> bool {
        let klass = this.class();
        (klass.as_ref().has_cameras)(this).await
    }

    pub(super) async fn camera_paintable(this: &super::Camera) -> Option<CameraPaintable> {
        let klass = this.class();
        (klass.as_ref().paintable)(this).await
    }

    #[derive(Debug, Default)]
    pub struct Camera;

    #[glib::object_subclass]
    impl ObjectSubclass for Camera {
        const NAME: &'static str = "Camera";
        type Type = super::Camera;
        type Class = CameraClass;
    }

    impl ObjectImpl for Camera {}
}

glib::wrapper! {
    /// Subclassable Camera API.
    ///
    /// The default implementation, for unsupported platforms, makes sure the camera support is disabled.
    pub struct Camera(ObjectSubclass<imp::Camera>);
}

impl Camera {
    /// Create a new `Camera`.
    ///
    /// Use `Camera::default()` to get a shared GObject.
    fn new() -> Self {
        #[cfg(target_os = "linux")]
        let obj = linux::LinuxCamera::new().upcast();

        #[cfg(not(target_os = "linux"))]
        let obj = glib::Object::new();

        obj
    }
}

impl Default for Camera {
    fn default() -> Self {
        static CAMERA: Lazy<Camera> = Lazy::new(Camera::new);

        CAMERA.to_owned()
    }
}

unsafe impl Send for Camera {}
unsafe impl Sync for Camera {}

pub trait CameraExt: 'static {
    /// Whether any cameras are available.
    async fn has_cameras(&self) -> bool;

    /// The paintable displaying the camera.
    async fn paintable(&self) -> Option<CameraPaintable>;
}

impl<O: IsA<Camera>> CameraExt for O {
    async fn has_cameras(&self) -> bool {
        imp::camera_has_cameras(self.upcast_ref()).await
    }

    async fn paintable(&self) -> Option<CameraPaintable> {
        imp::camera_paintable(self.upcast_ref()).await
    }
}

/// Public trait that must be implemented for everything that derives from
/// `Camera`.
///
/// Overriding a method from this Trait overrides also its behavior in
/// `CameraExt`.
#[allow(async_fn_in_trait)]
pub trait CameraImpl: ObjectImpl {
    /// Whether any cameras are available.
    async fn has_cameras(&self) -> bool {
        false
    }

    /// The paintable displaying the camera.
    async fn paintable(&self) -> Option<CameraPaintable> {
        error!("The camera API is not supported on this platform");
        None
    }
}

unsafe impl<T> IsSubclassable<T> for Camera
where
    T: CameraImpl,
    T::Type: IsA<Camera>,
{
    fn class_init(class: &mut glib::Class<Self>) {
        Self::parent_class_init::<T>(class.upcast_ref_mut());

        let klass = class.as_mut();

        klass.has_cameras = has_cameras_trampoline::<T>;
        klass.paintable = paintable_trampoline::<T>;
    }
}

// Virtual method implementation trampolines.
fn has_cameras_trampoline<T>(this: &Camera) -> LocalBoxFuture<bool>
where
    T: ObjectSubclass + CameraImpl,
    T::Type: IsA<Camera>,
{
    let this = this.downcast_ref::<T::Type>().unwrap();
    this.imp().has_cameras().boxed_local()
}

fn paintable_trampoline<T>(this: &Camera) -> LocalBoxFuture<Option<CameraPaintable>>
where
    T: ObjectSubclass + CameraImpl,
    T::Type: IsA<Camera>,
{
    let this = this.downcast_ref::<T::Type>().unwrap();
    this.imp().paintable().boxed_local()
}
