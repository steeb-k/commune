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
                        error!("Could not create instance of camera proxy: {error}");
                        return false;
                    }
                };

                match camera.is_present().await {
                    Ok(is_present) => is_present,
                    Err(error) => {
                        error!("Could not check whether system has cameras: {error}");
                        false
                    }
                }
            });
            let abort_handle = handle.abort_handle();

            if let Ok(is_present) = timeout_future(Duration::from_secs(1), handle).await {
                is_present.expect("The task should not have been aborted")
            } else {
                abort_handle.abort();
                error!("Could not check whether system has cameras: the request timed out");
                false
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

            if let Ok(tokio_res) = timeout_future(Duration::from_secs(1), handle).await {
                match tokio_res.expect("The task should not have been aborted") {
                    Ok(Some((fd, streams))) => {
                        let paintable = LinuxCameraPaintable::new(fd, streams).await;
                        self.paintable.set(Some(&paintable));

                        Some(paintable.upcast())
                    }
                    Ok(None) => {
                        error!("Could not request access to cameras: the response is empty");
                        None
                    }
                    Err(error) => {
                        error!("Could not request access to cameras: {error}");
                        None
                    }
                }
            } else {
                // Error because we reached the timeout.
                abort_handle.abort();
                error!("Could not request access to cameras: the request timed out");
                None
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
