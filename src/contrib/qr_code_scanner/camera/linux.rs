// SPDX-License-Identifier: GPL-3.0-or-later
use std::time::Duration;

use ashpd::desktop::camera;
use gtk::{glib, prelude::*, subclass::prelude::*};
use tracing::error;

use super::{
    camera_paintable::{linux::LinuxCameraPaintable, CameraPaintable},
    Camera, CameraImpl,
};
use crate::{spawn_tokio, utils::timeout_future};

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct LinuxCamera {
        pub paintable: glib::WeakRef<LinuxCameraPaintable>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LinuxCamera {
        const NAME: &'static str = "LinuxCamera";
        type Type = super::LinuxCamera;
        type ParentType = Camera;
    }

    impl ObjectImpl for LinuxCamera {}

    impl CameraImpl for LinuxCamera {
        async fn has_cameras(&self) -> bool {
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

        async fn paintable(&self) -> Option<CameraPaintable> {
            // We need to make sure that the Paintable is taken only from the MainContext
            assert!(glib::MainContext::default().is_owner());

            if let Some(paintable) = self.paintable.upgrade() {
                return Some(paintable.upcast());
            }

            let handle = spawn_tokio!(async move { camera::request().await });
            let abort_handle = handle.abort_handle();

            match timeout_future(Duration::from_secs(1), handle).await {
                Ok(tokio_res) => match tokio_res.expect("The task should not have been aborted") {
                    Ok(Some((fd, streams))) => {
                        let paintable = LinuxCameraPaintable::new(fd, streams).await;
                        self.paintable.set(Some(&paintable));

                        Some(paintable.upcast())
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
}

glib::wrapper! {
    pub struct LinuxCamera(ObjectSubclass<imp::LinuxCamera>) @extends Camera;
}

impl LinuxCamera {
    /// Create a new `LinuxCamera`.
    pub fn new() -> Self {
        glib::Object::new()
    }
}

impl Default for LinuxCamera {
    fn default() -> Self {
        Self::new()
    }
}

unsafe impl Send for LinuxCamera {}
unsafe impl Sync for LinuxCamera {}
