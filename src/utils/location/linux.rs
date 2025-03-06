//! Linux Location API.

use std::{cell::OnceCell, sync::Arc};

use ashpd::desktop::{
    location::{Accuracy, Location as PortalLocation, LocationProxy},
    Session,
};
use futures_util::{future, stream, FutureExt, Stream, StreamExt, TryFutureExt};
use geo_uri::GeoUri;
use tracing::error;

use super::{LocationError, LocationExt};
use crate::spawn_tokio;

/// Location API under Linux, using the Location XDG Desktop Portal.
#[derive(Debug, Default)]
pub(crate) struct LinuxLocation {
    inner: OnceCell<Arc<ProxyAndSession>>,
}

/// A location proxy and it's associated session.
#[derive(Debug)]
struct ProxyAndSession {
    proxy: LocationProxy<'static>,
    session: Session<'static, LocationProxy<'static>>,
}

impl LocationExt for LinuxLocation {
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
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Initialize the proxy.
    async fn init(&self) -> Result<(), ashpd::Error> {
        if self.inner.get().is_some() {
            return Ok(());
        }

        let inner = spawn_tokio!(async move {
            let proxy = LocationProxy::new().await?;

            let session = proxy
                .create_session(Some(0), Some(0), Some(Accuracy::Exact))
                .await?;

            ashpd::Result::Ok(ProxyAndSession { proxy, session })
        })
        .await
        .unwrap()?;

        self.inner.set(inner.into()).unwrap();
        Ok(())
    }

    /// Listen to updates from the proxy.
    async fn updates_stream(
        &self,
    ) -> Result<impl Stream<Item = PortalLocation> + '_, ashpd::Error> {
        let inner = self
            .inner
            .get()
            .expect("location API should be initialized")
            .clone();

        spawn_tokio!(async move {
            let ProxyAndSession { proxy, session } = &*inner;

            // We want to be listening for new locations whenever the session is up
            // otherwise we might lose the first response and will have to wait for a future
            // update by geoclue.
            let mut stream = proxy.receive_location_updated().await?;
            let (_, first_location) = future::try_join(
                proxy.start(session, None).into_future(),
                stream.next().map(|l| l.ok_or(ashpd::Error::NoResponse)),
            )
            .await?;

            ashpd::Result::Ok(stream::once(future::ready(first_location)).chain(stream))
        })
        .await
        .unwrap()
    }
}

impl Drop for LinuxLocation {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.take() {
            spawn_tokio!(async move {
                if let Err(error) = inner.session.close().await {
                    error!("Could not close session of location API: {error}");
                }
            });
        }
    }
}

impl From<ashpd::Error> for LocationError {
    fn from(value: ashpd::Error) -> Self {
        match value {
            ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled) => Self::Cancelled,
            ashpd::Error::Portal(ashpd::PortalError::NotAllowed(_)) => Self::Disabled,
            _ => Self::Other,
        }
    }
}
