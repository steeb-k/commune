use std::cell::{Cell, OnceCell, RefCell};

pub(crate) use commune_core::session::UrlPreviewImage;
use commune_core::session::{UrlPreviewEntry, UrlPreviewState};
use gtk::{glib, prelude::*, subclass::prelude::*};
use tokio::task::AbortHandle;
use url::Url;

use crate::{core_bridge::ObjectWatcher, utils::LoadingState};

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
        /// The task following the core's entry for this preview.
        watch_handle: RefCell<Option<AbortHandle>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RemoteUrlPreview {
        const NAME: &'static str = "RemoteUrlPreview";
        type Type = super::RemoteUrlPreview;
    }

    #[glib::derived_properties]
    impl ObjectImpl for RemoteUrlPreview {
        fn dispose(&self) {
            if let Some(handle) = self.watch_handle.take() {
                handle.abort();
            }
        }
    }

    impl RemoteUrlPreview {
        /// The URL that this is a preview of.
        pub(super) fn url(&self) -> &Url {
            self.url.get().expect("URL should be initialized")
        }

        /// The image of the page, if it has one.
        pub(super) fn image(&self) -> Option<UrlPreviewImage> {
            self.image.borrow().clone()
        }

        /// Present the given entry of the core's cache, and follow it.
        pub(super) fn init(&self, entry: &UrlPreviewEntry) {
            // The host is the fallback for the site name, and the only thing
            // we have to show until the homeserver answers.
            self.site_name.replace(entry.site_name());
            self.url
                .set(entry.url().clone())
                .expect("URL should be uninitialized");

            let handle = ObjectWatcher::new(&*self.obj())
                .follow(entry.subscribe(), |obj: &super::RemoteUrlPreview, state| {
                    obj.imp().update_state(&state);
                })
                .spawn();
            self.watch_handle.replace(Some(handle));

            // What the core already knows, after subscribing so that
            // nothing between the two is lost.
            self.update_state(&entry.state());
        }

        /// Mirror what the core knows about this preview.
        fn update_state(&self, state: &UrlPreviewState) {
            if let Some(preview) = &state.preview {
                let obj = self.obj();

                if *self.site_name.borrow() != preview.site_name {
                    self.site_name.replace(preview.site_name.clone());
                    obj.notify_site_name();
                }

                if *self.title.borrow() != preview.title {
                    self.title.replace(preview.title.clone());
                    obj.notify_title();
                }

                if *self.description.borrow() != preview.description {
                    self.description.replace(preview.description.clone());
                    obj.notify_description();
                }

                self.image.replace(preview.image.clone());
            }

            self.set_loading_state(state.loading_state.into());
        }

        /// Set the loading state.
        fn set_loading_state(&self, loading_state: LoadingState) {
            if self.loading_state.get() == loading_state {
                return;
            }

            self.loading_state.set(loading_state);
            self.obj().notify_loading_state();
        }
    }
}

glib::wrapper! {
    /// A preview of a URL, as the homeserver describes it.
    ///
    /// The response of the endpoint is free-form `OpenGraph` data: every
    /// property is optional, and none of them can be trusted to have the type
    /// we expect. The reading is the core's; this presents it.
    pub struct RemoteUrlPreview(ObjectSubclass<imp::RemoteUrlPreview>);
}

impl RemoteUrlPreview {
    /// Construct a new `RemoteUrlPreview` presenting the given entry of the
    /// core's cache, which asks for the preview once.
    pub(super) fn new(entry: &UrlPreviewEntry) -> Self {
        let obj = glib::Object::new::<Self>();
        obj.imp().init(entry);
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
}
