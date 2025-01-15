// SPDX-License-Identifier: GPL-3.0-or-later
use std::time::Duration;

use ashpd::desktop::camera;
use gtk::prelude::*;
use tracing::error;

mod viewfinder;

use self::viewfinder::LinuxCameraViewfinder;
use super::{CameraExt, CameraViewfinder};
use crate::{spawn_tokio, utils::timeout_future};

/// Camera API under Linux.
#[derive(Debug)]
pub(crate) struct LinuxCamera;

impl CameraExt for LinuxCamera {
    async fn has_cameras() -> bool {
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

    async fn viewfinder() -> Option<CameraViewfinder> {
        LinuxCameraViewfinder::new().await.and_upcast()
    }
}
