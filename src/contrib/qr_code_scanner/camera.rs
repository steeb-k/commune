// SPDX-License-Identifier: GPL-3.0-or-later
use std::time::Duration;

use ashpd::desktop::camera;
use gtk::{glib, subclass::prelude::*};
use once_cell::sync::Lazy;
use tracing::error;

use super::camera_paintable::CameraPaintable;
use crate::{spawn_tokio, utils::timeout_future};

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct Camera {
        pub paintable: glib::WeakRef<CameraPaintable>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Camera {
        const NAME: &'static str = "Camera";
        type Type = super::Camera;
    }

    impl ObjectImpl for Camera {}
}

glib::wrapper! {
    pub struct Camera(ObjectSubclass<imp::Camera>);
}

impl Camera {
    /// Create a new `Camera`.
    ///
    /// Use `Camera::default()` to get a shared GObject.
    fn new() -> Self {
        glib::Object::new()
    }

    /// Ask the system whether cameras are available.
    pub async fn has_cameras(&self) -> bool {
        let handle = spawn_tokio!(async move {
            let camera = match camera::Camera::new().await {
                Ok(camera) => camera,
                Err(error) => {
                    error!("Failed to create instance of camera proxy: {error}");
                    return false;
                }
            };

            match camera.is_present().await {
                Ok(is_present) => is_present,
                Err(error) => {
                    error!("Failed to check whether system has cameras: {error}");
                    false
                }
            }
        });
        let abort_handle = handle.abort_handle();

        match timeout_future(Duration::from_secs(1), handle).await {
            Ok(is_present) => is_present.expect("The task should not have been aborted"),
            Err(_) => {
                abort_handle.abort();
                error!("Failed to check whether system has cameras: the request timed out");
                false
            }
        }
    }

    /// Get the a `gdk::Paintable` displaying the content of a camera.
    ///
    /// Panics if not called from the `MainContext` where GTK is running.
    pub async fn paintable(&self) -> Option<CameraPaintable> {
        // We need to make sure that the Paintable is taken only from the MainContext
        assert!(glib::MainContext::default().is_owner());
        let imp = self.imp();

        if let Some(paintable) = imp.paintable.upgrade() {
            return Some(paintable);
        }

        let handle = spawn_tokio!(async move { camera::request().await });
        let abort_handle = handle.abort_handle();

        match timeout_future(Duration::from_secs(1), handle).await {
            Ok(tokio_res) => match tokio_res.expect("The task should not have been aborted") {
                Ok(Some((fd, streams))) => {
                    let paintable = CameraPaintable::new(fd, streams).await;
                    imp.paintable.set(Some(&paintable));

                    Some(paintable)
                }
                Ok(None) => {
                    error!("Failed to request access to cameras: the response is empty");
                    None
                }
                Err(error) => {
                    error!("Failed to request access to cameras: {error}");
                    None
                }
            },
            Err(_) => {
                // Error because we reached the timeout.
                abort_handle.abort();
                error!("Failed to request access to cameras: the request timed out");
                None
            }
        }
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
