//! Linux Location API.

use ashpd::{
    desktop::{
        location::{Accuracy, Location as PortalLocation, LocationProxy},
        Session,
    },
    WindowIdentifier,
};
use futures_util::{future, stream, FutureExt, Stream, StreamExt, TryFutureExt};
use geo_uri::GeoUri;
use gtk::{glib, subclass::prelude::*};
use tracing::error;

use super::{Location, LocationError, LocationImpl};
use crate::spawn_tokio;

impl From<ashpd::Error> for LocationError {
    fn from(value: ashpd::Error) -> Self {
        match value {
            ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled) => Self::Cancelled,
            ashpd::Error::Portal(ashpd::PortalError::NotAllowed(_)) => Self::Disabled,
            _ => Self::Other,
        }
    }
}

mod imp {
    use std::{cell::OnceCell, sync::Arc};

    use super::*;

    #[derive(Debug, Default)]
    pub struct LinuxLocation {
        pub proxy: OnceCell<
            Arc<(
                LocationProxy<'static>,
                Session<'static, LocationProxy<'static>>,
            )>,
        >,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LinuxLocation {
        const NAME: &'static str = "LinuxLocation";
        type Type = super::LinuxLocation;
        type ParentType = Location;
    }

    impl ObjectImpl for LinuxLocation {
        fn dispose(&self) {
            if let Some(proxy) = self.proxy.get().cloned() {
                spawn_tokio!(async move {
                    if let Err(error) = proxy.1.close().await {
                        error!("Could not close session of location API: {error}");
                    }
                });
            }
        }
    }

    impl LocationImpl for LinuxLocation {
        fn is_available(&self) -> bool {
            true
        }

        async fn init(&self) -> Result<(), LocationError> {
            match self.init().await {
                Ok(()) => Ok(()),
                Err(error) => {
                    error!("Could not initialize location API: {error}");
                    Err(error.into())
                }
            }
        }

        async fn updates_stream(&self) -> Result<impl Stream<Item = GeoUri> + '_, LocationError> {
            match self.updates_stream().await {
                Ok(stream) => Ok(stream.map(|l| {
                    GeoUri::builder()
                        .latitude(l.latitude())
                        .longitude(l.longitude())
                        .build()
                        .expect("Got invalid coordinates from location API")
                })),
                Err(error) => {
                    error!("Could not access update stream of location API: {error}");
                    Err(error.into())
                }
            }
        }
    }

    impl LinuxLocation {
        /// Initialize the proxy.
        async fn init(&self) -> Result<(), ashpd::Error> {
            if self.proxy.get().is_some() {
                return Ok(());
            }

            let proxy = spawn_tokio!(async move {
                let proxy = LocationProxy::new().await?;

                let session = proxy
                    .create_session(Some(0), Some(0), Some(Accuracy::Exact))
                    .await?;

                ashpd::Result::Ok((proxy, session))
            })
            .await
            .unwrap()?;

            self.proxy.set(proxy.into()).unwrap();
            Ok(())
        }

        /// Listen to updates from the proxy.
        async fn updates_stream(
            &self,
        ) -> Result<impl Stream<Item = PortalLocation> + '_, ashpd::Error> {
            let proxy = self.proxy.get().unwrap().clone();

            spawn_tokio!(async move {
                let (proxy, session) = &*proxy;
                let identifier = WindowIdentifier::default();

                // We want to be listening for new locations whenever the session is up
                // otherwise we might lose the first response and will have to wait for a future
                // update by geoclue.
                let mut stream = proxy.receive_location_updated().await?;
                let (_, first_location) = future::try_join(
                    proxy.start(session, &identifier).into_future(),
                    stream.next().map(|l| l.ok_or(ashpd::Error::NoResponse)),
                )
                .await?;

                ashpd::Result::Ok(stream::once(future::ready(first_location)).chain(stream))
            })
            .await
            .unwrap()
        }
    }
}

glib::wrapper! {
    /// Location API under Linux, using the Location XDG Desktop Portal.
    pub struct LinuxLocation(ObjectSubclass<imp::LinuxLocation>) @extends Location;
}

impl LinuxLocation {
    /// Create a new `LinuxLocation`.
    ///
    /// Use `LinuxLocation::default()` to get a shared GObject.
    pub fn new() -> Self {
        glib::Object::new()
    }
}

impl Default for LinuxLocation {
    fn default() -> Self {
        Self::new()
    }
}

unsafe impl Send for LinuxLocation {}
unsafe impl Sync for LinuxLocation {}
