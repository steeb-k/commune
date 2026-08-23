use std::{
    cell::{Cell, OnceCell, RefCell},
    rc::Rc,
};

use gtk::{glib, glib::clone, prelude::*, subclass::prelude::*};
use ruma::{
    OwnedMxcUri, UInt,
    api::{MatrixVersion, client::authenticated_media::get_media_preview, error::ErrorKind},
    events::room::ImageInfo,
};
use serde_json::{Map as JsonMap, Value as JsonValue};
use tracing::{debug, warn};
use url::Url;

use crate::{session::Session, spawn, spawn_tokio, utils::LoadingState};

/// The first Matrix version where the endpoint we use is stable.
///
/// Before that it only existed as MSC3916, under an unstable path. We never
/// send a request to an unstable path, so on an older homeserver this feature
/// is simply off.
const PREVIEW_URL_STABLE_SINCE: MatrixVersion = MatrixVersion::V1_11;

/// Whether the homeserver of a session can answer a URL preview request.
///
/// `None` means that we have not found out yet. This is shared between every
/// preview of a session, so that neither the version check nor a refusal is
/// paid for more than once.
pub(super) type UrlPreviewSupport = Rc<Cell<Option<bool>>>;

/// The image of a URL preview.
#[derive(Debug, Clone)]
pub(crate) struct UrlPreviewImage {
    /// The `mxc:` URI of the image.
    pub(crate) uri: OwnedMxcUri,
    /// What the homeserver said about the image.
    pub(crate) info: ImageInfo,
}

mod imp {
    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::RemoteUrlPreview)]
    pub struct RemoteUrlPreview {
        /// The URL that this is a preview of.
        url: OnceCell<Url>,
        /// The loading state of the preview.
        #[property(get, builder(LoadingState::default()))]
        loading_state: Cell<LoadingState>,
        /// The title of the page, if it has one.
        #[property(get, nullable)]
        title: RefCell<Option<String>>,
        /// The description of the page, if it has one.
        #[property(get, nullable)]
        description: RefCell<Option<String>>,
        /// The name of the site the page belongs to.
        ///
        /// This falls back to the host of the URL, so it is never empty.
        #[property(get)]
        site_name: RefCell<String>,
        /// The image of the page, if it has one.
        image: RefCell<Option<UrlPreviewImage>>,
        /// Whether the homeserver can answer a preview request at all.
        support: OnceCell<UrlPreviewSupport>,
        session: glib::WeakRef<Session>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RemoteUrlPreview {
        const NAME: &'static str = "RemoteUrlPreview";
        type Type = super::RemoteUrlPreview;
    }

    #[glib::derived_properties]
    impl ObjectImpl for RemoteUrlPreview {}

    impl RemoteUrlPreview {
        /// The URL that this is a preview of.
        pub(super) fn url(&self) -> &Url {
            self.url.get().expect("URL should be initialized")
        }

        /// The image of the page, if it has one.
        pub(super) fn image(&self) -> Option<UrlPreviewImage> {
            self.image.borrow().clone()
        }

        /// Initialize this preview.
        pub(super) fn init(&self, session: &Session, url: Url, support: UrlPreviewSupport) {
            // The host is the fallback for the site name, and the only thing we
            // have to show until the homeserver answers.
            self.site_name
                .replace(url.host_str().unwrap_or_default().to_owned());

            self.url.set(url).expect("URL should be uninitialized");
            self.support
                .set(support)
                .expect("support should be uninitialized");
            self.session.set(Some(session));
        }

        /// Set the loading state.
        fn set_loading_state(&self, loading_state: LoadingState) {
            if self.loading_state.get() == loading_state {
                return;
            }

            self.loading_state.set(loading_state);
            self.obj().notify_loading_state();
        }

        /// Request this preview from the homeserver.
        pub(super) async fn load(&self) {
            let Some(session) = self.session.upgrade() else {
                return;
            };
            let support = self.support.get().expect("support should be initialized");

            self.set_loading_state(LoadingState::Loading);

            if !self.is_supported(&session, support).await {
                self.set_loading_state(LoadingState::Error);
                return;
            }

            let client = session.client();
            let request = get_media_preview::v1::Request::new(self.url().to_string());
            let handle = spawn_tokio!(async move { client.send(request).await });

            let response = match handle.await.expect("task was not aborted") {
                Ok(response) => response,
                Err(error) => {
                    // A homeserver is allowed to switch previews off, and one
                    // that has refuses every request the same way. Take the
                    // first refusal for the whole session rather than asking
                    // again for every link in the timeline.
                    if error.client_api_error_kind() == Some(&ErrorKind::Unrecognized) {
                        debug!("Homeserver does not offer URL previews, not asking again");
                        support.set(Some(false));
                    } else {
                        warn!("Could not get a preview for `{}`: {error}", self.url());
                    }

                    self.set_loading_state(LoadingState::Error);
                    return;
                }
            };

            let Some(data) = response.data else {
                // An empty response is a valid answer: the homeserver looked
                // and found nothing to show.
                self.set_loading_state(LoadingState::Error);
                return;
            };

            match serde_json::from_str::<JsonMap<String, JsonValue>>(data.get()) {
                Ok(data) => self.set_data(&data),
                Err(error) => {
                    warn!(
                        "Could not deserialize the preview for `{}`: {error}",
                        self.url()
                    );
                    self.set_loading_state(LoadingState::Error);
                }
            }
        }

        /// Whether the homeserver can answer a preview request.
        async fn is_supported(&self, session: &Session, support: &UrlPreviewSupport) -> bool {
            if let Some(is_supported) = support.get() {
                return is_supported;
            }

            let client = session.client();
            let handle = spawn_tokio!(async move { client.server_versions().await });

            let is_supported = match handle.await.expect("task was not aborted") {
                Ok(versions) => versions
                    .iter()
                    .any(|version| *version >= PREVIEW_URL_STABLE_SINCE),
                Err(error) => {
                    // Not knowing is not the same as knowing that it is
                    // missing, so this answer is not remembered.
                    warn!("Could not get the versions supported by the homeserver: {error}");
                    return false;
                }
            };

            if !is_supported {
                debug!("Homeserver does not support Matrix 1.11, URL previews are off");
            }

            support.set(Some(is_supported));
            is_supported
        }

        /// Set the data of this preview from the `OpenGraph` properties
        /// returned by the homeserver.
        fn set_data(&self, data: &JsonMap<String, JsonValue>) {
            let title = string_property(data, "og:title");
            let description = string_property(data, "og:description");
            let image = image_property(data);

            if title.is_none() && description.is_none() && image.is_none() {
                // There is nothing to put on a card.
                self.set_loading_state(LoadingState::Error);
                return;
            }

            let obj = self.obj();

            if let Some(site_name) = string_property(data, "og:site_name") {
                self.site_name.replace(site_name);
                obj.notify_site_name();
            }

            self.title.replace(title);
            obj.notify_title();

            self.description.replace(description);
            obj.notify_description();

            self.image.replace(image);

            self.set_loading_state(LoadingState::Ready);
        }
    }
}

glib::wrapper! {
    /// A preview of a URL, as the homeserver describes it.
    ///
    /// The response of the endpoint is free-form `OpenGraph` data: every
    /// property is optional, and none of them can be trusted to have the type
    /// we expect.
    pub struct RemoteUrlPreview(ObjectSubclass<imp::RemoteUrlPreview>);
}

impl RemoteUrlPreview {
    pub(super) fn new(session: &Session, url: Url, support: UrlPreviewSupport) -> Self {
        let obj = glib::Object::new::<Self>();

        obj.imp().init(session, url, support);
        obj.load();

        obj
    }

    /// The URL that this is a preview of.
    pub(crate) fn url(&self) -> Url {
        self.imp().url().clone()
    }

    /// The image of the page, if it has one.
    pub(crate) fn image(&self) -> Option<UrlPreviewImage> {
        self.imp().image()
    }

    /// Request this preview from the homeserver.
    fn load(&self) {
        spawn!(clone!(
            #[weak(rename_to = obj)]
            self,
            async move {
                obj.imp().load().await;
            }
        ));
    }
}

/// The value of the given property, if it is a non-empty string.
fn string_property(data: &JsonMap<String, JsonValue>, name: &str) -> Option<String> {
    let value = data.get(name)?.as_str()?.trim();

    (!value.is_empty()).then(|| value.to_owned())
}

/// The value of the given property, if it is a number that fits in a `UInt`.
fn uint_property(data: &JsonMap<String, JsonValue>, name: &str) -> Option<UInt> {
    let value = data.get(name)?;

    // Some homeservers send these as strings, so accept both.
    let number = value
        .as_u64()
        .or_else(|| value.as_str()?.parse::<u64>().ok())?;

    UInt::try_from(number).ok()
}

/// The image of the preview, if the given properties describe one.
fn image_property(data: &JsonMap<String, JsonValue>) -> Option<UrlPreviewImage> {
    let uri = OwnedMxcUri::from(string_property(data, "og:image")?);

    // The spec says that this is an `mxc:` URI. A homeserver that sent an
    // ordinary URL instead would have us fetch it ourselves, straight from
    // whoever the page links to, which is the one thing a preview is meant to
    // avoid.
    if uri.parts().is_err() {
        debug!("Ignoring a preview image that is not an `mxc:` URI");
        return None;
    }

    let mut info = ImageInfo::new();
    info.width = uint_property(data, "og:image:width");
    info.height = uint_property(data, "og:image:height");
    info.mimetype = string_property(data, "og:image:type");
    info.size = uint_property(data, "matrix:image:size");

    Some(UrlPreviewImage { uri, info })
}
